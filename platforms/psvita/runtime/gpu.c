/* GXM compositor for the Vita player. Must match crates/cmvs-native/src/vita/gpu.rs.
 *
 * Runs on vita2d's GXM context between its display scenes. A frame waits for the GPU
 * (begin_frame), uploads textures, renders the logical canvas in a scene of its own and
 * hands the canvas to window.c, which scales it onto the display as it does software
 * frames. GPU work then overlaps the VM until the next frame starts. Texture memory
 * comes from video memory (CDRAM) chunks, then main memory; released textures keep
 * their memory until the GPU idles, so a submitted draw never reads freed memory. The
 * scene programs are offline-compiled GXP binaries (shaders.S). All calls stay on the
 * window's thread. */
#include "window.h"
#include <psp2/gxm.h>
#include <psp2/kernel/sysmem.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>

enum {
    CMVS_GPU_OK = 0,
    CMVS_GPU_ERROR = -1,     /* see cmvs_vita_gpu_error */
    CMVS_GPU_NO_MEMORY = -2, /* texture memory or slots exhausted; release and retry */
};

#define MAX_TEXTURES 4096
#define MAX_SIZE 4096 /* GXM texture and render target limit */
#define CDRAM_ALIGN (256 * 1024)
#define PAGE_ALIGN (4 * 1024)
#define CHUNK (8 * 1024 * 1024)
#define CHUNKS 12
#define RANGES 1024
#define SCENE_FLOATS 280
#define SCENE_BASE_FLOATS 24
#define VERTEX_FLOATS 16 /* target size, source rect, corners */
#define FLAG_SOLID 1.0f
#define BLEND_CLEAR 3 /* replace including alpha, for the canvas clear */

#define ALIGN(v, a) (((v) + (a) - 1) & ~((a) - 1))

extern const uint8_t cmvs_scene_v[], cmvs_scene_f[];

typedef struct {
    uint32_t offset, size;
} Range;

typedef struct {
    SceUID uid; /* 0 until the chunk is allocated */
    uint8_t *base;
    uint32_t count;
    Range free[RANGES];
} Pool;

enum { TEX_FREE, TEX_LIVE, TEX_RELEASED };

typedef struct {
    SceGxmTexture gxm;
    SceUID own; /* dedicated block, or 0 when pooled */
    uint32_t offset, size, width, height;
    uint16_t generation;
    uint8_t pool, state;
} Texture;

static struct {
    bool open, in_pass, released;
    SceGxmContext *context;
    SceGxmShaderPatcher *patcher;
    SceGxmShaderPatcherId vertex_id, fragment_id;
    bool vertex_registered, fragment_registered;
    SceGxmVertexProgram *vertex;
    SceGxmFragmentProgram *fragment[4];
    const SceGxmProgramParameter *vertex_v, *fragment_v, *fragment_dims;
    SceUID geometry_uid;
    const float *corners;
    const uint16_t *indices;
    SceUID canvas_uid;
    uint8_t *canvas_data;
    vita2d_texture canvas; /* only the texture, render target and color surface */
    uint32_t canvas_size[2];
    uint32_t white;
    int bound_blend; /* -1 = unknown */
    char error[192];
} g;
static Texture textures[MAX_TEXTURES];
static Pool pools[CHUNKS];

static void fail(const char *message) {
    if (!g.error[0])
        snprintf(g.error, sizeof(g.error), "%s", message);
}

static void fail_code(const char *call, int code) {
    if (!g.error[0])
        snprintf(g.error, sizeof(g.error), "%s failed: 0x%08x", call, (unsigned)code);
}

static int status(void) {
    if (!g.open && !g.error[0])
        fail("GPU not open");
    return g.error[0] ? CMVS_GPU_ERROR : CMVS_GPU_OK;
}

/* ---- memory ---- */

