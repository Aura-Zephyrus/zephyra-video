/*
 * Thin C bridge for the vendored vvdec (VVC / H.266) decoder.
 *
 * vvdec's C API uses vvdecParams / vvdecAccessUnit / vvdecFrame structs.
 * This shim hides those layouts from Rust and flattens each decoded picture
 * into scalar callback arguments. The vendored sources under vvdec/ are
 * never modified; version headers are generated into OUT_DIR by build.rs.
 */
#include <vvdec/vvdec.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

/* Keep in sync with the extern "C" callback type in src/vvc/mod.rs. */
typedef void (*z_vvc_frame_cb)(int w, int h, int bpc, int layout,
                               const uint8_t *y, ptrdiff_t ys,
                               const uint8_t *u, ptrdiff_t us,
                               const uint8_t *v, ptrdiff_t vs);

extern "C" {

int z_vvc_open(void **out)
{
    vvdecParams params;
    vvdecDecoder *dec;

    if (!out)
        return -1;
    vvdec_params_default(&params);
    params.threads = 1;
    params.logLevel = VVDEC_SILENT;
    dec = vvdec_decoder_open(&params);
    if (!dec)
        return -1;
    *out = dec;
    return 0;
}

void z_vvc_close(void *handle)
{
    if (handle)
        vvdec_decoder_close((vvdecDecoder *)handle);
}

/*
 * Feed one Annex-B access unit and emit every picture it produces.
 * Returns 0 on success (including VVDEC_TRY_AGAIN), negative on hard error.
 */
int z_vvc_decode(void *handle, const uint8_t *data, int len,
                 z_vvc_frame_cb callback)
{
    vvdecDecoder *dec = (vvdecDecoder *)handle;
    vvdecAccessUnit *au;
    vvdecFrame *frame = NULL;
    int ret;

    if (!dec || !data || len <= 0)
        return -1;

    au = vvdec_accessUnit_alloc();
    if (!au)
        return -1;
    vvdec_accessUnit_default(au);
    vvdec_accessUnit_alloc_payload(au, len);
    if (!au->payload || au->payloadSize < len) {
        vvdec_accessUnit_free(au);
        return -1;
    }
    memcpy(au->payload, data, (size_t)len);
    au->payloadUsedSize = len;

    ret = vvdec_decode(dec, au, &frame);
    vvdec_accessUnit_free_payload(au);
    vvdec_accessUnit_free(au);

    if (ret != VVDEC_OK)
        return ret;

    if (frame) {
        if (callback && frame->numPlanes >= 3 &&
            frame->colorFormat == VVDEC_CF_YUV420_PLANAR &&
            frame->width > 0 && frame->height > 0 &&
            frame->planes[0].ptr && frame->planes[1].ptr && frame->planes[2].ptr) {
            const int w = (int)frame->width;
            const int h = (int)frame->height;
            callback(w, h, (int)frame->bitDepth, 1,
                     frame->planes[0].ptr, (ptrdiff_t)frame->planes[0].stride,
                     frame->planes[1].ptr, (ptrdiff_t)frame->planes[1].stride,
                     frame->planes[2].ptr, (ptrdiff_t)frame->planes[2].stride);
        }
        vvdec_frame_unref(dec, frame);
    }
    return 0;
}

} // extern "C"
