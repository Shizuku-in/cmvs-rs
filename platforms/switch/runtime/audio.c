#include <switch.h>
#include <malloc.h>
#include <stdbool.h>
#include <string.h>

/* audout at its fixed 48 kHz stereo signed 16-bit format. Four 1024-frame buffers
   (21.3 ms each) keep at most about 85 ms queued. One thread owns these calls. */
#define BUFFERS 4
#define FRAMES 1024
#define CHANNELS 2
#define BYTES ((FRAMES * CHANNELS * sizeof(s16) + 0xfff) & ~(size_t)0xfff)

static AudioOutBuffer buffers[BUFFERS];
static unsigned unused;
static bool opened;

void cmvs_nx_audio_close(void);

int cmvs_nx_audio_open(u32 *rate, u32 *channels, u32 *frames) {
    if (opened)
        return -1;
    if (R_FAILED(audoutInitialize()))
        return -1;
    if (audoutGetChannelCount() != CHANNELS || audoutGetPcmFormat() != PcmFormat_Int16) {
        audoutExit();
        return -1;
    }
    opened = true;
    for (unsigned i = 0; i < BUFFERS; ++i) {
        void *data = memalign(0x1000, BYTES);
        if (!data) {
            cmvs_nx_audio_close();
            return -1;
        }
        memset(data, 0, BYTES);
        buffers[i] = (AudioOutBuffer){
            .buffer = data,
            .buffer_size = BYTES,
            .data_size = FRAMES * CHANNELS * sizeof(s16),
        };
    }
    if (R_FAILED(audoutStartAudioOut())) {
        cmvs_nx_audio_close();
        return -1;
    }
    unused = BUFFERS;
    *rate = audoutGetSampleRate();
    *channels = CHANNELS;
    *frames = FRAMES;
    return 0;
}

/* Queues exactly FRAMES * CHANNELS samples, blocking while every buffer is queued. */
int cmvs_nx_audio_submit(const s16 *pcm) {
    AudioOutBuffer *target;
    if (unused) {
        target = &buffers[BUFFERS - unused--];
    } else {
        /* Each call returns at most one released buffer, while the release event
           may cover several: drain the queue before waiting on the event again. */
        u32 count = 0;
        AudioOutBuffer *released = NULL;
        if (R_FAILED(audoutGetReleasedAudioOutBuffer(&released, &count)))
            return -1;
        while (!count || !released) {
            if (R_FAILED(audoutWaitPlayFinish(&released, &count, UINT64_MAX)))
                return -1;
        }
        target = released;
    }
    memcpy(target->buffer, pcm, target->data_size);
    armDCacheFlush(target->buffer, target->buffer_size);
    return R_SUCCEEDED(audoutAppendAudioOutBuffer(target)) ? 0 : -1;
}

/* Runs the calling thread on core 1, away from the main thread's VM and rendering
   on core 0, so decoding keeps up during long frames. Applications own cores 0..2. */
int cmvs_nx_audio_prefer_core(void) {
    return R_SUCCEEDED(svcSetThreadCoreMask(CUR_THREAD_HANDLE, 1, 1u << 1)) ? 0 : -1;
}

void cmvs_nx_audio_close(void) {
    if (!opened)
        return;
    audoutStopAudioOut();
    audoutExit();
    for (unsigned i = 0; i < BUFFERS; ++i) {
        free(buffers[i].buffer);
        buffers[i].buffer = NULL;
    }
    opened = false;
}
