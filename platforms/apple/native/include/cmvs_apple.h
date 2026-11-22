#pragma once
#include <stdbool.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Serialize all calls per handle. Copy borrowed text/pixels before the next
 * mutation. All strings are NUL-terminated UTF-8. Status -1 sets error text. */
typedef struct CmvsApple CmvsApple;
CmvsApple *cmvs_apple_create(void);
void cmvs_apple_destroy(CmvsApple *);
const char *cmvs_apple_text(const CmvsApple *);
int32_t cmvs_apple_list_profiles(CmvsApple *);
/* NULL profile and encoding select automatic inspection. */
int32_t cmvs_apple_inspect_game(CmvsApple *, const char *path, const char *profile, const char *encoding);
int32_t cmvs_apple_validate_font(CmvsApple *, const char *path, uint32_t face);
/* Returns JSON face records with index, PostScript name, and CJK coverage. */
int32_t cmvs_apple_inspect_font(CmvsApple *, const char *path);
int32_t cmvs_apple_open(CmvsApple *, const char *config_json);
/* Retain the CAMetalLayer until detach returns. Dimensions are physical pixels. */
int32_t cmvs_apple_attach_metal_layer(CmvsApple *, void *, uint32_t width, uint32_t height);
/* Returns the presentation rectangle in physical pixels (top-left origin). */
int32_t cmvs_apple_resize(CmvsApple *, uint32_t width, uint32_t height, bool windowed, bool resizable, int32_t rect[4]);
int32_t cmvs_apple_fullscreen(CmvsApple *, bool fullscreen);
int32_t cmvs_apple_detach(CmvsApple *);
/* 0 budget slice, 1 completed frame, 2 quit, 3 inactive */
int32_t cmvs_apple_tick(CmvsApple *, uint32_t delta_ms);
typedef struct {
    bool sampled;
    bool primary_held;
    bool touch_position_sampled;
} CmvsAppleTickInput;
/* Same status as cmvs_apple_tick. Fills input even if the tick fails. */
int32_t cmvs_apple_tick_traced(CmvsApple *, uint32_t delta_ms, CmvsAppleTickInput *input);
/* Returns 1 with one JSON event in text, or 0 if empty. After a stopped
 * tick, the first event includes the script, PC, step count and reason. */
int32_t cmvs_apple_next_event(CmvsApple *);
/* Returns 1 with the changed accessible page in text; activations refer to that revision. */
int32_t cmvs_apple_accessibility(CmvsApple *, uint64_t known_revision);
/* Pass -1 for a button or 0..255 for a color slider. */
int32_t cmvs_apple_choose(CmvsApple *, uint64_t revision, uint32_t index, int32_t slider);
int32_t cmvs_apple_key(CmvsApple *, uint32_t code, bool pressed);
int32_t cmvs_apple_pointer(CmvsApple *, double x, double y);
/* Physical surface pixels. Touch phases are begin/move/end/cancel (0..3). */
int32_t cmvs_apple_touch(CmvsApple *, uint64_t id, uint32_t phase, double x, double y);
#if defined(__APPLE__)
#include <TargetConditionals.h>
#endif
#if defined(TARGET_OS_IOS) && TARGET_OS_IOS
int32_t cmvs_apple_restart_audio(CmvsApple *);
#endif
int32_t cmvs_apple_pause(CmvsApple *, bool paused);
int32_t cmvs_apple_save_preferences(CmvsApple *);
int32_t cmvs_apple_comment(CmvsApple *, uint32_t id, const char *text);
/* Selects an ID from the font catalog passed to open and returns its save reference. NULL cancels. */
int32_t cmvs_apple_font(CmvsApple *, const char *id);
int32_t cmvs_apple_message(CmvsApple *, bool accept);
/* A NULL local_root exports the standalone user save directory. */
int32_t cmvs_apple_export_saves(CmvsApple *, const char *local_root, const char *user, const char *zip, const char *identity);
int32_t cmvs_apple_restore_saves(CmvsApple *, const char *game, const char *profile, const char *encoding, const char *user, bool local, const char *zip);
/* Encoded byte count, or -1 if the text cannot be encoded. */
int32_t cmvs_apple_comment_bytes(CmvsApple *, const char *text);
int32_t cmvs_apple_comment_valid(CmvsApple *, const char *text, uint32_t chars, uint32_t bytes);
int32_t cmvs_apple_read_frame(CmvsApple *, const uint8_t **pixels);
/* Full-size software presentation when a scale filter override is active. Same pointer lifetime. */
int32_t cmvs_apple_read_presentation(CmvsApple *, const uint8_t **pixels, uint32_t size[2]);
#ifdef __cplusplus
}
#endif
