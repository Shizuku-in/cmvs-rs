#ifndef CMVS_HOST_H
#define CMVS_HOST_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* API 1: calls for one handle must be serialized. Paths are UTF-8; game files
 * are read-only, user must be outside game. Destroy once, including failed open.
 * Pixels are RGBA8, tightly packed width*height*4; borrowed until next mutation.
 * Errors likewise belong to the handle. Native audio owns its callback thread. */
typedef struct CmvsHost CmvsHost;
uint32_t cmvs_host_api_version(void);
CmvsHost *cmvs_host_create(void);
int32_t cmvs_host_open(CmvsHost *, const char *game, const char *user,
                       const char *font, int32_t native_audio);
void cmvs_host_destroy(CmvsHost *);
/* Tick: -1 error, 0 cooperative slice, 1 complete frame, 2 quit, 3 inactive. */
int32_t cmvs_host_tick(CmvsHost *, uint32_t delta_ms);
/* CMVS codes: 1 left, 3 right, 4/5 wheel, 22 enter, 24 control, 25 escape,
 * 26 space, 33..36 arrows, 52..63 F1..F12. Pulses survive until a VM frame. */
int32_t cmvs_host_key(CmvsHost *, uint32_t code, int32_t pressed);
int32_t cmvs_host_pointer(CmvsHost *, int32_t x, int32_t y);
int32_t cmvs_host_pause(CmvsHost *, int32_t paused);
int32_t cmvs_host_save_preferences(CmvsHost *);
const char *cmvs_host_error(const CmvsHost *);
uint32_t cmvs_host_width(const CmvsHost *);
uint32_t cmvs_host_height(const CmvsHost *);
uint64_t cmvs_host_frame_serial(const CmvsHost *);
const uint8_t *cmvs_host_pixels(const CmvsHost *);
#ifdef __cplusplus
}
#endif
#endif
