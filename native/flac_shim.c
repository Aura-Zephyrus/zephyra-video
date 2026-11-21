/* Thin C bridge for FLAC decoding via vendored libFLAC (BSD-3). */
#include <FLAC/stream_decoder.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef void (*z_flac_frame_cb)(int channels, int sample_rate, int frames,
                                int bits, const int32_t *const *planes);

typedef struct {
    FLAC__StreamDecoder *dec;
    const uint8_t *data;
    size_t len;
    size_t pos;
    z_flac_frame_cb callback;
    int frames_emitted;
} z_flac_ctx;

static FLAC__StreamDecoderReadStatus read_cb(const FLAC__StreamDecoder *dec,
        FLAC__byte buffer[], size_t *bytes, void *client_data)
{
    z_flac_ctx *ctx = client_data;
    size_t available, n;
    (void)dec;
    if (*bytes == 0)
        return FLAC__STREAM_DECODER_READ_STATUS_ABORT;
    available = ctx->len - ctx->pos;
    n = *bytes < available ? *bytes : available;
    if (n > 0)
        memcpy(buffer, ctx->data + ctx->pos, n);
    ctx->pos += n;
    *bytes = n;
    return n == 0 ? FLAC__STREAM_DECODER_READ_STATUS_END_OF_STREAM
                  : FLAC__STREAM_DECODER_READ_STATUS_CONTINUE;
}

static FLAC__StreamDecoderWriteStatus write_cb(const FLAC__StreamDecoder *dec,
        const FLAC__Frame *frame, const FLAC__int32 *const buffer[], void *client_data)
{
    z_flac_ctx *ctx = client_data;
    (void)dec;
    if (ctx->callback) {
        ctx->frames_emitted++;
        ctx->callback(frame->header.channels, (int)frame->header.sample_rate,
                      (int)frame->header.blocksize, (int)frame->header.bits_per_sample,
                      buffer);
    }
    return FLAC__STREAM_DECODER_WRITE_STATUS_CONTINUE;
}

static void metadata_cb(const FLAC__StreamDecoder *dec,
        const FLAC__StreamMetadata *metadata, void *client_data)
{
    (void)dec;
    (void)metadata;
    (void)client_data;
}

static void error_cb(const FLAC__StreamDecoder *dec,
        FLAC__StreamDecoderErrorStatus status, void *client_data)
{
    (void)dec;
    (void)status;
    (void)client_data;
}

int z_flac_open(void **out)
{
    z_flac_ctx *ctx;
    if (!out)
        return -1;
    ctx = calloc(1, sizeof(*ctx));
    if (!ctx)
        return -1;
    ctx->dec = FLAC__stream_decoder_new();
    if (!ctx->dec) {
        free(ctx);
        return -1;
    }
    *out = ctx;
    return 0;
}

void z_flac_close(void *handle)
{
    z_flac_ctx *ctx = handle;
    if (!ctx)
        return;
    FLAC__stream_decoder_finish(ctx->dec);
    FLAC__stream_decoder_delete(ctx->dec);
    free(ctx);
}

int z_flac_decode(void *handle, const uint8_t *data, int len,
                  z_flac_frame_cb callback)
{
    z_flac_ctx *ctx = handle;
    FLAC__StreamDecoderInitStatus init;
    size_t last;

    if (!ctx || !data || len <= 0)
        return -1;
    FLAC__stream_decoder_finish(ctx->dec);
    ctx->data = data;
    ctx->len = (size_t)len;
    ctx->pos = 0;
    ctx->callback = callback;
    ctx->frames_emitted = 0;

    init = FLAC__stream_decoder_init_stream(ctx->dec, read_cb, NULL, NULL, NULL,
                                            NULL, write_cb, metadata_cb, error_cb, ctx);
    if (init != FLAC__STREAM_DECODER_INIT_STATUS_OK)
        return -1;

    /* The whole FLAC stream may be in one buffer; process frames until drained. */
    for (int i = 0; i < 10000; i++) {
        last = ctx->pos;
        if (!FLAC__stream_decoder_process_single(ctx->dec))
            break;
        int state = (int)FLAC__stream_decoder_get_state(ctx->dec);
        if (state == FLAC__STREAM_DECODER_END_OF_STREAM
            || state == FLAC__STREAM_DECODER_ABORTED)
            break;
        (void)last;
    }
    FLAC__stream_decoder_finish(ctx->dec);
    return ctx->frames_emitted > 0 ? 0 : -1;
}
