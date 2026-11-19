/*
 * Thin C bridge for the vendored dav1d decoder.
 *
 * The upstream API uses structs with many fields (Dav1dPicture, Dav1dData).
 * Rather than mirror those layouts in Rust, this shim flattens the output
 * into scalar arguments passed to a Rust callback. The vendored sources
 * under dav1d/ are never modified.
 */
#include <dav1d/dav1d.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* dav1d_data_wrap validates that the free callback is non-NULL. The bytes
 * belong to the Rust caller, so releasing them is a no-op. */
static void z_no_op_free(const uint8_t *buf, void *cookie)
{
    (void)buf;
    (void)cookie;
}

typedef void (*z_dav1d_frame_cb)(int w, int h, int bpc, int layout,
                                const uint8_t *y, ptrdiff_t ys,
                                const uint8_t *u, ptrdiff_t us,
                                const uint8_t *v, ptrdiff_t vs);

int z_dav1d_open(void **out, int threads)
{
    Dav1dSettings settings;
    Dav1dContext *ctx = NULL;
    int ret;

    dav1d_default_settings(&settings);
    settings.n_threads = threads;
    settings.max_frame_delay = 1;   /* low latency: no reordering delay */
    settings.all_layers = 0;
    ret = dav1d_open(&ctx, &settings);
    if (ret < 0)
        return ret;
    *out = ctx;
    return 0;
}

void z_dav1d_close(void *handle)
{
    Dav1dContext *ctx = handle;
    if (ctx)
        dav1d_close(&ctx);
}

/*
 * Feed one buffer of OBUs and emit every picture it produces.
 * Returns 0 on success (even when no picture was ready), negative on error.
 */
int z_dav1d_decode(void *handle, const uint8_t *data, int len,
                   z_dav1d_frame_cb callback)
{
    Dav1dContext *ctx = handle;
    Dav1dData input;
    Dav1dPicture picture;
    int ret;

    if (!ctx || !data || len <= 0)
        return -1;

    memset(&input, 0, sizeof(input));
    memset(&picture, 0, sizeof(picture));

    /* The bytes belong to the Rust caller and stay alive for this call;
     * dav1d still requires a non-NULL free callback. */
    ret = dav1d_data_wrap(&input, data, (size_t)len, z_no_op_free, NULL);
    if (ret < 0)
        return ret;

    ret = dav1d_send_data(ctx, &input);
    if (ret < 0 && ret != DAV1D_ERR(EAGAIN)) {
        dav1d_data_unref(&input);
        return ret;
    }

    for (;;) {
        ret = dav1d_get_picture(ctx, &picture);
        if (ret < 0)
            break;
        if (callback) {
            callback(picture.p.w, picture.p.h, picture.p.bpc, picture.p.layout,
                     picture.data[0], picture.stride[0],
                     picture.data[1], picture.stride[1],
                     picture.data[2], picture.stride[1]);
        }
        dav1d_picture_unref(&picture);
        memset(&picture, 0, sizeof(picture));
    }
    dav1d_data_unref(&input);

    if (ret < 0 && ret != DAV1D_ERR(EAGAIN))
        return ret;
    return 0;
}
