/*
 * Thin C bridge for the vendored xevd (MPEG-5 EVC) decoder.
 *
 * Upstream exposes XEVD_BITB / XEVD_IMGB structs with ~30 fields. Rather than
 * mirror those layouts in Rust, this shim flattens each decoded picture into
 * scalar arguments passed to a Rust callback. The vendored sources under
 * xevd/ are never modified.
 */
#include <xevd.h>
#include <stdint.h>
#include <string.h>

/* Keep in sync with the extern "C" callback type in src/evc/mod.rs. */
typedef void (*z_evc_frame_cb)(int w, int h, int bpc, int layout,
                               const uint8_t *y, ptrdiff_t ys,
                               const uint8_t *u, ptrdiff_t us,
                               const uint8_t *v, ptrdiff_t vs);

int z_evc_open(void **out, int threads)
{
    XEVD_CDSC cdsc;
    int err = 0;
    XEVD id;

    if (!out)
        return -1;
    memset(&cdsc, 0, sizeof(cdsc));
    cdsc.threads = threads;
    id = xevd_create(&cdsc, &err);
    if (!id || err < 0)
        return err < 0 ? err : -1;
    *out = id;
    return 0;
}

void z_evc_close(void *handle)
{
    if (handle)
        xevd_delete((XEVD)handle);
}

/*
 * Feed one access unit and emit every picture it produces.
 * Returns 0 on success (even when no picture was ready), negative on error.
 */
int z_evc_decode(void *handle, const uint8_t *data, int len,
                 z_evc_frame_cb callback)
{
    XEVD id = (XEVD)handle;
    XEVD_BITB bitb;
    XEVD_STAT stat;
    XEVD_IMGB *img = NULL;
    int ret;

    if (!id || !data || len <= 0)
        return -1;

    memset(&bitb, 0, sizeof(bitb));
    memset(&stat, 0, sizeof(stat));
    /* The bytes belong to the Rust caller and stay alive for this call. */
    bitb.addr = (void *)data;
    bitb.bsize = len;
    bitb.ssize = len;

    ret = xevd_decode(id, &bitb, &stat);
    if (ret < 0)
        return ret;

    for (;;) {
        ret = xevd_pull(id, &img);
        if (ret < 0 || !img)
            break;
        if (callback && img->np >= 3 && img->a[0] && img->a[1] && img->a[2]) {
            /* Both XEVD_CS_YCBCR420 (8-bit) and XEVD_CS_YCBCR420_10LE are
             * accepted; layout 1 means planar 4:2:0 to the Rust side. */
            int bpc = XEVD_CS_GET_BIT_DEPTH(img->cs);
            int layout = (XEVD_CS_GET_FORMAT(img->cs) == XEVD_CF_YCBCR420) ? 1 : 0;
            callback(img->w[0], img->h[0], bpc, layout,
                     (const uint8_t *)img->a[0], (ptrdiff_t)img->s[0],
                     (const uint8_t *)img->a[1], (ptrdiff_t)img->s[1],
                     (const uint8_t *)img->a[2], (ptrdiff_t)img->s[2]);
        }
        if (img->release)
            img->release(img);
        img = NULL;
    }
    return 0;
}
