/*
 * Thin C bridge for the vendored OpenH264 H.264 decoder (BSD-2-Clause).
 *
 * OpenH264's C API (codec_api.h) exposes ISVCDecoderVtbl with function
 * pointers. This shim hides the vtable and SBufferInfo layout from Rust,
 * flattening each decoded picture into scalar callback arguments.
 * The vendored sources under openh264/ are never modified.
 */
#include <codec_api.h>
#include <stdint.h>
#include <string.h>

/* Keep in sync with the extern "C" callback type in src/h264/decoder.rs. */
typedef void (*z_h264_frame_cb)(int w, int h, int bpc, int layout,
                                const uint8_t *y, ptrdiff_t ys,
                                const uint8_t *u, ptrdiff_t us,
                                const uint8_t *v, ptrdiff_t vs);

int z_h264_open(void **out)
{
    ISVCDecoder *dec = NULL;
    SDecodingParam param;
    long ret;

    if (!out)
        return -1;
    memset(&param, 0, sizeof(param));
    param.sVideoProperty.eVideoBsType = VIDEO_BITSTREAM_DEFAULT;

    ret = WelsCreateDecoder(&dec);
    if (ret != 0 || !dec)
        return -1;
    ret = (*dec)->Initialize(dec, &param);
    if (ret != 0) {
        WelsDestroyDecoder(dec);
        return -1;
    }
    *out = dec;
    return 0;
}

void z_h264_close(void *handle)
{
    ISVCDecoder *dec = handle;
    if (!dec)
        return;
    (*dec)->Uninitialize(dec);
    WelsDestroyDecoder(dec);
}

/*
 * Feed one access unit (Annex-B) and emit at most one picture.
 * Returns 0 on success (including "no picture ready yet"), negative on error.
 */
int z_h264_decode(void *handle, const uint8_t *data, int len,
                  z_h264_frame_cb callback)
{
    ISVCDecoder *dec = handle;
    unsigned char *dst[3] = { NULL, NULL, NULL };
    SBufferInfo info;
    DECODING_STATE state;

    if (!dec || !data || len <= 0)
        return -1;

    memset(&info, 0, sizeof(info));
    state = (*dec)->DecodeFrameNoDelay(dec, data, len, dst, &info);
    if (state != dsErrorFree)
        return -1;

    if (info.iBufferStatus == 1 && dst[0] && dst[1] && dst[2] &&
        info.UsrData.sSystemBuffer.iFormat == videoFormatI420 &&
        info.UsrData.sSystemBuffer.iWidth > 0 &&
        info.UsrData.sSystemBuffer.iHeight > 0 && callback) {
        const int w = info.UsrData.sSystemBuffer.iWidth;
        const int h = info.UsrData.sSystemBuffer.iHeight;
        const int ys = info.UsrData.sSystemBuffer.iStride[0];
        const int us = info.UsrData.sSystemBuffer.iStride[1];
        /* 8-bit 4:2:0 -> I420 (layout 1). */
        callback(w, h, 8, 1,
                 dst[0], (ptrdiff_t)ys,
                 dst[1], (ptrdiff_t)us,
                 dst[2], (ptrdiff_t)us);
    }
    return 0;
}
