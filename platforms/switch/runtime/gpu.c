/* deko3d compositor for the Switch player. Must match crates/cmvs-native/src/switch/gpu.rs.
 *
 * One graphics queue executes everything in submission order. Commands of a frame are
 * recorded into one command memory region and staging buffer, both reset after the
 * queue idles at the next frame start, so GPU work overlaps the VM between frames.
 * Released textures keep their memory and descriptor until the queue idles, so a
 * recorded draw can never read freed memory. All calls stay on the opening thread. */
#include <deko3d.h>
#include <stdio.h>
#include <string.h>
#include <switch.h>

enum {
    CMVS_GPU_OK = 0,
    CMVS_GPU_ERROR = -1,     /* see cmvs_gpu_error */
    CMVS_GPU_NO_MEMORY = -2, /* texture memory or slots exhausted; release and retry */
};

#define SCREEN_W 1280
#define SCREEN_H 720
#define SWAP_IMAGES 2
#define MAX_TEXTURES 4096
#define CANVAS_SLOT 0 /* image descriptor 0; texture n uses descriptor n + 1 */
#define IMAGE_SLOTS (MAX_TEXTURES + 1)
#define CODE_SIZE (64 * 1024)
#define CMD_SIZE (4 * 1024 * 1024)
#define CMD_RESERVE (256 * 1024)
#define STAGING_SIZE (32 * 1024 * 1024)
#define POOL_CHUNK (32 * 1024 * 1024)
#define POOL_CHUNKS 8
#define POOL_RANGES 1024
#define SCENE_FLOATS 280
#define SCENE_BASE_FLOATS 24
#define SCENE_UBO_SIZE 0x500
#define PRESENT_UBO_SIZE 0x100
/* Conservative command memory estimates, checked before recording. */
#define COST_DRAW (SCENE_FLOATS * 4 + 1024)
#define COST_SMALL 1024
#define COST_PASS 4096

#define ALIGN(v, a) (((v) + (a) - 1) & ~((a) - 1))

extern const u8 cmvs_scene_vsh[], cmvs_scene_vsh_end[];
extern const u8 cmvs_scene_fsh[], cmvs_scene_fsh_end[];
extern const u8 cmvs_present_vsh[], cmvs_present_vsh_end[];
extern const u8 cmvs_present_fsh[], cmvs_present_fsh_end[];

typedef struct {
    u32 offset, size;
} Range;

typedef struct {
    DkMemBlock block;
    u32 count;
    Range free[POOL_RANGES];
} Pool;

enum { TEX_FREE, TEX_LIVE, TEX_RELEASED };

typedef struct {
    DkImage image;
    DkMemBlock own; /* dedicated block, or NULL when pooled */
    u32 offset, size, width, height;
    u16 generation;
    u8 pool, state;
} Texture;

enum { PASS_NONE, PASS_CANVAS };

static struct {
    bool open;
    DkDevice device;
    DkQueue queue;
    DkCmdBuf cmdbuf;
    DkMemBlock code, data, cmd, staging, swap_memory, canvas_memory, readback;
    DkImage swap[SWAP_IMAGES];
    DkSwapchain swapchain;
    DkImage canvas;
    u32 canvas_size[2];
    DkShader scene_vsh, scene_fsh, present_vsh, present_fsh;
    u32 cmd_used, staging_used;
    bool reserve_given, recorded, barrier_due, released;
    bool quiet_oom; /* texture memory exhaustion is reported to the caller instead */
    int pass;
    int blend_bound; /* -1 = unknown */
    char error[192];
} g;
static Texture textures[MAX_TEXTURES];
static Pool pools[POOL_CHUNKS];

static u32 descriptors_offset(void) {
    return 0;
}
static u32 samplers_offset(void) {
    return ALIGN(IMAGE_SLOTS * sizeof(DkImageDescriptor), 0x100);
}
static u32 scene_ubo_offset(void) {
    return ALIGN(samplers_offset() + sizeof(DkSamplerDescriptor), DK_UNIFORM_BUF_ALIGNMENT);
}
static u32 present_ubo_offset(void) {
    return scene_ubo_offset() + SCENE_UBO_SIZE;
}
static u32 data_size(void) {
    return ALIGN(present_ubo_offset() + PRESENT_UBO_SIZE, DK_MEMBLOCK_ALIGNMENT);
}
static DkGpuAddr data_addr(u32 offset) {
    return dkMemBlockGetGpuAddr(g.data) + offset;
}

