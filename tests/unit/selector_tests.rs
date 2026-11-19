//! `selector.rs` 的断言：Chromium 的「360 像素分界」与候选者排序。

use super::super::{
    level_8bit_420, select_decoder_priority, AlphaMode, DecoderPriority, DecoderSelector,
    DecoderStatus, DecoderStatusCode, Rect, Size, VideoCodec, VideoCodecProfile, VideoDecoder,
    VideoDecoderConfig, VideoDecoderType, VideoFrame, VideoTransformation,
};

fn config_with_height(height: u32) -> VideoDecoderConfig {
    VideoDecoderConfig::new(
        VideoCodec::H264,
        VideoCodecProfile::H264Baseline,
        level_8bit_420(),
        AlphaMode::IsOpaque,
        super::super::bt709_color_space(),
        VideoTransformation::None,
        Size::new(640, height),
        Rect::new(0, 0, 640, height),
        Size::new(640, height),
        Vec::new(),
    )
}

struct FakeDecoder {
    platform: bool,
    init_code: DecoderStatusCode,
    type_name: VideoDecoderType,
}

impl FakeDecoder {
    fn new(platform: bool, init_code: DecoderStatusCode) -> Self {
        Self {
            platform,
            init_code,
            type_name: VideoDecoderType::Software,
        }
    }

    fn hardware(init_code: DecoderStatusCode) -> Self {
        let mut decoder = Self::new(true, init_code);
        decoder.type_name = VideoDecoderType::D3D11;
        decoder
    }
}

impl VideoDecoder for FakeDecoder {
    fn initialize(&mut self, _config: &VideoDecoderConfig, _low_delay: bool) -> DecoderStatus {
        DecoderStatus::new(self.init_code)
    }

    fn decode(
        &mut self,
        _buffer: &super::super::DecoderBuffer,
    ) -> Result<Vec<VideoFrame>, DecoderStatus> {
        Ok(Vec::new())
    }

    fn reset(&mut self) {}

    fn decoder_type(&self) -> VideoDecoderType {
        self.type_name
    }

    fn is_platform_decoder(&self) -> bool {
        self.platform
    }
}

#[test]
fn large_visible_area_prefers_the_platform_decoder() {
    let config = config_with_height(360);
    assert_eq!(
        select_decoder_priority(&config, true),
        DecoderPriority::Normal
    );
    assert_eq!(
        select_decoder_priority(&config, false),
        DecoderPriority::Deprioritized
    );
}

#[test]
fn small_visible_area_prefers_the_software_decoder() {
    let config = config_with_height(359);
    assert_eq!(
        select_decoder_priority(&config, false),
        DecoderPriority::Normal
    );
    assert_eq!(
        select_decoder_priority(&config, true),
        DecoderPriority::Deprioritized
    );
}

#[test]
fn deprioritized_candidates_are_kept_not_dropped() {
    let config = config_with_height(360);
    let mut selector = DecoderSelector::new(vec![
        Box::new(FakeDecoder::new(false, DecoderStatusCode::Ok)),
        Box::new(FakeDecoder::hardware(DecoderStatusCode::Ok)),
    ]);
    assert_eq!(selector.len(), 2);
    selector.filter_and_sort(&config);
    // 排序**不删**候选者。
    assert_eq!(selector.len(), 2);
}

#[test]
fn select_returns_the_first_candidate_that_initializes() {
    let config = config_with_height(360);
    let mut selector = DecoderSelector::new(vec![
        Box::new(FakeDecoder::new(false, DecoderStatusCode::Ok)),
        Box::new(FakeDecoder::hardware(DecoderStatusCode::Ok)),
    ]);
    selector.filter_and_sort(&config);
    let decoder = selector.select(&config, false).unwrap();
    // 360 像素及以上 ⇒ 平台解码器优先。
    assert!(decoder.is_platform_decoder());
    assert_eq!(decoder.decoder_type(), VideoDecoderType::D3D11);
    assert!(selector.failures().is_empty());
}

#[test]
fn select_falls_back_to_the_next_candidate() {
    let config = config_with_height(360);
    let mut selector = DecoderSelector::new(vec![
        Box::new(FakeDecoder::hardware(
            DecoderStatusCode::FailedToCreateDecoder,
        )),
        Box::new(FakeDecoder::new(false, DecoderStatusCode::Ok)),
    ]);
    selector.filter_and_sort(&config);
    let decoder = selector.select(&config, false).unwrap();
    assert!(!decoder.is_platform_decoder());
    assert_eq!(selector.failures().len(), 1);
    assert_eq!(
        selector.failures()[0].code(),
        DecoderStatusCode::FailedToCreateDecoder
    );
}

#[test]
fn all_candidates_failing_reports_the_last_failure() {
    let config = config_with_height(360);
    let mut selector = DecoderSelector::new(vec![
        Box::new(FakeDecoder::hardware(
            DecoderStatusCode::FailedToCreateDecoder,
        )),
        Box::new(FakeDecoder::new(
            false,
            DecoderStatusCode::UnsupportedProfile,
        )),
    ]);
    selector.filter_and_sort(&config);
    let error = match selector.select(&config, false) {
        Ok(_) => panic!("expected every candidate to fail"),
        Err(status) => status,
    };
    assert_eq!(error.code(), DecoderStatusCode::UnsupportedProfile);
    assert_eq!(selector.failures().len(), 2);
}

#[test]
fn empty_candidate_list_reports_unsupported_config() {
    let config = config_with_height(360);
    let mut selector = DecoderSelector::new(Vec::new());
    assert!(selector.is_empty());
    let error = match selector.select(&config, false) {
        Ok(_) => panic!("expected an empty candidate list to fail"),
        Err(status) => status,
    };
    assert_eq!(error.code(), DecoderStatusCode::UnsupportedConfig);
}
