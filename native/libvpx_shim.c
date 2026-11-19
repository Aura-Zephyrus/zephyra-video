/*
 * Thin C bridge for the vendored libvpx VP8/VP9 decoders.
 *
 * vpx_codec_ctx_t and vpx_image_t are large/versioned structs; the shim
 * flattens each decoded image into scalar callback arguments so Rust never
 * has to mirror those layouts. The vendored sources under libvpx/ are
 * never modified.
 */
#include <vpx/vp8dx.h>
#include <vpx/vpx_decoder.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef void (*z_vpx_frame_cb)(int w, int h, int bitdepth, int fmt,
                              const uint8_t *y, int ys,
                              const uint8_t *u, int us,
                              const uint8_t *v, int vs);

static int z_vpx_open_with(void **out, vpx_codec_iface_t *(*iface)(void))
{
    vpx_codec_ctx_t *ctx = calloc(1, sizeof(vpx_codec_ctx_t));
    vpx_codec_dec_cfg_t cfg;
    if (!ctx)
        return -1;
    memset(&cfg, 0, sizeof(cfg));
    cfg.threads = 1;
    if (vpx_codec_dec_init(ctx, iface(), &cfg, 0) != VPX_CODEC_OK) {
        free(ctx);
        return -1;
    }
    *out = ctx;
    return 0;
}

int z_vpx9_open(void **out) { return z_vpx_open_with(out, vpx_codec_vp9_dx); }
int z_vpx8_open(void **out) { return z_vpx_open_with(out, vpx_codec_vp8_dx); }

void z_vpx_close(void *handle)
{
    vpx_codec_ctx_t *ctx = handle;
    if (!ctx)
        return;
    vpx_codec_destroy(ctx);
    free(ctx);
}

int z_vpx_decode(void *handle, const uint8_t *data, int len,
                 z_vpx_frame_cb callback)
{
    vpx_codec_ctx_t *ctx = handle;
    vpx_codec_iter_t iter = NULL;
    const vpx_image_t *image;

    if (!ctx || !data || len <= 0)
        return -1;
    if (vpx_codec_decode(ctx, data, (unsigned int)len, NULL, 1) != VPX_CODEC_OK)
        return -1;

    while ((image = vpx_codec_get_frame(ctx, &iter)) != NULL) {
        if (callback) {
            callback((int)image->d_w, (int)image->d_h, (int)image->bit_depth,
                     (int)image->fmt,
                     image->planes[0], (int)image->stride[0],
                     image->planes[1], (int)image->stride[1],
                     image->planes[2], (int)image->stride[2]);
        }
    }
    return 0;
}
