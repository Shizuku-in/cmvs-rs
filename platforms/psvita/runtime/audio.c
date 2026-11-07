#include <psp2/audioout.h>
#include <stdbool.h>
#include <stdint.h>
#include <string.h>

/* One 48 kHz stereo BGM port. sceAudioOutOutput keeps reading a buffer until the next
   call returns, so blocks alternate between two copies: at most two 1024-frame blocks
   (42.7 ms) are queued. One thread owns these calls. */
#define FRAMES 1024
#define CHANNELS 2
#define RATE 48000

static int16_t buffers[2][FRAMES * CHANNELS] __attribute__((aligned(64)));
static unsigned next;
static int port = -1;

int cmvs_vita_audio_open(uint32_t *rate, uint32_t *channels, uint32_t *frames) {
    if (port >= 0)
        return -1;
    port =
        sceAudioOutOpenPort(SCE_AUDIO_OUT_PORT_TYPE_BGM, FRAMES, RATE, SCE_AUDIO_OUT_MODE_STEREO);
    if (port < 0)
        return -1;
    int volume[2] = {SCE_AUDIO_OUT_MAX_VOL, SCE_AUDIO_OUT_MAX_VOL};
    sceAudioOutSetVolume(port, SCE_AUDIO_VOLUME_FLAG_L_CH | SCE_AUDIO_VOLUME_FLAG_R_CH, volume);
    next = 0;
    *rate = RATE;
    *channels = CHANNELS;
    *frames = FRAMES;
    return 0;
}

/* Queues exactly FRAMES * CHANNELS samples, blocking while the previous block plays. */
int cmvs_vita_audio_submit(const int16_t *pcm) {
    if (port < 0)
        return -1;
    memcpy(buffers[next], pcm, sizeof(buffers[next]));
    int result = sceAudioOutOutput(port, buffers[next]);
    next ^= 1;
    return result < 0 ? -1 : 0;
}

void cmvs_vita_audio_close(void) {
    if (port < 0)
        return;
    /* A null block waits for the queued ones to finish. */
    sceAudioOutOutput(port, NULL);
    sceAudioOutReleasePort(port);
    port = -1;
}