static void *gpu_alloc(SceKernelMemBlockType type, uint32_t size, SceUID *uid) {
    uint32_t aligned = ALIGN(size, type == SCE_KERNEL_MEMBLOCK_TYPE_USER_CDRAM_RW ? CDRAM_ALIGN
                                                                                    : PAGE_ALIGN);
    *uid = sceKernelAllocMemBlock("cmvs_gpu", type, aligned, NULL);
    if (*uid < 0) {
        *uid = 0;
        return NULL;
    }
    void *base = NULL;
    if (sceKernelGetMemBlockBase(*uid, &base) < 0 ||
        sceGxmMapMemory(base, aligned, SCE_GXM_MEMORY_ATTRIB_READ | SCE_GXM_MEMORY_ATTRIB_WRITE) <
            0) {
        sceKernelFreeMemBlock(*uid);
        *uid = 0;
        return NULL;
    }
    return base;
}

/* Video memory first, uncached main memory once it is exhausted. */
static void *texture_alloc(uint32_t size, SceUID *uid) {
    void *base = gpu_alloc(SCE_KERNEL_MEMBLOCK_TYPE_USER_CDRAM_RW, size, uid);
    return base ? base : gpu_alloc(SCE_KERNEL_MEMBLOCK_TYPE_USER_RW_UNCACHE, size, uid);
}

/* Only after the GPU no longer uses the memory. */
static void gpu_free(SceUID *uid) {
    if (*uid <= 0)
        return;
    void *base = NULL;
    if (sceKernelGetMemBlockBase(*uid, &base) >= 0)
        sceGxmUnmapMemory(base);
    sceKernelFreeMemBlock(*uid);
    *uid = 0;
}

