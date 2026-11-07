#include "window.h"
#include <psp2/appmgr.h>
#include <psp2/apputil.h>
#include <psp2/common_dialog.h>
#include <psp2/ctrl.h>
#include <psp2/ime_dialog.h>
#include <psp2/message_dialog.h>
#include <psp2/system_param.h>
#include <psp2/touch.h>
#include <vita2d.h>
#include <stdbool.h>
#include <stdint.h>
#include <string.h>

/* Must match `VitaInput` in crates/cmvs-native/src/vita/window.rs. */
#define CMVS_VITA_TOUCHES 4
typedef struct {
    uint32_t id;
    uint32_t x;
    uint32_t y;
} CmvsVitaTouch;
typedef struct {
    uint32_t buttons;
    uint32_t flags;
    int32_t sticks[2][2];
    uint32_t touch_count;
    CmvsVitaTouch touches[CMVS_VITA_TOUCHES];
} CmvsVitaInput;
enum {
    CMVS_FLAG_FOCUSED = 2,
};

#define SCREEN_WIDTH 960
#define SCREEN_HEIGHT 544

static bool opened;
static bool confirm_cross;
/* Software frames are uploaded here. */
static vita2d_texture *frame;
static unsigned frame_width, frame_height;
/* The last presented texture (`frame` or the GPU canvas), redrawn behind system
   dialogs, and the part of it in use. */
static vita2d_texture *shown;
static unsigned shown_width, shown_height;
static float frame_view[4];
static bool has_frame;
static bool has_cursor;
static float cursor_at[2];

/* vita2d owns GXM, the display queue and the common dialog render target. */
int cmvs_vita_open(void) {
    if (opened)
        return -1;
    SceAppUtilInitParam init;
    SceAppUtilBootParam boot;
    memset(&init, 0, sizeof(init));
    memset(&boot, 0, sizeof(boot));
    sceAppUtilInit(&init, &boot);
    int enter = SCE_SYSTEM_PARAM_ENTER_BUTTON_CIRCLE;
    sceAppUtilSystemParamGetInt(SCE_SYSTEM_PARAM_ID_ENTER_BUTTON, &enter);
    confirm_cross = enter == SCE_SYSTEM_PARAM_ENTER_BUTTON_CROSS;
    SceCommonDialogConfigParam config;
    sceCommonDialogConfigParamInit(&config);
    int language = SCE_SYSTEM_PARAM_LANG_JAPANESE;
    sceAppUtilSystemParamGetInt(SCE_SYSTEM_PARAM_ID_LANG, &language);
    config.language = (SceSystemParamLang)language;
    config.enterButtonAssign = (SceSystemParamEnterButtonAssign)enter;
    sceCommonDialogSetConfigParam(&config);
    if (vita2d_init() <= 0) {
        sceAppUtilShutdown();
        return -1;
    }
    vita2d_set_clear_color(0xff000000);
    sceCtrlSetSamplingMode(SCE_CTRL_MODE_ANALOG_WIDE);
    sceTouchSetSamplingState(SCE_TOUCH_PORT_FRONT, SCE_TOUCH_SAMPLING_STATE_START);
    has_frame = false;
    has_cursor = false;
    opened = true;
    return 0;
}

int cmvs_vita_window_is_open(void) {
    return opened;
}

int cmvs_vita_confirm_is_cross(void) {
    return confirm_cross;
}

static uint32_t map_buttons(uint32_t held) {
    /* Bit order of `vita_button` in crates/cmvs-native/src/vita/input.rs. */
    static const uint32_t order[] = {
        SCE_CTRL_CROSS,    SCE_CTRL_CIRCLE,   SCE_CTRL_SQUARE, SCE_CTRL_TRIANGLE,
        SCE_CTRL_LTRIGGER, SCE_CTRL_RTRIGGER, SCE_CTRL_SELECT, SCE_CTRL_START,
        SCE_CTRL_UP,       SCE_CTRL_DOWN,     SCE_CTRL_LEFT,   SCE_CTRL_RIGHT,
    };
    uint32_t bits = 0;
    for (unsigned i = 0; i < sizeof(order) / sizeof(order[0]); ++i) {
        if (held & order[i])
            bits |= 1u << i;
    }
    return bits;
}

/* 0..255 with 128 at rest to -32767..32767; `flip` turns screen-down into +y up. */
static int32_t axis(uint8_t value, bool flip) {
    int32_t centred = (int32_t)value - 128;
    int32_t scaled = centred * 32767 / 127;
    if (scaled > 32767)
        scaled = 32767;
    if (scaled < -32767)
        scaled = -32767;
    return flip ? -scaled : scaled;
}