static void fail(const char *message) {
    if (!g.error[0])
        snprintf(g.error, sizeof(g.error), "%s", message);
}

static void on_debug(void *user, const char *context, DkResult result, const char *message) {
    (void)user;
    if (result == DkResult_OutOfMemory && g.quiet_oom)
        return;
    if (result != DkResult_Success && !g.error[0])
        snprintf(g.error, sizeof(g.error), "deko3d %s: %s (result %d)", context ? context : "?",
                 message ? message : "?", (int)result);
}

/* Normally unreachable: recording checks the estimated cost first. */
static void on_cmd_full(void *user, DkCmdBuf cmdbuf, size_t need) {
    (void)user;
    if (g.reserve_given || need > CMD_RESERVE)
        diagAbortWithResult(MAKERESULT(Module_Libnx, LibnxError_OutOfMemory));
    g.reserve_given = true;
    dkCmdBufAddMemory(cmdbuf, g.cmd, CMD_SIZE - CMD_RESERVE, CMD_RESERVE);
}

static int status(void) {
    if (!g.error[0] && g.queue && dkQueueIsInErrorState(g.queue))
        fail("GPU queue entered the error state");
    return g.error[0] ? CMVS_GPU_ERROR : CMVS_GPU_OK;
}

static DkMemBlock memblock(u32 size, u32 flags) {
    DkMemBlockMaker maker;
    dkMemBlockMakerDefaults(&maker, g.device, ALIGN(size, DK_MEMBLOCK_ALIGNMENT));
    maker.flags = flags;
    return dkMemBlockCreate(&maker);
}

/* ---- image memory: first-fit ranges over fixed chunks ---- */

static bool image_alloc_quiet(Texture *t, u32 size, u32 align);

static bool pool_take(Pool *pool, u32 size, u32 align, u32 *out) {
    for (u32 i = 0; i < pool->count; ++i) {
        Range r = pool->free[i];
        u32 start = ALIGN(r.offset, align);
        if (start < r.offset || start - r.offset > r.size || r.size - (start - r.offset) < size)
            continue;
        Range before = {r.offset, start - r.offset};
        Range after = {start + size, r.offset + r.size - start - size};
        if (before.size && after.size) {
            if (pool->count == POOL_RANGES)
                continue;
            memmove(&pool->free[i + 2], &pool->free[i + 1], (pool->count - i - 1) * sizeof(Range));
            pool->free[i] = before;
            pool->free[i + 1] = after;
            pool->count++;
        } else if (before.size || after.size) {
            pool->free[i] = before.size ? before : after;
        } else {
            memmove(&pool->free[i], &pool->free[i + 1], (pool->count - i - 1) * sizeof(Range));
            pool->count--;
        }
        *out = start;
        return true;
    }
    return false;
}

static void pool_give(Pool *pool, u32 offset, u32 size) {
    u32 i = 0;
    while (i < pool->count && pool->free[i].offset < offset)
        ++i;
    bool joins_prev = i > 0 && pool->free[i - 1].offset + pool->free[i - 1].size == offset;
    bool joins_next = i < pool->count && offset + size == pool->free[i].offset;
    if (joins_prev && joins_next) {
        pool->free[i - 1].size += size + pool->free[i].size;
        memmove(&pool->free[i], &pool->free[i + 1], (pool->count - i - 1) * sizeof(Range));
        pool->count--;
    } else if (joins_prev) {
        pool->free[i - 1].size += size;
    } else if (joins_next) {
        pool->free[i].offset = offset;
        pool->free[i].size += size;
    } else if (pool->count < POOL_RANGES) {
        memmove(&pool->free[i + 1], &pool->free[i], (pool->count - i) * sizeof(Range));
        pool->free[i] = (Range){offset, size};
        pool->count++;
    } /* else: the range stays unusable; only reachable under extreme fragmentation */
}

static bool image_alloc(Texture *t, u32 size, u32 align) {
    g.quiet_oom = true;
    bool ok = image_alloc_quiet(t, size, align);
    g.quiet_oom = false;
    return ok;
}

