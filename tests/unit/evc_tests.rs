//! `evc` 模块的断言：xevd 的 FFI 创建 / 拒绝错误 codec / 垃圾码流。

use crate::buffer::DecoderBuffer;
use crate::decoder::VideoDecoder;
use crate::status::DecoderStatusCode;
use crate::{
    bt709_color_space, level_8bit_420, AlphaMode, Rect, Size, VideoCodec, VideoCodecProfile,
    VideoDecoderConfig, VideoTransformation, XevdDecoder,
};

fn config(codec: VideoCodec) -> VideoDecoderConfig {
    VideoDecoderConfig::new(
        codec,
        VideoCodecProfile::Unknown,
        level_8bit_420(),
        AlphaMode::IsOpaque,
        bt709_color_space(),
        VideoTransformation::None,
        Size::new(64, 64),
        Rect::new(0, 0, 64, 64),
        Size::new(64, 64),
        Vec::new(),
    )
}

#[test]
fn create_and_drop_an_xevd_context() {
    let mut decoder = XevdDecoder::new();
    assert!(decoder.initialize(&config(VideoCodec::Evc), false).is_ok());
    assert_eq!(decoder.decoder_type(), crate::VideoDecoderType::Xevd);
    // Drop 调 z_evc_close。
}

#[test]
fn xevd_rejects_non_evc_configs() {
    let mut decoder = XevdDecoder::new();
    assert_eq!(
        decoder.initialize(&config(VideoCodec::VP9), false).code(),
        DecoderStatusCode::UnsupportedCodec
    );
}

#[test]
fn garbage_bitstream_returns_an_error_not_a_panic() {
    let mut decoder = XevdDecoder::new();
    assert!(decoder.initialize(&config(VideoCodec::Evc), false).is_ok());
    let buffer = DecoderBuffer::new(vec![0xFF, 0xFF, 0xFF], 0, 0);
    assert!(decoder.decode(&buffer).is_err());
}