static uint32_t clamp_u32(int32_t value, uint32_t limit) {
    if (value < 0)
        return 0;
    return (uint32_t)value >= limit ? limit - 1 : (uint32_t)value;
}

void cmvs_vita_poll(CmvsVitaInput *out) {
    memset(out, 0, sizeof(*out));
    SceCtrlData pad;
    memset(&pad, 0, sizeof(pad));
    if (sceCtrlPeekBufferPositive(0, &pad, 1) >= 0) {
        out->buttons = map_buttons(pad.buttons);
        out->sticks[0][0] = axis(pad.lx, false);
        out->sticks[0][1] = axis(pad.ly, true);
        out->sticks[1][0] = axis(pad.rx, false);
        out->sticks[1][1] = axis(pad.ry, true);
    }
    SceTouchData touch;
    memset(&touch, 0, sizeof(touch));
    if (sceTouchPeek(SCE_TOUCH_PORT_FRONT, &touch, 1) >= 0) {
        unsigned count = touch.reportNum < CMVS_VITA_TOUCHES ? touch.reportNum : CMVS_VITA_TOUCHES;
        for (unsigned i = 0; i < count; ++i) {
            /* The front panel reports 1920x1088 for the 960x544 screen. */
            out->touches[i] = (CmvsVitaTouch){
                .id = touch.report[i].id,
                .x = clamp_u32(touch.report[i].x / 2, SCREEN_WIDTH),
                .y = clamp_u32(touch.report[i].y / 2, SCREEN_HEIGHT),
            };
        }
        out->touch_count = count;
    }
    bool focused = true;
    SceAppMgrAppState state;
    memset(&state, 0, sizeof(state));
    if (_sceAppMgrGetAppState(&state, sizeof(state), PSP2_SDK_VERSION) >= 0) {
        focused = !state.isSystemUiOverlaid;
        /* Threads stop while suspended; after a resume, report one unfocused sample
           so the host drops held input and restarts its clocks. */
        for (SceUInt32 i = 0; i < state.systemEventNum; ++i) {
            SceAppMgrSystemEvent event;
            memset(&event, 0, sizeof(event));
            if (sceAppMgrReceiveSystemEvent(&event) < 0)
                break;
            if (event.systemEvent == SCE_APPMGR_SYSTEMEVENT_ON_RESUME)
                focused = false;
        }
    }
    if (focused)
        out->flags |= CMVS_FLAG_FOCUSED;
}

static void draw(void) {
    cmvs_vita_gpu_end_pass();
    vita2d_start_drawing();
    /* The GPU canvas pass sets its own viewport. */
    sceGxmSetViewport(vita2d_get_context(), SCREEN_WIDTH / 2.0f, SCREEN_WIDTH / 2.0f,
                      SCREEN_HEIGHT / 2.0f, -SCREEN_HEIGHT / 2.0f, 0.5f, 0.5f);
    vita2d_clear_screen();
    if (has_frame) {
        vita2d_draw_texture_part_scale(shown, frame_view[0], frame_view[1], 0.0f, 0.0f,
                                       (float)shown_width, (float)shown_height,
                                       frame_view[2] / (float)shown_width,
                                       frame_view[3] / (float)shown_height);
    }
    if (has_cursor) {
        vita2d_draw_rectangle(cursor_at[0] - 5.0f, cursor_at[1], 11.0f, 1.0f, 0xffffffff);
        vita2d_draw_rectangle(cursor_at[0], cursor_at[1] - 5.0f, 1.0f, 11.0f, 0xffffffff);
    }
    vita2d_end_drawing();
    vita2d_common_dialog_update();
    vita2d_swap_buffers();
}

/* `rgba` holds width*height tightly packed pixels shown at view = left, top, width,
   height in screen pixels; `cursor` is an optional screen x, y crosshair. */
int cmvs_vita_present(const uint8_t *rgba, uint32_t width, uint32_t height, const float *view,
                      int linear, const float *cursor) {
    if (!opened || width == 0 || height == 0 || width > 4096 || height > 4096)
        return -1;
    /* The texture is rewritten in place: wait until the GPU has read it. */
    vita2d_wait_rendering_done();
    if (!frame || frame_width != width || frame_height != height) {
        if (frame)
            vita2d_free_texture(frame);
        if (shown == frame)
            has_frame = false;
        frame = vita2d_create_empty_texture(width, height);
        if (!frame)
            return -2;
        frame_width = width;
        frame_height = height;
    }
    uint8_t *texels = vita2d_texture_get_datap(frame);
    unsigned stride = vita2d_texture_get_stride(frame);
    for (uint32_t y = 0; y < height; ++y)
        memcpy(texels + (size_t)y * stride, rgba + (size_t)y * width * 4, (size_t)width * 4);
    return cmvs_vita_window_show(frame, width, height, view, linear, cursor);
}