/* First-fit ranges over fixed chunks, as in the Switch compositor. */
static bool pool_take(Pool *pool, uint32_t size, uint32_t align, uint32_t *out) {
    for (uint32_t i = 0; i < pool->count; ++i) {
        Range r = pool->free[i];
        uint32_t start = ALIGN(r.offset, align);
        if (start < r.offset || start - r.offset > r.size || r.size - (start - r.offset) < size)
            continue;
        Range before = {r.offset, start - r.offset};
        Range after = {start + size, r.offset + r.size - start - size};
        if (before.size && after.size) {
            if (pool->count == RANGES)
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

static void pool_give(Pool *pool, uint32_t offset, uint32_t size) {
    uint32_t i = 0;
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
    } else if (pool->count < RANGES) {
        memmove(&pool->free[i + 1], &pool->free[i], (pool->count - i) * sizeof(Range));
        pool->free[i] = (Range){offset, size};
        pool->count++;
    } /* else: the range stays unusable; only reachable under extreme fragmentation */
}

static uint8_t *texture_memory(const Texture *t) {
    if (t->own) {
        void *base = NULL;
        sceKernelGetMemBlockBase(t->own, &base);
        return base;
    }
    return pools[t->pool].base + t->offset;
}

static bool image_alloc(Texture *t, uint32_t size) {
    t->own = 0;
    t->size = ALIGN(size, SCE_GXM_TEXTURE_ALIGNMENT);
    if (t->size > CHUNK / 2) {
        t->offset = 0;
        return texture_alloc(t->size, &t->own) != NULL;
    }
    for (uint32_t i = 0; i < CHUNKS; ++i) {
        Pool *pool = &pools[i];
        if (!pool->uid) {
            pool->base = texture_alloc(CHUNK, &pool->uid);
            if (!pool->base)
                return false;
            pool->count = 1;
            pool->free[0] = (Range){0, CHUNK};
        }
        if (pool_take(pool, t->size, SCE_GXM_TEXTURE_ALIGNMENT, &t->offset)) {
            t->pool = (uint8_t)i;
            return true;
        }
    }
    return false;
}

static void image_free(Texture *t) {
    if (t->own)
        gpu_free(&t->own);
    else
        pool_give(&pools[t->pool], t->offset, t->size);
}

/* Waits for all submitted work, then frees released textures. */
static void idle(void) {
    sceGxmFinish(g.context);
    if (!g.released)
        return;
    for (uint32_t i = 0; i < MAX_TEXTURES; ++i) {
        if (textures[i].state == TEX_RELEASED) {
            image_free(&textures[i]);
            textures[i].state = TEX_FREE;
        }
    }
    g.released = false;
}

static Texture *lookup(uint32_t id) {
    uint32_t slot = id & 0xffff;
    if (slot >= MAX_TEXTURES)
        return NULL;
    Texture *t = &textures[slot];
    return t->state == TEX_LIVE && t->generation == (id >> 16) ? t : NULL;
}

static uint32_t free_slot(void) {
    uint32_t slot = 0;
    while (slot < MAX_TEXTURES && textures[slot].state != TEX_FREE)
        ++slot;
    return slot;
}

/* ---- canvas ---- */

static void free_canvas(void) {
    if (!g.canvas_uid)
        return;
    cmvs_vita_window_forget(&g.canvas);
    if (g.canvas.gxm_rtgt)
        sceGxmDestroyRenderTarget(g.canvas.gxm_rtgt);
    gpu_free(&g.canvas_uid);
    memset(&g.canvas, 0, sizeof(g.canvas));
    g.canvas_size[0] = g.canvas_size[1] = 0;
}

/* Texture and color surface share rows of ALIGN(width, 8) pixels, the stride GXM gives
   linear textures. */
static bool make_canvas(uint32_t width, uint32_t height) {
    uint32_t stride = ALIGN(width, 8);
    g.canvas_data = texture_alloc(stride * height * 4, &g.canvas_uid);
    if (!g.canvas_data) {
        fail("GPU canvas allocation failed");
        return false;
    }
    int code = sceGxmTextureInitLinear(&g.canvas.gxm_tex, g.canvas_data,
                                       SCE_GXM_TEXTURE_FORMAT_A8B8G8R8, width, height, 0);
    if (code < 0) {
        fail_code("sceGxmTextureInitLinear", code);
        return false;
    }
    code = sceGxmColorSurfaceInit(&g.canvas.gxm_sfc, SCE_GXM_COLOR_FORMAT_A8B8G8R8,
                                  SCE_GXM_COLOR_SURFACE_LINEAR, SCE_GXM_COLOR_SURFACE_SCALE_NONE,
                                  SCE_GXM_OUTPUT_REGISTER_SIZE_32BIT, width, height, stride,
                                  g.canvas_data);
    if (code < 0) {
        fail_code("sceGxmColorSurfaceInit", code);
        return false;
    }
    SceGxmRenderTargetParams params;
    memset(&params, 0, sizeof(params));
    params.width = (uint16_t)width;
    params.height = (uint16_t)height;
    params.scenesPerFrame = 1;
    params.multisampleMode = SCE_GXM_MULTISAMPLE_NONE;
    params.driverMemBlock = -1;
    code = sceGxmCreateRenderTarget(&params, &g.canvas.gxm_rtgt);
    if (code < 0) {
        g.canvas.gxm_rtgt = NULL;
        fail_code("sceGxmCreateRenderTarget", code);
        return false;
    }
    g.canvas_size[0] = width;
    g.canvas_size[1] = height;
    return true;
}

/* ---- drawing ---- */

/* Binds programs, uniforms and textures, then draws the quad. `params` holds `count`
   floats in the cmvs_core::scene_render::shader layout. */
static void draw_quad(int blend, const float *params, uint32_t count, Texture *const t[3]) {
    sceGxmSetVertexProgram(g.context, g.vertex);
    if (blend != g.bound_blend) {
        sceGxmSetFragmentProgram(g.context, g.fragment[blend]);
        g.bound_blend = blend;
    }
    void *uniforms = NULL;
    sceGxmReserveVertexDefaultUniformBuffer(g.context, &uniforms);
    sceGxmSetUniformDataF(uniforms, g.vertex_v, 0, VERTEX_FLOATS, params);
    sceGxmReserveFragmentDefaultUniformBuffer(g.context, &uniforms);
    sceGxmSetUniformDataF(uniforms, g.fragment_v, 0, count, params);
    float dims[8] = {(float)t[0]->width, (float)t[0]->height, (float)t[1]->width,
                     (float)t[1]->height, (float)t[2]->width, (float)t[2]->height, 0.0f, 0.0f};
    sceGxmSetUniformDataF(uniforms, g.fragment_dims, 0, 8, dims);
    for (unsigned i = 0; i < 3; ++i)
        sceGxmSetFragmentTexture(g.context, i, &t[i]->gxm);
    sceGxmSetVertexStream(g.context, 0, g.corners);
    int code = sceGxmDraw(g.context, SCE_GXM_PRIMITIVE_TRIANGLE_STRIP, SCE_GXM_INDEX_FORMAT_U16,
                          g.indices, 4);
    if (code < 0)
        fail_code("sceGxmDraw", code);
}

static bool create_programs(void) {
    const SceGxmProgram *vertex = (const SceGxmProgram *)cmvs_scene_v;
    const SceGxmProgram *fragment = (const SceGxmProgram *)cmvs_scene_f;
    if (sceGxmProgramCheck(vertex) < 0 || sceGxmProgramCheck(fragment) < 0) {
        fail("invalid embedded GPU shader");
        return false;
    }
    int code = sceGxmShaderPatcherRegisterProgram(g.patcher, vertex, &g.vertex_id);
    g.vertex_registered = code >= 0;
    if (code >= 0) {
        code = sceGxmShaderPatcherRegisterProgram(g.patcher, fragment, &g.fragment_id);
        g.fragment_registered = code >= 0;
    }
    if (code < 0) {
        fail_code("sceGxmShaderPatcherRegisterProgram", code);
        return false;
    }
    const SceGxmProgramParameter *corner = sceGxmProgramFindParameterByName(vertex, "corner");
    g.vertex_v = sceGxmProgramFindParameterByName(vertex, "v");
    g.fragment_v = sceGxmProgramFindParameterByName(fragment, "v");
    g.fragment_dims = sceGxmProgramFindParameterByName(fragment, "dims");
    if (!corner || !g.vertex_v || !g.fragment_v || !g.fragment_dims) {
        fail("GPU shader parameters missing");
        return false;
    }
    SceGxmVertexAttribute attribute = {
        .streamIndex = 0,
        .offset = 0,
        .format = SCE_GXM_ATTRIBUTE_FORMAT_F32,
        .componentCount = 2,
        .regIndex = (uint16_t)sceGxmProgramParameterGetResourceIndex(corner),
    };
    SceGxmVertexStream stream = {.stride = 8, .indexSource = SCE_GXM_INDEX_SOURCE_INDEX_16BIT};
    code = sceGxmShaderPatcherCreateVertexProgram(g.patcher, g.vertex_id, &attribute, 1, &stream,
                                                  1, &g.vertex);
    if (code < 0) {
        g.vertex = NULL;
        fail_code("sceGxmShaderPatcherCreateVertexProgram", code);
        return false;
    }
    /* Replace, alpha and additive leave the canvas alpha alone, as the wgpu pipelines do;
       the clear writes it. */
    static const struct {
        uint8_t mask, func, src, dst;
    } modes[4] = {
        {SCE_GXM_COLOR_MASK_R | SCE_GXM_COLOR_MASK_G | SCE_GXM_COLOR_MASK_B,
         SCE_GXM_BLEND_FUNC_NONE, SCE_GXM_BLEND_FACTOR_ONE, SCE_GXM_BLEND_FACTOR_ZERO},
        {SCE_GXM_COLOR_MASK_R | SCE_GXM_COLOR_MASK_G | SCE_GXM_COLOR_MASK_B,
         SCE_GXM_BLEND_FUNC_ADD, SCE_GXM_BLEND_FACTOR_SRC_ALPHA,
         SCE_GXM_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA},
        {SCE_GXM_COLOR_MASK_R | SCE_GXM_COLOR_MASK_G | SCE_GXM_COLOR_MASK_B,
         SCE_GXM_BLEND_FUNC_ADD, SCE_GXM_BLEND_FACTOR_SRC_ALPHA, SCE_GXM_BLEND_FACTOR_ONE},
        {SCE_GXM_COLOR_MASK_ALL, SCE_GXM_BLEND_FUNC_NONE, SCE_GXM_BLEND_FACTOR_ONE,
         SCE_GXM_BLEND_FACTOR_ZERO},
    };
    for (unsigned i = 0; i < 4; ++i) {
        SceGxmBlendInfo blend;
        memset(&blend, 0, sizeof(blend));
        blend.colorMask = modes[i].mask;
        blend.colorFunc = modes[i].func;
        blend.alphaFunc = modes[i].func;
        blend.colorSrc = modes[i].src;
        blend.colorDst = modes[i].dst;
        blend.alphaSrc = SCE_GXM_BLEND_FACTOR_ONE;
        blend.alphaDst = SCE_GXM_BLEND_FACTOR_ZERO;
        code = sceGxmShaderPatcherCreateFragmentProgram(
            g.patcher, g.fragment_id, SCE_GXM_OUTPUT_REGISTER_FORMAT_UCHAR4,
            SCE_GXM_MULTISAMPLE_NONE, &blend, vertex, &g.fragment[i]);
        if (code < 0) {
            g.fragment[i] = NULL;
            fail_code("sceGxmShaderPatcherCreateFragmentProgram", code);
            return false;
        }
    }
    return true;
}

/* ---- exported API ---- */

int cmvs_vita_gpu_create_texture(uint32_t width, uint32_t height, const uint8_t *rgba,
                                 uint32_t *out);

int cmvs_vita_gpu_open(void) {
    if (g.open || !cmvs_vita_window_is_open())
        return CMVS_GPU_ERROR; /* the Rust owner flag and window ownership prevent this */
    memset(&g, 0, sizeof(g));
    memset(textures, 0, sizeof(textures));
    memset(pools, 0, sizeof(pools));
    g.open = true;
    g.bound_blend = -1;
    g.context = vita2d_get_context();
    g.patcher = vita2d_get_shader_patcher();
    if (!g.context || !g.patcher) {
        fail("vita2d has no GXM context");
        cmvs_vita_gpu_close();
        return CMVS_GPU_ERROR;
    }
    uint8_t *geometry = gpu_alloc(SCE_KERNEL_MEMBLOCK_TYPE_USER_RW_UNCACHE, PAGE_ALIGN,
                                  &g.geometry_uid);
    if (!geometry) {
        fail("GPU memory allocation failed");
        cmvs_vita_gpu_close();
        return CMVS_GPU_ERROR;
    }
    /* Strip corners as (0,0), (1,0), (0,1), (1,1); the vertex shader picks positions. */
    static const float corners[8] = {0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 1.0f, 1.0f, 1.0f};
    static const uint16_t indices[4] = {0, 1, 2, 3};
    memcpy(geometry, corners, sizeof(corners));
    memcpy(geometry + sizeof(corners), indices, sizeof(indices));
    g.corners = (const float *)geometry;
    g.indices = (const uint16_t *)(geometry + sizeof(corners));
    static const uint8_t white[4] = {255, 255, 255, 255};
    if (!create_programs() ||
        cmvs_vita_gpu_create_texture(1, 1, white, &g.white) != CMVS_GPU_OK) {
        fail("GPU initialization failed");
        cmvs_vita_gpu_close();
        return CMVS_GPU_ERROR;
    }
    return CMVS_GPU_OK;
}

/* Copies the first error message; returns its length. Errors are sticky. */
uint32_t cmvs_vita_gpu_error(char *out, uint32_t capacity) {
    uint32_t length = (uint32_t)strnlen(g.error, sizeof(g.error));
    if (capacity) {
        uint32_t n = length < capacity ? length : capacity;
        memcpy(out, g.error, n);
    }
    return length;
}

void cmvs_vita_gpu_end_pass(void) {
    if (!g.in_pass)
        return;
    sceGxmEndScene(g.context, NULL, NULL);
    g.in_pass = false;
}

int cmvs_vita_gpu_begin_frame(void) {
    if (status() != CMVS_GPU_OK)
        return CMVS_GPU_ERROR;
    /* A frame abandoned by a host error left its pass open; its commands are harmless. */
    cmvs_vita_gpu_end_pass();
    idle();
    return status();
}

int cmvs_vita_gpu_create_texture(uint32_t width, uint32_t height, const uint8_t *rgba,
                                 uint32_t *out) {
    if (status() != CMVS_GPU_OK)
        return CMVS_GPU_ERROR;
    if (!width || !height || width > MAX_SIZE || height > MAX_SIZE) {
        fail("GPU texture exceeds device limits");
        return CMVS_GPU_ERROR;
    }
    if (g.in_pass) {
        fail("GPU texture upload inside the canvas pass");
        return CMVS_GPU_ERROR;
    }
    uint32_t slot = free_slot();
    if (slot == MAX_TEXTURES && g.released) {
        idle();
        slot = free_slot();
    }
    if (slot == MAX_TEXTURES)
        return CMVS_GPU_NO_MEMORY;
    Texture *t = &textures[slot];
    uint32_t stride = ALIGN(width, 8) * 4;
    if (!image_alloc(t, stride * height)) {
        if (!g.released)
            return CMVS_GPU_NO_MEMORY;
        idle();
        if (!image_alloc(t, stride * height))
            return CMVS_GPU_NO_MEMORY;
    }
    uint8_t *data = texture_memory(t);
    for (uint32_t y = 0; y < height; ++y)
        memcpy(data + (size_t)y * stride, rgba + (size_t)y * width * 4, (size_t)width * 4);
    int code = sceGxmTextureInitLinear(&t->gxm, data, SCE_GXM_TEXTURE_FORMAT_U8U8U8U8_ABGR, width,
                                       height, 0);
    if (code < 0) {
        image_free(t);
        fail_code("sceGxmTextureInitLinear", code);
        return CMVS_GPU_ERROR;
    }
    /* The shader addresses texels itself: exact texel centers, clamped indices. */
    sceGxmTextureSetMinFilter(&t->gxm, SCE_GXM_TEXTURE_FILTER_POINT);
    sceGxmTextureSetMagFilter(&t->gxm, SCE_GXM_TEXTURE_FILTER_POINT);
    sceGxmTextureSetUAddrMode(&t->gxm, SCE_GXM_TEXTURE_ADDR_CLAMP);
    sceGxmTextureSetVAddrMode(&t->gxm, SCE_GXM_TEXTURE_ADDR_CLAMP);
    t->generation = (uint16_t)(t->generation + 1) ? (uint16_t)(t->generation + 1) : 1;
    t->width = width;
    t->height = height;
    t->state = TEX_LIVE;
    *out = ((uint32_t)t->generation << 16) | slot;
    return status();
}

/* The memory is reused only after all work submitted so far has completed. */
void cmvs_vita_gpu_release_texture(uint32_t id) {
    Texture *t = lookup(id);
    if (!t || id == g.white)
        return;
    t->state = TEX_RELEASED;
    g.released = true;
}

int cmvs_vita_gpu_begin_canvas(uint32_t width, uint32_t height, const uint8_t clear[4]) {
    if (status() != CMVS_GPU_OK)
        return CMVS_GPU_ERROR;
    if (!width || !height || width > MAX_SIZE || height > MAX_SIZE || g.in_pass) {
        fail("invalid GPU canvas");
        return CMVS_GPU_ERROR;
    }
    if (g.canvas_size[0] != width || g.canvas_size[1] != height) {
        sceGxmFinish(g.context);
        free_canvas();
        if (!make_canvas(width, height)) {
            free_canvas();
            return CMVS_GPU_ERROR;
        }
    }
    int code = sceGxmBeginScene(g.context, 0, g.canvas.gxm_rtgt, NULL, NULL, NULL,
                                &g.canvas.gxm_sfc, NULL);
    if (code < 0) {
        fail_code("sceGxmBeginScene", code);
        return CMVS_GPU_ERROR;
    }
    g.in_pass = true;
    float w = (float)width, h = (float)height;
    /* Device pixels: NDC +1 is the top row, as in vita2d's display scenes. */
    sceGxmSetViewport(g.context, w / 2.0f, w / 2.0f, h / 2.0f, -h / 2.0f, 0.5f, 0.5f);
    sceGxmSetCullMode(g.context, SCE_GXM_CULL_NONE);
    sceGxmSetFrontDepthFunc(g.context, SCE_GXM_DEPTH_FUNC_ALWAYS);
    sceGxmSetFrontDepthWriteEnable(g.context, SCE_GXM_DEPTH_WRITE_DISABLED);
    g.bound_blend = -1;
    /* A solid quad through the scene program writes the exact clear bytes. */
    float params[SCENE_BASE_FLOATS] = {
        w, h, (float)clear[3], 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, w, 0.0f,
        0.0f, h, w, h, (float)clear[0], (float)clear[1], (float)clear[2], FLAG_SOLID,
    };
    Texture *white = lookup(g.white);
    Texture *bound[3] = {white, white, white};
    draw_quad(BLEND_CLEAR, params, SCENE_BASE_FLOATS, bound);
    return status();
}

/* `count` is 24 or 280 floats in the cmvs_core::scene_render::shader layout. */
int cmvs_vita_gpu_draw(uint32_t blend, const float *params, uint32_t count, const uint32_t ids[3]) {
    if (status() != CMVS_GPU_OK)
        return CMVS_GPU_ERROR;
    if (!g.in_pass || blend > 2 || (count != SCENE_BASE_FLOATS && count != SCENE_FLOATS)) {
        fail("invalid GPU draw");
        return CMVS_GPU_ERROR;
    }
    Texture *bound[3];
    for (unsigned i = 0; i < 3; ++i) {
        bound[i] = lookup(ids[i]);
        if (!bound[i]) {
            fail("GPU draw uses a released texture");
            return CMVS_GPU_ERROR;
        }
    }
    draw_quad((int)blend, params, count, bound);
    return status();
}

/* Ends the canvas pass and shows the canvas through the window: view = left, top,
   width, height in screen pixels; `cursor` is an optional screen x, y crosshair. */
int cmvs_vita_gpu_present(const float view[4], int linear, const float *cursor) {
    if (status() != CMVS_GPU_OK)
        return CMVS_GPU_ERROR;
    if (!g.in_pass) {
        fail("invalid GPU present");
        return CMVS_GPU_ERROR;
    }
    cmvs_vita_gpu_end_pass();
    if (cmvs_vita_window_show(&g.canvas, g.canvas_size[0], g.canvas_size[1], view, linear,
                              cursor) != 0)
        fail("GPU canvas present failed");
    return status();
}

/* Reads the canvas as last presented (tightly packed RGBA rows). */
int cmvs_vita_gpu_read(uint8_t *out, uint32_t width, uint32_t height) {
    if (status() != CMVS_GPU_OK)
        return CMVS_GPU_ERROR;
    if (g.in_pass || !width || width != g.canvas_size[0] || height != g.canvas_size[1]) {
        fail("invalid GPU readback");
        return CMVS_GPU_ERROR;
    }
    sceGxmFinish(g.context);
    size_t stride = (size_t)ALIGN(width, 8) * 4;
    for (uint32_t y = 0; y < height; ++y)
        memcpy(out + (size_t)y * width * 4, g.canvas_data + y * stride, (size_t)width * 4);
    return status();
}

void cmvs_vita_gpu_close(void) {
    if (!g.open)
        return;
    cmvs_vita_gpu_end_pass();
    if (g.context)
        sceGxmFinish(g.context);
    free_canvas();
    for (uint32_t i = 0; i < MAX_TEXTURES; ++i)
        if (textures[i].own)
            gpu_free(&textures[i].own);
    for (uint32_t i = 0; i < CHUNKS; ++i)
        gpu_free(&pools[i].uid);
    for (unsigned i = 0; i < 4; ++i)
        if (g.fragment[i])
            sceGxmShaderPatcherReleaseFragmentProgram(g.patcher, g.fragment[i]);
    if (g.vertex)
        sceGxmShaderPatcherReleaseVertexProgram(g.patcher, g.vertex);
    if (g.fragment_registered)
        sceGxmShaderPatcherUnregisterProgram(g.patcher, g.fragment_id);
    if (g.vertex_registered)
        sceGxmShaderPatcherUnregisterProgram(g.patcher, g.vertex_id);
    gpu_free(&g.geometry_uid);
    char error[sizeof(g.error)];
    memcpy(error, g.error, sizeof(error)); /* a failed open still reports why */
    memset(&g, 0, sizeof(g));
    memcpy(g.error, error, sizeof(error));
}