static bool image_alloc_quiet(Texture *t, u32 size, u32 align) {
    t->own = NULL;
    t->size = ALIGN(size, align);
    if (t->size > POOL_CHUNK / 4) {
        t->own = memblock(t->size, DkMemBlockFlags_GpuCached | DkMemBlockFlags_Image);
        t->offset = 0;
        return t->own != NULL;
    }
    for (u32 i = 0; i < POOL_CHUNKS; ++i) {
        Pool *pool = &pools[i];
        if (!pool->block) {
            pool->block = memblock(POOL_CHUNK, DkMemBlockFlags_GpuCached | DkMemBlockFlags_Image);
            if (!pool->block)
                return false;
            pool->count = 1;
            pool->free[0] = (Range){0, POOL_CHUNK};
        }
        if (pool_take(pool, t->size, align, &t->offset)) {
            t->pool = i;
            return true;
        }
    }
    return false;
}

static void image_free(Texture *t) {
    if (t->own)
        dkMemBlockDestroy(t->own);
    else
        pool_give(&pools[t->pool], t->offset, t->size);
    t->own = NULL;
}

/* ---- command recording ---- */

static void write_descriptor(u32 slot, const DkImage *image) {
    DkImageView view;
    dkImageViewDefaults(&view, image);
    DkImageDescriptor *set =
        (DkImageDescriptor *)((u8 *)dkMemBlockGetCpuAddr(g.data) + descriptors_offset());
    dkImageDescriptorInitialize(&set[slot], &view, false, false);
    g.barrier_due = true;
}

static void submit(void) {
    if (!g.recorded)
        return;
    dkQueueSubmitCommands(g.queue, dkCmdBufFinishList(g.cmdbuf));
    g.recorded = false;
}

/* Waits for all submitted work, then recycles command, staging and released memory. */
static void idle(void) {
    submit();
    dkQueueWaitIdle(g.queue);
    dkCmdBufClear(g.cmdbuf);
    dkCmdBufAddMemory(g.cmdbuf, g.cmd, 0, CMD_SIZE - CMD_RESERVE);
    g.cmd_used = g.staging_used = 0;
    g.reserve_given = false;
    g.blend_bound = -1;
    if (g.released) {
        for (u32 i = 0; i < MAX_TEXTURES; ++i) {
            if (textures[i].state == TEX_RELEASED) {
                image_free(&textures[i]);
                textures[i].state = TEX_FREE;
            }
        }
        g.released = false;
    }
}

static void bind_target(const DkImage *image, u32 w, u32 h) {
    DkImageView view;
    dkImageViewDefaults(&view, image);
    dkCmdBufBindRenderTarget(g.cmdbuf, &view, NULL);
    DkViewport viewport = {0.0f, 0.0f, (float)w, (float)h, 0.0f, 1.0f};
    DkScissor scissor = {0, 0, w, h};
    dkCmdBufSetViewports(g.cmdbuf, 0, &viewport, 1);
    dkCmdBufSetScissors(g.cmdbuf, 0, &scissor, 1);
    DkRasterizerState rasterizer;
    dkRasterizerStateDefaults(&rasterizer);
    rasterizer.cullMode = DkFace_None;
    dkCmdBufBindRasterizerState(g.cmdbuf, &rasterizer);
    DkDepthStencilState depth;
    dkDepthStencilStateDefaults(&depth);
    depth.depthTestEnable = false;
    depth.depthWriteEnable = false;
    dkCmdBufBindDepthStencilState(g.cmdbuf, &depth);
    dkCmdBufBindImageDescriptorSet(g.cmdbuf, data_addr(descriptors_offset()), IMAGE_SLOTS);
    dkCmdBufBindSamplerDescriptorSet(g.cmdbuf, data_addr(samplers_offset()), 1);
    g.blend_bound = -1;
}

static void bind_scene(u32 write_mask) {
    DkShader const *shaders[] = {&g.scene_vsh, &g.scene_fsh};
    dkCmdBufBindShaders(g.cmdbuf, DkStageFlag_GraphicsMask, shaders, 2);
    dkCmdBufBindUniformBuffer(g.cmdbuf, DkStage_Vertex, 0, data_addr(scene_ubo_offset()),
                              SCENE_UBO_SIZE);
    dkCmdBufBindUniformBuffer(g.cmdbuf, DkStage_Fragment, 0, data_addr(scene_ubo_offset()),
                              SCENE_UBO_SIZE);
    DkColorWriteState write;
    dkColorWriteStateDefaults(&write);
    dkColorWriteStateSetMask(&write, 0, write_mask);
    dkCmdBufBindColorWriteState(g.cmdbuf, &write);
    g.blend_bound = -1;
}

