/* Calls between window.c (vita2d display, input, dialogs) and gpu.c (GXM compositor). */
#ifndef CMVS_VITA_WINDOW_H
#define CMVS_VITA_WINDOW_H
#include <stdint.h>
#include <vita2d.h>

/* window.c */
int cmvs_vita_window_is_open(void);
/* Shows the top-left width x height texels of `texture` scaled to view = left, top,
   width, height in screen pixels, with an optional crosshair at a screen x, y. The
   window redraws the texture behind system dialogs until something else is shown or
   it is forgotten. */
int cmvs_vita_window_show(vita2d_texture *texture, uint32_t width, uint32_t height,
                          const float *view, int linear, const float *cursor);
/* Stops showing `texture` (about to be freed); the screen stays black until the next
   present. */
void cmvs_vita_window_forget(const vita2d_texture *texture);

/* gpu.c */
/* Ends a canvas pass a failed frame left open, so vita2d can begin its display scene. */
void cmvs_vita_gpu_end_pass(void);
void cmvs_vita_gpu_close(void);

#endif
