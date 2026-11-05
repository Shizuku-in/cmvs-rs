#include <switch.h>
#include <stdbool.h>
static Framebuffer fb;
static PadState pad;
static bool opened;
int cmvs_nx_open(void) {
    if (opened)
        return -1;
    Result rc = framebufferCreate(&fb, nwindowGetDefault(), 1280, 720, PIXEL_FORMAT_RGBA_8888, 2);
    if (R_FAILED(rc))
        return -1;
    rc = framebufferMakeLinear(&fb);
    if (R_FAILED(rc)) {
        framebufferClose(&fb);
        return -1;
    }
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    padInitializeDefault(&pad);
    opened = true;
    return 0;
}
unsigned cmvs_nx_poll(void) {
    if (!appletMainLoop())
        return 1;
    padUpdate(&pad);
    u64 held = padGetButtons(&pad);
    return ((held & HidNpadButton_Plus) ? 1 : 0) | ((held & HidNpadButton_A) ? 2 : 0) |
           ((held & HidNpadButton_B) ? 4 : 0) | ((held & HidNpadButton_Left) ? 8 : 0) |
           ((held & HidNpadButton_Right) ? 16 : 0) | ((held & HidNpadButton_Up) ? 32 : 0) |
           ((held & HidNpadButton_Down) ? 64 : 0);
}
void cmvs_nx_present(const u32 *pixels) {
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
    if (opened)
        framebufferClose(&fb);
    opened = false;
}