static void bind_blend(int mode) {
    if (mode == g.blend_bound)
        return;
    DkColorState color;
    dkColorStateDefaults(&color);
    dkColorStateSetBlendEnable(&color, 0, mode != 0);
    dkCmdBufBindColorState(g.cmdbuf, &color);
    if (mode) {
        DkBlendState blend;
        dkBlendStateDefaults(&blend);
        if (mode == 1)
            dkBlendStateSetFactors(&blend, DkBlendFactor_SrcAlpha, DkBlendFactor_InvSrcAlpha,
                                   DkBlendFactor_One, DkBlendFactor_InvSrcAlpha);
        else
            dkBlendStateSetFactors(&blend, DkBlendFactor_SrcAlpha, DkBlendFactor_One,
                                   DkBlendFactor_One, DkBlendFactor_Zero);
        dkCmdBufBindBlendState(g.cmdbuf, 0, &blend);
    }
    g.blend_bound = mode;
}

static void bind_canvas_pass(void) {
    bind_target(&g.canvas, g.canvas_size[0], g.canvas_size[1]);
    bind_scene(DkColorMask_RGB);
}

/* Makes room for `cost` bytes of commands, resuming an open canvas pass after a flush. */
static void reserve(u32 cost) {
    if (g.cmd_used + cost > CMD_SIZE - CMD_RESERVE) {
        idle();
        if (g.pass == PASS_CANVAS)
            bind_canvas_pass();
    }
    g.cmd_used += cost;
    g.recorded = true;
}

static void barrier_if_due(void) {
    if (!g.barrier_due)
        return;
    dkCmdBufBarrier(g.cmdbuf, DkBarrier_Full,
                    DkInvalidateFlags_Image | DkInvalidateFlags_Descriptors);
    g.barrier_due = false;
}

static Texture *lookup(u32 id) {
    u32 slot = id & 0xffff;
    if (slot >= MAX_TEXTURES)
        return NULL;
    Texture *t = &textures[slot];
    return t->state == TEX_LIVE && t->generation == (id >> 16) ? t : NULL;
}

static DkResHandle handle(u32 id) {
    return dkMakeTextureHandle((id & 0xffff) + 1, 0);
}

static bool load_shader(DkShader *shader, const u8 *start, const u8 *end, u32 *offset) {
    u32 size = (u32)(end - start);
    if (*offset + size + DK_SHADER_CODE_UNUSABLE_SIZE > CODE_SIZE)
        return false;
    memcpy((u8 *)dkMemBlockGetCpuAddr(g.code) + *offset, start, size);
    DkShaderMaker maker;
    dkShaderMakerDefaults(&maker, g.code, *offset);
    dkShaderInitialize(shader, &maker);
    *offset = ALIGN(*offset + size, DK_SHADER_CODE_ALIGNMENT);
    return dkShaderIsValid(shader);
}

/* ---- exported API ---- */

void cmvs_gpu_close(void);

