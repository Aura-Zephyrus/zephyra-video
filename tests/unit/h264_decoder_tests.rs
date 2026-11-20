//! `h264::decoder` 模块的断言：OpenH264 的 FFI 创建 / 拒绝错误 codec / 垃圾码流。

use crate::buffer::DecoderBuffer;
use crate::decoder::VideoDecoder;
use crate::status::DecoderStatusCode;
use crate::{
    bt601_color_space, level_8bit_420, AlphaMode, OpenH264Decoder, Rect, Size, VideoCodec,
    VideoCodecProfile, VideoDecoderConfig, VideoTransformation,
};

fn config(codec: VideoCodec) -> VideoDecoderConfig {
    VideoDecoderConfig::new(
        codec,
        VideoCodecProfile::Unknown,
        level_8bit_420(),
        AlphaMode::IsOpaque,
        bt601_color_space(),
        VideoTransformation::None,
        Size::new(64, 64),
        Rect::new(0, 0, 64, 64),
        Size::new(64, 64),
        Vec::new(),
    )
}

#[test]
fn create_and_drop_an_openh264_context() {
    let mut decoder = OpenH264Decoder::new();
    assert!(decoder.initialize(&config(VideoCodec::H264), false).is_ok());
    assert_eq!(decoder.decoder_type(), crate::VideoDecoderType::OpenH264);
    // H.264 软件解码器吃 Annex-B，需要调用方做 AVCC → Annex-B 转换。
    assert!(decoder.needs_bitstream_conversion());
}

#[test]
fn openh264_rejects_non_h264_configs() {
    let mut decoder = OpenH264Decoder::new();
    assert_eq!(
        decoder.initialize(&config(VideoCodec::VP9), false).code(),
        DecoderStatusCode::UnsupportedCodec
    );
}

#[test]
fn garbage_bitstream_returns_an_error_not_a_panic() {
    let mut decoder = OpenH264Decoder::new();
    assert!(decoder.initialize(&config(VideoCodec::H264), false).is_ok());
    let buffer = DecoderBuffer::new(vec![0xFF, 0xFF, 0xFF], 0, 0);
    assert!(decoder.decode(&buffer).is_err());
}
