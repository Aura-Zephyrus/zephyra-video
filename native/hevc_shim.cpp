/*
 * Thin C++ bridge for the vendored libde265 HEVC decoder.
 *
 * libde265's public API is C (de265.h), but the library itself is C++.
 * The shim hides de265_image and flatten decoded planes into scalar
 * callback arguments, so Rust mirrors no structs. Vendored sources are
 * never modified.
 */
#include <libde265/de265.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef void (*z_hevc_frame_cb)(int w, int h, int bpc, int chroma,
                               const uint8_t *y, int ys,
                               const uint8_t *u, int us,
                               const uint8_t *v, int vs);

extern "C" void *z_hevc_open(void)
{
    de265_decoder_context *ctx = de265_new_decoder();
    if (ctx)
        de265_start_worker_threads(ctx, 0);
    return ctx;
}

extern "C" void z_hevc_close(void *handle)
{
    if (handle)
        de265_free_decoder((de265_decoder_context *)handle);
}

extern "C" void z_hevc_reset(void *handle)
{
    if (handle)
        de265_reset((de265_decoder_context *)handle);
}

/*
 * Push one Annex-B buffer (start-code NAL units), drain all pictures.
 * Returns 0 on success, negative on a fatal decoder error.
 */
extern "C" int z_hevc_decode(void *handle, const uint8_t *data, int len,
                             z_hevc_frame_cb callback)
{
    de265_decoder_context *ctx = (de265_decoder_context *)handle;
    if (!ctx || !data || len <= 0)
        return -1;

    if (de265_push_data(ctx, data, len, 0, NULL) != DE265_OK)
        return -1;
    de265_push_end_of_NAL(ctx);
    if (de265_flush_data(ctx) != DE265_OK)
        return -1;

    for (;;) {
        int more = 0;
        de265_error err = de265_decode(ctx, &more);
        if (err != DE265_OK)
            break;

        const de265_image *image = de265_get_next_picture(ctx);
        if (image) {
            if (callback) {
                int chroma = (int)de265_get_chroma_format(image);
                int bpc = de265_get_bits_per_pixel(image, 0);
                int ys = 0, us = 0, vs = 0;
                const uint8_t *y = de265_get_image_plane(image, 0, &ys);
                const uint8_t *u = de265_get_image_plane(image, 1, &us);
                const uint8_t *v = de265_get_image_plane(image, 2, &vs);
                callback(de265_get_image_width(image, 0),
                         de265_get_image_height(image, 0),
                         bpc, chroma,
                         y, ys, u, us, v, vs);
            }
            de265_release_next_picture(ctx);
        }
        if (!more)
            break;
    }
    return 0;
}