int cmvs_gpu_open(void) {
    if (g.open)
        return CMVS_GPU_ERROR; /* the Rust owner flag prevents this */
    memset(&g, 0, sizeof(g));
    memset(textures, 0, sizeof(textures));
    memset(pools, 0, sizeof(pools));
    g.open = true;
    g.blend_bound = -1;

    DkDeviceMaker device;
    dkDeviceMakerDefaults(&device);
    device.cbDebug = on_debug;
    g.device = dkDeviceCreate(&device);
    if (!g.device) {
        fail("deko3d device creation failed");
        cmvs_gpu_close();
        return CMVS_GPU_ERROR;
    }
    DkQueueMaker queue;
    dkQueueMakerDefaults(&queue, g.device);
    queue.flags = DkQueueFlags_Graphics;
    g.queue = dkQueueCreate(&queue);

    g.code = memblock(CODE_SIZE, DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached |
                                     DkMemBlockFlags_Code);
    g.data = memblock(data_size(), DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached);
    g.cmd = memblock(CMD_SIZE, DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached);
    g.staging = memblock(STAGING_SIZE, DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached);
    if (!g.queue || !g.code || !g.data || !g.cmd || !g.staging) {
        fail("GPU memory allocation failed");
        cmvs_gpu_close();
        return CMVS_GPU_ERROR;
    }
    u32 offset = 0;
    if (!load_shader(&g.scene_vsh, cmvs_scene_vsh, cmvs_scene_vsh_end, &offset) ||
        !load_shader(&g.scene_fsh, cmvs_scene_fsh, cmvs_scene_fsh_end, &offset) ||
        !load_shader(&g.present_vsh, cmvs_present_vsh, cmvs_present_vsh_end, &offset) ||
        !load_shader(&g.present_fsh, cmvs_present_fsh, cmvs_present_fsh_end, &offset)) {
        fail("invalid embedded GPU shader");
        cmvs_gpu_close();
        return CMVS_GPU_ERROR;
    }
    DkSampler sampler;
    dkSamplerDefaults(&sampler);
    sampler.wrapMode[0] = sampler.wrapMode[1] = sampler.wrapMode[2] = DkWrapMode_ClampToEdge;
    dkSamplerDescriptorInitialize(
        (DkSamplerDescriptor *)((u8 *)dkMemBlockGetCpuAddr(g.data) + samplers_offset()), &sampler);

    DkCmdBufMaker cmdbuf;
    dkCmdBufMakerDefaults(&cmdbuf, g.device);
    cmdbuf.cbAddMem = on_cmd_full;
    g.cmdbuf = dkCmdBufCreate(&cmdbuf);
    dkCmdBufAddMemory(g.cmdbuf, g.cmd, 0, CMD_SIZE - CMD_RESERVE);

    DkImageLayoutMaker layout_maker;
    dkImageLayoutMakerDefaults(&layout_maker, g.device);
    layout_maker.flags =
        DkImageFlags_UsageRender | DkImageFlags_UsagePresent | DkImageFlags_HwCompression;
    layout_maker.format = DkImageFormat_RGBA8_Unorm;
    layout_maker.dimensions[0] = SCREEN_W;
    layout_maker.dimensions[1] = SCREEN_H;
    DkImageLayout layout;
    dkImageLayoutInitialize(&layout, &layout_maker);
    u32 align = dkImageLayoutGetAlignment(&layout);
    u32 size = ALIGN((u32)dkImageLayoutGetSize(&layout), align);
    g.swap_memory = memblock(SWAP_IMAGES * size, DkMemBlockFlags_GpuCached | DkMemBlockFlags_Image);
    if (!g.swap_memory) {
        fail("GPU swapchain allocation failed");
        cmvs_gpu_close();
        return CMVS_GPU_ERROR;
    }
    DkImage const *images[SWAP_IMAGES];
    for (u32 i = 0; i < SWAP_IMAGES; ++i) {
        dkImageInitialize(&g.swap[i], &layout, g.swap_memory, i * size);
        images[i] = &g.swap[i];
    }
    DkSwapchainMaker swapchain;
    dkSwapchainMakerDefaults(&swapchain, g.device, nwindowGetDefault(), images, SWAP_IMAGES);
    g.swapchain = dkSwapchainCreate(&swapchain);
    if (!g.swapchain)
        fail("GPU swapchain creation failed");
    if (status() != CMVS_GPU_OK) {
        cmvs_gpu_close();
        return CMVS_GPU_ERROR;
    }
    return CMVS_GPU_OK;
}

/* Copies the first error message; returns its length. Errors are sticky. */
u32 cmvs_gpu_error(char *out, u32 capacity) {
    u32 length = (u32)strnlen(g.error, sizeof(g.error));
    if (capacity) {
        u32 n = length < capacity ? length : capacity;
        memcpy(out, g.error, n);
    }
    return length;
}

int cmvs_gpu_begin_frame(void) {
    if (status() != CMVS_GPU_OK)
        return CMVS_GPU_ERROR;
    /* A frame abandoned by a host error left its pass open; its commands are harmless. */
    g.pass = PASS_NONE;
    idle();
    return status();
}

