/*
 * Thin C bridge for the vendored libtheora decoder (BSD-3, Xiph.org).
 *
 * Theora packets live inside Ogg pages; callers feed one decoded packet per
 * DecoderBuffer. The first three packets are headers, consumed by
 * th_decode_headerin(), after which th_decode_alloc() creates the context.
 * th_ycbcr_buffer is flattened into scalar callback arguments.
 * The vendored sources under theora/ are never modified.
 */
#include <theora/theora.h>
#include <theora/theoradec.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    th_info info;
    th_comment comment;
    th_setup_info *setup;
    th_dec_ctx *dec;
    int headers_done;
} z_theora_ctx;

typedef void (*z_theora_frame_cb)(int w, int h, int bpc, int layout,
                                  const uint8_t *y, ptrdiff_t ys,
                                  const uint8_t *u, ptrdiff_t us,
                                  const uint8_t *v, ptrdiff_t vs);

int z_theora_open(void **out)
{
    z_theora_ctx *ctx;

    if (!out)
        return -1;
    ctx = calloc(1, sizeof(*ctx));
    if (!ctx)
        return -1;
    th_info_init(&ctx->info);
    th_comment_init(&ctx->comment);
    *out = ctx;
    return 0;
}

void z_theora_close(void *handle)
{
    z_theora_ctx *ctx = handle;

    if (!ctx)
        return;
    if (ctx->dec)
        th_decode_free(ctx->dec);
    if (ctx->setup)
        th_setup_free(ctx->setup);
    th_comment_clear(&ctx->comment);
    th_info_clear(&ctx->info);
    free(ctx);
}

int z_theora_decode(void *handle, const uint8_t *data, int len,
                    z_theora_frame_cb callback)
{
    z_theora_ctx *ctx = handle;
    ogg_packet op;
    th_ycbcr_buffer buf;
    int ret;

    if (!ctx || !data || len <= 0)
        return -1;

    memset(&op, 0, sizeof(op));
    op.packet = (unsigned char *)data;
    op.bytes = len;

    if (!ctx->headers_done) {
        ret = th_decode_headerin(&ctx->info, &ctx->comment, &ctx->setup, &op);
        if (ret > 0)
            return 0; /* more header packets to come */
        if (ret == 0) {
            ctx->headers_done = 1;
            ctx->dec = th_decode_alloc(&ctx->info, ctx->setup);
            return ctx->dec ? 0 : -1;
        }
        return ret;
    }

    ret = th_decode_packetin(ctx->dec, &op, NULL);
    if (ret < 0)
        return ret;

    if (th_decode_ycbcr_out(ctx->dec, buf) != 0)
        return -1;

    if (callback && buf[0].data && buf[1].data && buf[2].data &&
        buf[0].stride > 0 && buf[1].stride > 0 && buf[2].stride > 0) {
        /* libtheora always outputs 8-bit 4:2:0 (layout 1). */
        callback(buf[0].width, buf[0].height, 8, 1,
                 buf[0].data, (ptrdiff_t)buf[0].stride,
                 buf[1].data, (ptrdiff_t)buf[1].stride,
                 buf[2].data, (ptrdiff_t)buf[2].stride);
    }
    return 0;
}