int cmvs_vita_window_show(vita2d_texture *texture, uint32_t width, uint32_t height,
                          const float *view, int linear, const float *cursor) {
    if (!opened || !texture || width == 0 || height == 0)
        return -1;
    SceGxmTextureFilter filter =
        linear ? SCE_GXM_TEXTURE_FILTER_LINEAR : SCE_GXM_TEXTURE_FILTER_POINT;
    vita2d_texture_set_filters(texture, filter, filter);
    shown = texture;
    shown_width = width;
    shown_height = height;
    memcpy(frame_view, view, sizeof(frame_view));
    has_frame = true;
    has_cursor = cursor != NULL;
    if (cursor)
        memcpy(cursor_at, cursor, sizeof(cursor_at));
    draw();
    return 0;
}

void cmvs_vita_window_forget(const vita2d_texture *texture) {
    if (shown == texture) {
        has_frame = false;
        shown = NULL;
    }
}

static void run_dialog(SceCommonDialogStatus (*status)(void)) {
    while (status() == SCE_COMMON_DIALOG_STATUS_RUNNING)
        draw();
}

/* UTF-16 title and initial text; `out` holds out_len units including the terminator.
   Returns 1 on enter, 0 on cancel, -1 when the dialog cannot be shown. */
int cmvs_vita_ime(const uint16_t *title, const uint16_t *initial, uint32_t max_chars, uint16_t *out,
                  uint32_t out_len) {
    if (!opened || max_chars == 0 || max_chars > SCE_IME_DIALOG_MAX_TEXT_LENGTH ||
        out_len <= max_chars)
        return -1;
    memset(out, 0, out_len * sizeof(uint16_t));
    SceImeDialogParam param;
    sceImeDialogParamInit(&param);
    param.supportedLanguages = SCE_IME_LANGUAGE_JAPANESE | SCE_IME_LANGUAGE_ENGLISH;
    param.languagesForced = SCE_FALSE;
    param.type = SCE_IME_TYPE_DEFAULT;
    param.option = 0;
    param.dialogMode = SCE_IME_DIALOG_DIALOG_MODE_WITH_CANCEL;
    param.textBoxMode = SCE_IME_DIALOG_TEXTBOX_MODE_WITH_CLEAR;
    param.title = title;
    param.maxTextLength = max_chars;
    param.initialText = (SceWChar16 *)initial;
    param.inputTextBuffer = out;
    if (sceImeDialogInit(&param) < 0)
        return -1;
    run_dialog(sceImeDialogGetStatus);
    SceImeDialogResult result;
    memset(&result, 0, sizeof(result));
    int got = sceImeDialogGetResult(&result);
    sceImeDialogTerm();
    if (got < 0)
        return -1;
    return result.button == SCE_IME_DIALOG_BUTTON_ENTER ? 1 : 0;
}

/* A modal OK dialog over the last frame; `text` is UTF-8 of at most 511 bytes. */
int cmvs_vita_message(const char *text) {
    if (!opened || strlen(text) >= SCE_MSG_DIALOG_USER_MSG_SIZE)
        return -1;
    SceMsgDialogUserMessageParam user;
    memset(&user, 0, sizeof(user));
    user.buttonType = SCE_MSG_DIALOG_BUTTON_TYPE_OK;
    user.msg = (const SceChar8 *)text;
    SceMsgDialogParam param;
    sceMsgDialogParamInit(&param);
    param.mode = SCE_MSG_DIALOG_MODE_USER_MSG;
    param.userMsgParam = &user;
    if (sceMsgDialogInit(&param) < 0)
        return -1;
    run_dialog(sceMsgDialogGetStatus);
    sceMsgDialogTerm();
    return 0;
}

void cmvs_vita_close(void) {
    if (!opened)
        return;
    cmvs_vita_gpu_close();
    vita2d_wait_rendering_done();
    if (frame)
        vita2d_free_texture(frame);
    frame = NULL;
    shown = NULL;
    has_frame = false;
    vita2d_fini();
    sceAppUtilShutdown();
    opened = false;
}