int cmvs_gpu_create_texture(u32 width, u32 height, const u8 *rgba, u32 *out) {
    if (status() != CMVS_GPU_OK)
        return CMVS_GPU_ERROR;
    if (!width || !height || width > 16384 || height > 16384) {
        fail("GPU texture exceeds device limits");
        return CMVS_GPU_ERROR;
    }
    u32 bytes = width * height * 4;
    u32 slot = 0;
    while (slot < MAX_TEXTURES && textures[slot].state != TEX_FREE)
        ++slot;
    if (slot == MAX_TEXTURES && g.released) {
        idle();
        if (g.pass == PASS_CANVAS)
            bind_canvas_pass();
        slot = 0;
        while (slot < MAX_TEXTURES && textures[slot].state != TEX_FREE)
            ++slot;
    }
    if (slot == MAX_TEXTURES)
        return CMVS_GPU_NO_MEMORY;
    Texture *t = &textures[slot];

    DkImageLayoutMaker maker;
    dkImageLayoutMakerDefaults(&maker, g.device);
    maker.format = DkImageFormat_RGBA8_Unorm;
    maker.dimensions[0] = width;
    maker.dimensions[1] = height;
    DkImageLayout layout;
    dkImageLayoutInitialize(&layout, &maker);
    u64 size = dkImageLayoutGetSize(&layout);
    u32 align = dkImageLayoutGetAlignment(&layout);
    if (size > UINT32_MAX / 2) {
        fail("GPU texture exceeds device limits");
        return CMVS_GPU_ERROR;
    }
    if (!image_alloc(t, (u32)size, align)) {
        if (!g.released)
            return CMVS_GPU_NO_MEMORY;
        idle();
        if (g.pass == PASS_CANVAS)
            bind_canvas_pass();
        if (!image_alloc(t, (u32)size, align))
            return CMVS_GPU_NO_MEMORY;
    }
    dkImageInitialize(&t->image, &layout, t->own ? t->own : pools[t->pool].block, t->offset);

    if (bytes <= STAGING_SIZE && g.staging_used + bytes > STAGING_SIZE) {
        idle();
        if (g.pass == PASS_CANVAS)
            bind_canvas_pass();
    }
    /* Reserve before staging: a command flush resets g.staging_used. */
    reserve(COST_SMALL);

    /* Stage the pixels; a texture larger than the whole staging buffer uses its own. */
    DkMemBlock temporary = NULL;
    DkGpuAddr source;
    if (bytes > STAGING_SIZE) {
        temporary = memblock(bytes, DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached);
        if (!temporary) {
            image_free(t);
            fail("GPU staging allocation failed");
            return CMVS_GPU_ERROR;
        }
        memcpy(dkMemBlockGetCpuAddr(temporary), rgba, bytes);
        source = dkMemBlockGetGpuAddr(temporary);
    } else {
        u32 at = g.staging_used;
        g.staging_used = ALIGN(at + bytes, 256);
        memcpy((u8 *)dkMemBlockGetCpuAddr(g.staging) + at, rgba, bytes);
        source = dkMemBlockGetGpuAddr(g.staging) + at;
    }
    DkImageView view;
    dkImageViewDefaults(&view, &t->image);
    DkCopyBuf copy = {source, 0, 0};
    DkImageRect rect = {0, 0, 0, width, height, 1};
    dkCmdBufCopyBufferToImage(g.cmdbuf, &copy, &view, &rect, 0);
    if (temporary) {
        idle();
        if (g.pass == PASS_CANVAS)
            bind_canvas_pass();
        dkMemBlockDestroy(temporary);
    }
    t->generation = (u16)(t->generation + 1) ? (u16)(t->generation + 1) : 1;
    t->width = width;
    t->height = height;
    t->state = TEX_LIVE;
    write_descriptor(slot + 1, &t->image);
    *out = ((u32)t->generation << 16) | slot;
    return status();
}

/* The memory is reused only after all work recorded so far has completed. */
void cmvs_gpu_release_texture(u32 id) {
    Texture *t = lookup(id);
    if (!t)
        return;
    t->state = TEX_RELEASED;
    g.released = true;
}

