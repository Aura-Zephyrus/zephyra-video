/* Thin C bridge for Vorbis decoding via vendored libvorbis (BSD-3). */
#include <vorbis/codec.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef void (*z_vorbis_frame_cb)(int channels, int sample_rate, int frames,
                                  const float *const *planes);

typedef struct {
    vorbis_info vi;
    vorbis_comment vc;
    vorbis_dsp_state vd;
    vorbis_block vb;
    int headers_done;
    int headers_seen;
    int packets_seen;
} z_vorbis_ctx;

int z_vorbis_open(void **out)
{
    z_vorbis_ctx *ctx;
    if (!out)
        return -1;
    ctx = calloc(1, sizeof(*ctx));
    if (!ctx)
        return -1;
    vorbis_info_init(&ctx->vi);
    vorbis_comment_init(&ctx->vc);
    *out = ctx;
    return 0;
}

void z_vorbis_close(void *handle)
{
    z_vorbis_ctx *ctx = handle;
    if (!ctx)
        return;
    if (ctx->headers_done) {
        vorbis_block_clear(&ctx->vb);
        vorbis_dsp_clear(&ctx->vd);
    }
    vorbis_comment_clear(&ctx->vc);
    vorbis_info_clear(&ctx->vi);
    free(ctx);
}

int z_vorbis_decode(void *handle, const uint8_t *data, int len,
                    z_vorbis_frame_cb callback)
{
    z_vorbis_ctx *ctx = handle;
    ogg_packet op;
    int ret;

    if (!ctx || !data || len <= 0)
        return -1;
    memset(&op, 0, sizeof(op));
    op.packet = (unsigned char *)data;
    op.bytes = len;
    op.b_o_s = ctx->packets_seen == 0;
    op.packetno = ctx->packets_seen;
    ctx->packets_seen++;

    if (!ctx->headers_done) {
        ret = vorbis_synthesis_headerin(&ctx->vi, &ctx->vc, &op);
        if (ret < 0)
            return ret;
        ctx->headers_seen++;
        if (ctx->headers_seen < 3)
            return 0;
        if (vorbis_synthesis_init(&ctx->vd, &ctx->vi) != 0)
            return -1;
        if (vorbis_block_init(&ctx->vd, &ctx->vb) != 0)
            return -1;
        ctx->headers_done = 1;
        return 0;
    }

    if (vorbis_synthesis(&ctx->vb, &op) != 0)
        return -1;
    if (vorbis_synthesis_blockin(&ctx->vd, &ctx->vb) != 0)
        return -1;

    for (;;) {
        float **pcm;
        int samples = vorbis_synthesis_pcmout(&ctx->vd, &pcm);
        if (samples <= 0)
            break;
        if (callback)
            callback(ctx->vi.channels, ctx->vi.rate, samples, (const float *const *)pcm);
        vorbis_synthesis_read(&ctx->vd, samples);
    }
    return 0;
}
