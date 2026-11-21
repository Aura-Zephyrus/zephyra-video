/* Thin C bridge for Opus decoding via vendored libopus (BSD-3). */
#include <opus.h>
#include <stdint.h>
#include <stdlib.h>

typedef void (*z_opus_frame_cb)(int frames, const int16_t *interleaved);

int z_opus_open(void **out, int sample_rate, int channels)
{
    int err = 0;
    OpusDecoder *dec;
    if (!out || sample_rate <= 0 || channels <= 0)
        return -1;
    dec = opus_decoder_create(sample_rate, channels, &err);
    if (!dec || err != OPUS_OK)
        return err == OPUS_OK ? -1 : err;
    *out = dec;
    return 0;
}

void z_opus_close(void *handle)
{
    if (handle)
        opus_decoder_destroy((OpusDecoder *)handle);
}

int z_opus_decode(void *handle, const uint8_t *data, int len,
                  z_opus_frame_cb callback)
{
    OpusDecoder *dec = handle;
    /* 120ms at 48kHz, the maximum Opus frame. */
    int16_t pcm[5760 * 8];
    int samples;

    if (!dec || !data || len <= 0)
        return -1;
    samples = opus_decode(dec, data, len, pcm, 5760, 0);
    if (samples < 0)
        return samples;
    if (samples > 0 && callback)
        callback(samples, pcm);
    return 0;
}