int cmvs_gpu_begin_canvas(u32 width, u32 height, const u8 clear[4]) {
    if (status() != CMVS_GPU_OK)
        return CMVS_GPU_ERROR;
    if (!width || !height || width > 16384 || height > 16384) {
        fail("invalid GPU canvas size");
        return CMVS_GPU_ERROR;
    }
    if (!g.canvas_memory || g.canvas_size[0] != width || g.canvas_size[1] != height) {
        idle();
        if (g.canvas_memory)
            dkMemBlockDestroy(g.canvas_memory);
        DkImageLayoutMaker maker;
        dkImageLayoutMakerDefaults(&maker, g.device);
        maker.flags = DkImageFlags_UsageRender;
        maker.format = DkImageFormat_RGBA8_Unorm;
        maker.dimensions[0] = width;
        maker.dimensions[1] = height;
        DkImageLayout layout;
        dkImageLayoutInitialize(&layout, &maker);
        g.canvas_memory = memblock((u32)dkImageLayoutGetSize(&layout),
                                   DkMemBlockFlags_GpuCached | DkMemBlockFlags_Image);
        g.canvas_size[0] = g.canvas_size[1] = 0;
        if (!g.canvas_memory) {
            fail("GPU canvas allocation failed");
            return CMVS_GPU_ERROR;
        }
        dkImageInitialize(&g.canvas, &layout, g.canvas_memory, 0);
        g.canvas_size[0] = width;
        g.canvas_size[1] = height;
        write_descriptor(CANVAS_SLOT, &g.canvas);
    }
    reserve(COST_PASS);
    /* Uploads and the previous present (which samples the canvas) complete first. */
    dkCmdBufBarrier(g.cmdbuf, DkBarrier_Full,
                    DkInvalidateFlags_Image | DkInvalidateFlags_Descriptors);
    g.barrier_due = false;
    g.pass = PASS_CANVAS;
    bind_canvas_pass();
    dkCmdBufClearColorFloat(g.cmdbuf, 0, DkColorMask_RGBA, clear[0] / 255.0f, clear[1] / 255.0f,
                            clear[2] / 255.0f, clear[3] / 255.0f);
    return status();
}

/* `count` is 24 or 280 floats in the cmvs_core::scene_render::shader layout. */
int cmvs_gpu_draw(u32 blend, const float *params, u32 count, const u32 ids[3]) {
    if (status() != CMVS_GPU_OK)
        return CMVS_GPU_ERROR;
    if (g.pass != PASS_CANVAS || blend > 2 ||
        (count != SCENE_BASE_FLOATS && count != SCENE_FLOATS)) {
        fail("invalid GPU draw");
        return CMVS_GPU_ERROR;
    }
    DkResHandle handles[3];
    for (u32 i = 0; i < 3; ++i) {
        if (!lookup(ids[i])) {
            fail("GPU draw uses a released texture");
            return CMVS_GPU_ERROR;
        }
        handles[i] = handle(ids[i]);
    }
    reserve(COST_DRAW);
    barrier_if_due();
    bind_blend((int)blend);
    dkCmdBufPushConstants(g.cmdbuf, data_addr(scene_ubo_offset()), SCENE_UBO_SIZE, 0, count * 4,
                          params);
    dkCmdBufBindTextures(g.cmdbuf, DkStage_Fragment, 0, handles, 3);
    dkCmdBufDraw(g.cmdbuf, DkPrimitive_TriangleStrip, 4, 1, 0, 0);
    return status();
}

static void draw_present(const float view[8], DkResHandle source) {
    DkShader const *shaders[] = {&g.present_vsh, &g.present_fsh};
    dkCmdBufBindShaders(g.cmdbuf, DkStageFlag_GraphicsMask, shaders, 2);
    dkCmdBufBindUniformBuffer(g.cmdbuf, DkStage_Fragment, 0, data_addr(present_ubo_offset()),
                              PRESENT_UBO_SIZE);
    DkColorWriteState write;
    dkColorWriteStateDefaults(&write);
    dkCmdBufBindColorWriteState(g.cmdbuf, &write);
    bind_blend(0);
    dkCmdBufPushConstants(g.cmdbuf, data_addr(present_ubo_offset()), PRESENT_UBO_SIZE, 0, 8 * 4,
                          view);
    dkCmdBufBindTexture(g.cmdbuf, DkStage_Fragment, 0, source);
    dkCmdBufDraw(g.cmdbuf, DkPrimitive_Triangles, 3, 1, 0, 0);
}

/* Scales a texture onto the open canvas: view = left, top, width, height, source
   width, source height, linear, 0. Used for decoded movie frames. */
int cmvs_gpu_scale(u32 id, const float view[8]) {
    if (status() != CMVS_GPU_OK)
        return CMVS_GPU_ERROR;
    if (g.pass != PASS_CANVAS || !lookup(id)) {
        fail("invalid GPU scale");
        return CMVS_GPU_ERROR;
    }
    reserve(COST_DRAW);
    barrier_if_due();
    draw_present(view, handle(id));
    bind_scene(DkColorMask_RGB);
    return status();
}

/* Ends the canvas pass and shows it on screen, then draws `count` replace-blended
   24-float scene overlays (the host cursor) in screen coordinates. */
