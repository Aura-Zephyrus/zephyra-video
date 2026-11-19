//! `av1` 模块的断言：dav1d 的 FFI 创建 / 拒绝错误 codec / 垃圾码流。

use crate::buffer::DecoderBuffer;
use crate::decoder::VideoDecoder;
use crate::status::DecoderStatusCode;
use crate::{
    bt709_color_space, level_8bit_420, AlphaMode, Dav1dDecoder, Rect, Size, VideoCodec,
    VideoCodecProfile, VideoDecoderConfig, VideoTransformation,
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
fn create_and_drop_a_dav1d_context() {
    let mut decoder = Dav1dDecoder::new();
    assert!(decoder.initialize(&config(VideoCodec::AV1), false).is_ok());
    assert_eq!(decoder.decoder_type(), crate::VideoDecoderType::Dav1d);
    // Drop 调 z_dav1d_close。
}

#[test]
fn dav1d_rejects_non_av1_configs() {
    let mut decoder = Dav1dDecoder::new();
    assert_eq!(
        decoder.initialize(&config(VideoCodec::VP9), false).code(),
        DecoderStatusCode::UnsupportedCodec
    );
}

#[test]
fn garbage_obu_returns_an_error_not_a_panic() {
    let mut decoder = Dav1dDecoder::new();
    assert!(decoder.initialize(&config(VideoCodec::AV1), false).is_ok());
    let buffer = DecoderBuffer::new(vec![0xFF, 0xFF, 0xFF], 0, 0);
    assert!(decoder.decode(&buffer).is_err());
}
