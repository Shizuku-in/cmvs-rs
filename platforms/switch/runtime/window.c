#include <switch.h>
#include <stdbool.h>
#include <string.h>

/* Must match `NxInput` in crates/cmvs-native/src/switch/window.rs. */
#define CMVS_NX_TOUCHES 4
typedef struct {
    u32 id;
    u32 x;
    u32 y;
} CmvsNxTouch;
typedef struct {
    u32 buttons;
    u32 flags;
    s32 sticks[2][2];
    u32 touch_count;
    CmvsNxTouch touches[CMVS_NX_TOUCHES];
} CmvsNxInput;
enum {
    CMVS_FLAG_EXIT = 1,
    CMVS_FLAG_FOCUSED = 2,
};

static Framebuffer fb;
static PadState pad;
static bool opened;
static bool has_framebuffer;
static bool exit_locked;

/* Without the software framebuffer, the default window is left to the deko3d
   swapchain in gpu.c. */
int cmvs_nx_open(int framebuffer) {
    if (opened)
        return -1;
    if (framebuffer) {
        Result rc =
            framebufferCreate(&fb, nwindowGetDefault(), 1280, 720, PIXEL_FORMAT_RGBA_8888, 2);
        if (R_FAILED(rc))
            return -1;
        rc = framebufferMakeLinear(&fb);
        if (R_FAILED(rc)) {
            framebufferClose(&fb);
            return -1;
        }
    }
    has_framebuffer = framebuffer != 0;
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    padInitializeDefault(&pad);
    hidInitializeTouchScreen();
    /* Closing from HOME then arrives as an exit request instead of killing the
       process, so the host can persist settings. Unavailable in applet mode. */
    exit_locked = R_SUCCEEDED(appletLockExit());
    opened = true;
    return 0;
}

static u32 map_buttons(u64 held) {
    static const u64 order[] = {
        HidNpadButton_Plus, HidNpadButton_A,     HidNpadButton_B,      HidNpadButton_X,
        HidNpadButton_Y,    HidNpadButton_L,     HidNpadButton_R,      HidNpadButton_ZL,
        HidNpadButton_ZR,   HidNpadButton_Minus, HidNpadButton_Up,     HidNpadButton_Down,
        HidNpadButton_Left, HidNpadButton_Right, HidNpadButton_StickL, HidNpadButton_StickR,
    };
    u32 result = 0;
    for (unsigned i = 0; i < sizeof(order) / sizeof(order[0]); ++i) {
        if (held & order[i])
            result |= 1u << i;
    }
    return result;
}

void cmvs_nx_poll(CmvsNxInput *out) {
    memset(out, 0, sizeof(*out));
    if (!appletMainLoop()) {
        out->flags = CMVS_FLAG_EXIT;
        return;
    }
    if (appletGetFocusState() == AppletFocusState_InFocus)
        out->flags |= CMVS_FLAG_FOCUSED;
    padUpdate(&pad);
    out->buttons = map_buttons(padGetButtons(&pad));
    for (unsigned i = 0; i < 2; ++i) {
        HidAnalogStickState stick = padGetStickPos(&pad, i);
        out->sticks[i][0] = stick.x;
        out->sticks[i][1] = stick.y;
    }
    HidTouchScreenState touch = {0};
    if (hidGetTouchScreenStates(&touch, 1) != 0) {
        u32 count = touch.count < CMVS_NX_TOUCHES ? touch.count : CMVS_NX_TOUCHES;
        for (u32 i = 0; i < count; ++i) {
            out->touches[i].id = touch.touches[i].finger_id;
            out->touches[i].x = touch.touches[i].x;
            out->touches[i].y = touch.touches[i].y;
        }
        out->touch_count = count;
    }
}

void cmvs_nx_present(const u32 *pixels) {
    if (!has_framebuffer)
        return;
    u32 stride;
    u8 *out = framebufferBegin(&fb, &stride);
    for (unsigned y = 0; y < 720; ++y) {
        u32 *row = (u32 *)(out + y * stride);
        for (unsigned x = 0; x < 1280; ++x) {
            u32 p = pixels[y * 1280 + x]; /* core output is 0x00RRGGBB */
            row[x] = 0xff000000 | ((p & 255) << 16) | (p & 0xff00) | ((p >> 16) & 255);
        }
    }
    framebufferEnd(&fb);
}

void cmvs_nx_close(void) {
    if (has_framebuffer)
        framebufferClose(&fb);
    has_framebuffer = false;
    if (exit_locked)
        appletUnlockExit();
    exit_locked = false;
    opened = false;
}