int cmvs_gpu_present(const float view[8], const float *overlays, u32 count) {
    if (status() != CMVS_GPU_OK)
        return CMVS_GPU_ERROR;
    if (g.pass != PASS_CANVAS || count > 16) {
        fail("invalid GPU present");
        return CMVS_GPU_ERROR;
    }
    reserve(COST_SMALL);
    dkCmdBufBarrier(g.cmdbuf, DkBarrier_Fragments, DkInvalidateFlags_Image);
    g.pass = PASS_NONE;
    submit();
    /* Canvas work runs while the queue waits for a free swapchain image. */
    int slot = dkQueueAcquireImage(g.queue, g.swapchain);
    reserve(COST_PASS + count * COST_DRAW);
    bind_target(&g.swap[slot], SCREEN_W, SCREEN_H);
    draw_present(view, dkMakeTextureHandle(CANVAS_SLOT, 0));
    if (count) {
        bind_scene(DkColorMask_RGB);
        bind_blend(0);
        DkResHandle canvas = dkMakeTextureHandle(CANVAS_SLOT, 0);
        DkResHandle handles[3] = {canvas, canvas, canvas};
        for (u32 i = 0; i < count; ++i) {
            dkCmdBufPushConstants(g.cmdbuf, data_addr(scene_ubo_offset()), SCENE_UBO_SIZE, 0,
                                  SCENE_BASE_FLOATS * 4, overlays + i * SCENE_BASE_FLOATS);
            dkCmdBufBindTextures(g.cmdbuf, DkStage_Fragment, 0, handles, 3);
            dkCmdBufDraw(g.cmdbuf, DkPrimitive_TriangleStrip, 4, 1, 0, 0);
        }
    }
    submit();
    dkQueuePresentImage(g.queue, g.swapchain, slot);
    return status();
}

/* Reads the canvas as last presented (tightly packed RGBA rows). */
int cmvs_gpu_read(u8 *out, u32 width, u32 height) {
    if (status() != CMVS_GPU_OK)
        return CMVS_GPU_ERROR;
    if (g.pass != PASS_NONE || width != g.canvas_size[0] || height != g.canvas_size[1] || !width) {
        fail("invalid GPU readback");
        return CMVS_GPU_ERROR;
    }
    u32 bytes = width * height * 4;
    if (g.readback && dkMemBlockGetSize(g.readback) < bytes) {
        dkMemBlockDestroy(g.readback);
        g.readback = NULL;
    }
    if (!g.readback)
        g.readback = memblock(bytes, DkMemBlockFlags_CpuCached | DkMemBlockFlags_GpuCached);
    if (!g.readback) {
        fail("GPU readback allocation failed");
        return CMVS_GPU_ERROR;
    }
    reserve(COST_SMALL);
    DkImageView view;
    dkImageViewDefaults(&view, &g.canvas);
    DkImageRect rect = {0, 0, 0, width, height, 1};
    DkCopyBuf copy = {dkMemBlockGetGpuAddr(g.readback), 0, 0};
    dkCmdBufCopyImageToBuffer(g.cmdbuf, &view, &rect, &copy, 0);
    submit();
    dkQueueWaitIdle(g.queue);
    dkMemBlockFlushCpuCache(g.readback, 0, bytes);
    memcpy(out, dkMemBlockGetCpuAddr(g.readback), bytes);
    return status();
}

void cmvs_gpu_close(void) {
    if (!g.open)
        return;
    if (g.queue)
        dkQueueWaitIdle(g.queue);
    for (u32 i = 0; i < MAX_TEXTURES; ++i)
        if (textures[i].own)
            dkMemBlockDestroy(textures[i].own);
    for (u32 i = 0; i < POOL_CHUNKS; ++i)
        if (pools[i].block)
            dkMemBlockDestroy(pools[i].block);
    if (g.swapchain)
        dkSwapchainDestroy(g.swapchain);
    if (g.queue)
        dkQueueDestroy(g.queue);
    if (g.cmdbuf)
        dkCmdBufDestroy(g.cmdbuf);
    DkMemBlock blocks[] = {g.readback, g.canvas_memory, g.swap_memory, g.staging,
                           g.cmd,      g.data,          g.code};
    for (u32 i = 0; i < sizeof(blocks) / sizeof(blocks[0]); ++i)
        if (blocks[i])
            dkMemBlockDestroy(blocks[i]);
    if (g.device)
        dkDeviceDestroy(g.device);
    char error[sizeof(g.error)];
    memcpy(error, g.error, sizeof(error)); /* a failed open still reports why */
    memset(&g, 0, sizeof(g));
    memcpy(g.error, error, sizeof(error));
}
