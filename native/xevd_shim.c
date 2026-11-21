/*
 * Thin C bridge for the vendored xevd (MPEG-5 EVC) decoder.
 *
 * Upstream exposes XEVD_BITB / XEVD_IMGB structs with ~30 fields. Rather than
 * mirror those layouts in Rust, this shim flattens each decoded picture into
 * scalar arguments passed to a Rust callback. xevd's xevd_decode() consumes
 * **one NAL at a time** (no start code), so the Annex-B input is split here.
 * The vendored sources under xevd/ are never modified.
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

static void pull_frames(XEVD id, z_evc_frame_cb callback)
{
    XEVD_IMGB *img = NULL;
    int ret;

    for (;;) {
        ret = xevd_pull(id, &img);
        if (ret < 0 || !img)
            break;
        if (callback && img->np >= 3 && img->a[0] && img->a[1] && img->a[2]) {
            /* Both XEVD_CS_YCBCR420 (8-bit) and XEVD_CS_YCBCR420_10LE are
             * accepted; layout 1 means planar 4:2:0 to the Rust side. */
            int bpc = XEVD_CS_GET_BIT_DEPTH(img->cs);
            int fmt = XEVD_CS_GET_FORMAT(img->cs);
            /* 10-bit is encoded in the bit-depth bits of cs, not a new format. */
            int layout = (fmt == XEVD_CF_YCBCR420) ? 1 : 0;
            callback(img->w[0], img->h[0], bpc, layout,
                     (const uint8_t *)img->a[0], (ptrdiff_t)img->s[0],
                     (const uint8_t *)img->a[1], (ptrdiff_t)img->s[1],
                     (const uint8_t *)img->a[2], (ptrdiff_t)img->s[2]);
        }
        if (img->release)
            img->release(img);
        img = NULL;
    }
}

static int decode_one_nal(XEVD id, const uint8_t *nal, int len,
                          z_evc_frame_cb callback)
{
    XEVD_BITB bitb;
    XEVD_STAT stat;
    int ret;

    if (len <= 0)
        return 0;
    memset(&bitb, 0, sizeof(bitb));
    memset(&stat, 0, sizeof(stat));
    /* NAL bytes belong to the Rust caller and stay alive for this call. */
    bitb.addr = (void *)nal;
    bitb.bsize = len;
    bitb.ssize = len;
    ret = xevd_decode(id, &bitb, &stat);
    if (ret < 0)
        return ret;
    pull_frames(id, callback);
    return 0;
}

/*
 * Feed one Annex-B access unit and emit every picture it produces.
 * Returns 0 on success, negative on a hard decoder error.
 */
int z_evc_decode(void *handle, const uint8_t *data, int len,
                 z_evc_frame_cb callback)
{
    XEVD id = (XEVD)handle;
    int i = 0;
    int nal_start = -1;
    int found = 0;
    int ret;

    if (!id || !data || len <= 0)
        return -1;

    while (i + 2 < len) {
        if (data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1) {
            if (nal_start >= 0) {
                int end = i;
                /* 4-byte start code: the extra zero belongs to the prefix. */
                if (end > nal_start && data[end - 1] == 0)
                    end--;
                while (end > nal_start && data[end - 1] == 0)
                    end--;
                ret = decode_one_nal(id, data + nal_start, end - nal_start,
                                     callback);
                if (ret < 0)
                    return ret;
                found++;
            }
            nal_start = i + 3;
            i += 3;
        } else {
            i++;
        }
    }
    if (nal_start >= 0 && nal_start < len) {
        int end = len;
        while (end > nal_start && data[end - 1] == 0)
            end--;
        ret = decode_one_nal(id, data + nal_start, end - nal_start, callback);
        if (ret < 0)
            return ret;
        found++;
    }
    /* 没有任何 Annex-B 起始码 ⇒ 不是合法 EVC 码流。 */
    return found > 0 ? 0 : -1;
}
