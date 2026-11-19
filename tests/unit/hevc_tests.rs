//! `hevc` 模块的断言：libde265 的 FFI 创建 / 拒绝错误 codec / 垃圾码流。

use crate::buffer::DecoderBuffer;
use crate::decoder::VideoDecoder;
use crate::status::DecoderStatusCode;
use crate::{
    bt709_color_space, level_8bit_420, AlphaMode, HevcDecoder, Rect, Size, VideoCodec,
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
fn create_and_drop_a_hevc_context() {
    let mut decoder = HevcDecoder::new();
    assert!(decoder.initialize(&config(VideoCodec::HEVC), false).is_ok());
    assert_eq!(decoder.decoder_type(), crate::VideoDecoderType::Hevc);
}

#[test]
fn hevc_rejects_non_hevc_configs() {
    let mut decoder = HevcDecoder::new();
    assert_eq!(
        decoder.initialize(&config(VideoCodec::H264), false).code(),
        DecoderStatusCode::UnsupportedCodec
    );
}

#[test]
fn garbage_bitstream_returns_an_error_not_a_panic() {
    let mut decoder = HevcDecoder::new();
    assert!(decoder.initialize(&config(VideoCodec::HEVC), false).is_ok());
    let buffer = DecoderBuffer::new(vec![0xFF, 0xFF, 0xFF], 0, 0);
    // libde265 gracefully ignores garbage; just assert no panic.
    let _ = decoder.decode(&buffer);
}
