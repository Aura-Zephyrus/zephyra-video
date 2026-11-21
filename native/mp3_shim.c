/*
 * Thin C bridge for MP3 decoding via vendored minimp3 (CC0).
 * Flattens each decoded frame into scalar arguments so Rust never mirrors
 * mp3dec_frame_info_t. The vendored header is never modified.
 */
#define MINIMP3_IMPLEMENTATION
#include <minimp3.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef void (*z_mp3_frame_cb)(int channels, int sample_rate, int frames,
                               const int16_t *interleaved);

int z_mp3_open(void **out)
{
    mp3dec_t *dec;
    if (!out)
        return -1;
    dec = malloc(sizeof(*dec));
    if (!dec)
        return -1;
    mp3dec_init(dec);
    *out = dec;
    return 0;
}

void z_mp3_close(void *handle)
{
    free(handle);
}

int z_mp3_decode(void *handle, const uint8_t *data, int len,
                 z_mp3_frame_cb callback)
{
    mp3dec_t *dec = handle;
    mp3dec_frame_info_t info;
    int16_t pcm[MINIMP3_MAX_SAMPLES_PER_FRAME];
    int offset = 0;

    if (!dec || !data || len <= 0)
        return -1;

    while (offset < len) {
        int samples;
        memset(&info, 0, sizeof(info));
        samples = mp3dec_decode_frame(dec, data + offset, len - offset, pcm, &info);
        if (info.frame_bytes <= 0)
            break;
        offset += info.frame_bytes;
        if (samples > 0 && info.channels > 0 && info.hz > 0 && callback)
            callback(info.channels, info.hz, samples, pcm);
    }
    return 0;
}
