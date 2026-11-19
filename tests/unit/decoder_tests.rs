//! `decoder.rs` 的断言：解码器接口的默认行为。
//!
//! 用一个「假解码器」验证 trait 的默认方法 —— 这些默认值是**契约**：
//! 后端不覆盖它们就意味着「我不需要特殊处理」。

use super::super::{
    DecoderBuffer, DecoderStatus, DecoderStatusCode, Rect, Size, VideoCodec, VideoCodecProfile,
    VideoDecoder, VideoDecoderConfig, VideoDecoderType, VideoFrame,
};

/// 记录收到的调用、最小可用实现。
struct FakeDecoder {
    platform: bool,
    init_result: DecoderStatus,
    init_calls: usize,
    decode_calls: usize,
    reset_calls: usize,
}

impl FakeDecoder {
    fn new(platform: bool) -> Self {
        Self {
            platform,
            init_result: DecoderStatus::new(DecoderStatusCode::Ok),
            init_calls: 0,
            decode_calls: 0,
            reset_calls: 0,
        }
    }

    fn failing(code: DecoderStatusCode) -> Self {
        let mut decoder = Self::new(false);
        decoder.init_result = DecoderStatus::with_message(code, "nope");
        decoder
    }
}

impl VideoDecoder for FakeDecoder {
    fn initialize(&mut self, _config: &VideoDecoderConfig, _low_delay: bool) -> DecoderStatus {
        self.init_calls += 1;
        self.init_result.clone()
    }

    fn decode(&mut self, _buffer: &DecoderBuffer) -> Result<Vec<VideoFrame>, DecoderStatus> {
        self.decode_calls += 1;
        Ok(Vec::new())
    }

    fn reset(&mut self) {
        self.reset_calls += 1;
    }

    fn decoder_type(&self) -> VideoDecoderType {
        VideoDecoderType::Software
    }

    fn is_platform_decoder(&self) -> bool {
        self.platform
    }
}

fn config() -> VideoDecoderConfig {
    VideoDecoderConfig::new(
        VideoCodec::H264,
        VideoCodecProfile::H264Baseline,
        super::super::level_8bit_420(),
        super::super::AlphaMode::IsOpaque,
        super::super::bt709_color_space(),
        super::super::VideoTransformation::None,
        Size::new(640, 360),
        Rect::new(0, 0, 640, 360),
        Size::new(640, 360),
        Vec::new(),
    )
}

#[test]
fn default_capabilities_are_conservative() {
    let decoder = FakeDecoder::new(false);
    // 软件解码器不需要位流转换（它吃的就是喂进来的格式）。
    assert!(!decoder.needs_bitstream_conversion());
    assert!(decoder.can_read_without_stalling());
    // 一次只解一个缓冲。
    assert_eq!(decoder.max_decode_requests(), 1);
    // 输出帧不持有外部资源。
    assert!(!decoder.frames_hold_external_resources());
    assert!(!decoder.is_platform_decoder());
}

#[test]
fn platform_decoder_reports_itself() {
    let decoder = FakeDecoder::new(true);
    assert!(decoder.is_platform_decoder());
}

#[test]
fn initialize_returns_the_status_verbatim() {
    let mut decoder = FakeDecoder::new(false);
    let status = decoder.initialize(&config(), false);
    assert!(status.is_ok());
    assert_eq!(decoder.init_calls, 1);

    let mut failing = FakeDecoder::failing(DecoderStatusCode::UnsupportedProfile);
    let status = failing.initialize(&config(), false);
    assert_eq!(status.code(), DecoderStatusCode::UnsupportedProfile);
    assert_eq!(status.message(), "nope");
}

#[test]
fn decode_and_reset_reach_the_implementation() {
    let mut decoder = FakeDecoder::new(false);
    let buffer = DecoderBuffer::new(vec![1, 2, 3], 0, 0);
    let frames = decoder.decode(&buffer).unwrap();
    assert!(frames.is_empty());
    assert_eq!(decoder.decode_calls, 1);

    decoder.reset();
    assert_eq!(decoder.reset_calls, 1);
}

#[test]
fn decoder_type_names_are_stable() {
    assert_eq!(VideoDecoderType::Unknown.as_str(), "unknown");
    assert_eq!(VideoDecoderType::Software.as_str(), "software");
    assert_eq!(VideoDecoderType::D3D11.as_str(), "d3d11");
    assert_eq!(VideoDecoderType::D3D12.as_str(), "d3d12");
    assert_eq!(VideoDecoderType::Ffmpeg.as_str(), "ffmpeg");
}
