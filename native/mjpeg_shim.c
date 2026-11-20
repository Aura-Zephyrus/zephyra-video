/*
 * Thin C bridge for MJPEG decoding via vendored libjpeg-turbo (BSD-3 / IJG).
 *
 * Each DecoderBuffer must contain exactly one complete JPEG frame. The shim
 * asks libjpeg for full-resolution YCbCr, box-filters chroma down to 4:2:0,
 * and flattens the three planes into scalar callback arguments. libjpeg's
 * default error handler calls exit(1), so we replace it with setjmp/longjmp.
 * The vendored sources under libjpeg/ are never modified.
 */
#include <stdio.h> /* jpeglib.h uses FILE in its API */
#include <jpeglib.h>
#include <setjmp.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

struct z_error_mgr {
    struct jpeg_error_mgr pub;
    jmp_buf setjmp_buffer;
};

struct z_mjpeg_ctx {
    struct jpeg_decompress_struct cinfo;
    struct z_error_mgr jerr;
    int ready;
};

typedef void (*z_mjpeg_frame_cb_t)(int w, int h, int bpc, int layout,
                                    const uint8_t *y, ptrdiff_t ys,
                                    const uint8_t *u, ptrdiff_t us,
                                    const uint8_t *v, ptrdiff_t vs);

static void z_error_exit(j_common_ptr cinfo)
{
    struct z_error_mgr *err = (struct z_error_mgr *)cinfo->err;
    longjmp(err->setjmp_buffer, 1);
}

static void z_no_message(j_common_ptr cinfo)
{
    (void)cinfo;
}

int z_mjpeg_open(void **out)
{
    struct z_mjpeg_ctx *ctx;

    if (!out)
        return -1;
    ctx = calloc(1, sizeof(*ctx));
    if (!ctx)
        return -1;
    ctx->cinfo.err = jpeg_std_error(&ctx->jerr.pub);
    ctx->jerr.pub.error_exit = z_error_exit;
    ctx->jerr.pub.output_message = z_no_message;
    jpeg_create_decompress(&ctx->cinfo);
    ctx->ready = 1;
    *out = ctx;
    return 0;
}

void z_mjpeg_close(void *handle)
{
    struct z_mjpeg_ctx *ctx = handle;

    if (!ctx)
        return;
    if (ctx->ready)
        jpeg_destroy_decompress(&ctx->cinfo);
    free(ctx);
}

int z_mjpeg_decode(void *handle, const uint8_t *data, int len,
                   z_mjpeg_frame_cb_t callback)
{
    struct z_mjpeg_ctx *ctx = handle;
    struct jpeg_decompress_struct *cinfo;
    uint8_t *full = NULL;
    uint8_t *y = NULL, *u = NULL, *v = NULL, *row = NULL;
    int w, h, cw, ch, x, y0;
    int ret = -1;

    if (!ctx || !ctx->ready || !data || len <= 0)
        return -1;
    cinfo = &ctx->cinfo;

    if (setjmp(ctx->jerr.setjmp_buffer)) {
        jpeg_abort_decompress(cinfo);
        goto done;
    }

    jpeg_abort_decompress(cinfo); /* reset state from any previous frame */
    jpeg_mem_src(cinfo, (const unsigned char *)data, (unsigned long)len);
    if (jpeg_read_header(cinfo, TRUE) != JPEG_HEADER_OK)
        goto done;

    cinfo->out_color_space = JCS_YCbCr;
    if (!jpeg_start_decompress(cinfo))
        goto done;

    w = (int)cinfo->output_width;
    h = (int)cinfo->output_height;
    if (w <= 0 || h <= 0 || cinfo->output_components != 3)
        goto done;
    cw = (w + 1) / 2;
    ch = (h + 1) / 2;

    full = malloc((size_t)w * (size_t)h * 3);
    y = malloc((size_t)w * (size_t)h);
    u = malloc((size_t)cw * (size_t)ch);
    v = malloc((size_t)cw * (size_t)ch);
    row = malloc((size_t)w * 3);
    if (!full || !y || !u || !v || !row)
        goto done;

    while (cinfo->output_scanline < cinfo->output_height) {
        jpeg_read_scanlines(cinfo, &row, 1);
        memcpy(full + ((size_t)cinfo->output_scanline - 1) * (size_t)w * 3,
               row, (size_t)w * 3);
    }

    /* Deinterleave Y and 2x2 box-filter Cb/Cr into 4:2:0 planes. */
    for (y0 = 0; y0 < h; y0++) {
        const uint8_t *src = full + (size_t)y0 * (size_t)w * 3;
        memcpy(y + (size_t)y0 * (size_t)w, src, (size_t)w);
    }
    for (y0 = 0; y0 < ch; y0++) {
        for (x = 0; x < cw; x++) {
            int sy, sx, uc = 0, vc = 0, n = 0;
            for (sy = 0; sy < 2; sy++) {
                int yy = y0 * 2 + sy;
                if (yy >= h)
                    continue;
                for (sx = 0; sx < 2; sx++) {
                    int xx = x * 2 + sx;
                    const uint8_t *pix;
                    if (xx >= w)
                        continue;
                    pix = full + ((size_t)yy * (size_t)w + (size_t)xx) * 3;
                    uc += pix[1];
                    vc += pix[2];
                    n++;
                }
            }
            u[(size_t)y0 * (size_t)cw + (size_t)x] = n ? (uint8_t)(uc / n) : 128;
            v[(size_t)y0 * (size_t)cw + (size_t)x] = n ? (uint8_t)(vc / n) : 128;
        }
    }

    if (callback)
        callback(w, h, 8, 1, y, w, u, cw, v, cw);
    jpeg_finish_decompress(cinfo);
    ret = 0;

done:
    free(row);
    free(full);
    free(y);
    free(u);
    free(v);
    return ret;
}
