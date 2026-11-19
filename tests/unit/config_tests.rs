//! `config.rs` 的断言：解码配置。

use super::super::{
    bt601_color_space, level_8bit_420, AlphaMode, BitstreamFormat, Rect, Size, VideoAspectRatio,
    VideoCodec, VideoCodecProfile, VideoDecoderConfig, VideoPixelFormat, VideoTransformation,
};

fn base_config() -> VideoDecoderConfig {
    VideoDecoderConfig::new(
        VideoCodec::H264,
        VideoCodecProfile::H264Baseline,
        level_8bit_420(),
        AlphaMode::IsOpaque,
        bt601_color_space(),
        VideoTransformation::None,
        Size::new(640, 360),
        Rect::new(0, 0, 640, 360),
        Size::new(640, 360),
        Vec::new(),
    )
}

#[test]
fn getters_return_what_was_passed_in() {
    let config = base_config();
    assert_eq!(config.codec(), VideoCodec::H264);
    assert_eq!(config.profile(), VideoCodecProfile::H264Baseline);
    assert_eq!(config.alpha_mode(), AlphaMode::IsOpaque);
    assert_eq!(config.transformation(), VideoTransformation::None);
    assert_eq!(config.coded_size(), Size::new(640, 360));
    assert_eq!(config.visible_rect(), Rect::new(0, 0, 640, 360));
    assert_eq!(config.natural_size(), Size::new(640, 360));
    assert!(config.extra_data().is_empty());
    assert!(!config.is_encrypted());
}

#[test]
fn empty_extra_data_means_annex_b() {
    let config = base_config();
    assert_eq!(config.bitstream_format(), BitstreamFormat::AnnexB);
}

#[test]
fn non_empty_extra_data_means_avcc() {
    // Chromium 的约定：**有 extra_data 就是 AVCC**（H.264）。
    let mut config = base_config();
    config.set_extra_data(vec![1, 0x42, 0xE0, 0x1F]);
    assert_eq!(config.bitstream_format(), BitstreamFormat::Avcc);
}

#[test]
fn non_h264_codecs_have_unknown_bitstream_format() {
    let mut vp8 = VideoDecoderConfig::new(
        VideoCodec::VP8,
        VideoCodecProfile::VP8Any,
        level_8bit_420(),
        AlphaMode::IsOpaque,
        bt601_color_space(),
        VideoTransformation::None,
        Size::new(640, 360),
        Rect::new(0, 0, 640, 360),
        Size::new(640, 360),
        Vec::new(),
    ); // VP8 之类没有「AVCC / Annex-B」之分。
    assert_eq!(vp8.bitstream_format(), BitstreamFormat::Unknown);
    vp8.set_extra_data(vec![1, 2, 3]);
    assert_eq!(vp8.bitstream_format(), BitstreamFormat::Unknown);
}

#[test]
fn valid_config_needs_known_codec_and_non_empty_sizes() {
    assert!(base_config().is_valid_config());

    let mut unknown_codec = VideoDecoderConfig::new(
        VideoCodec::Unknown,
        VideoCodecProfile::Unknown,
        level_8bit_420(),
        AlphaMode::IsOpaque,
        bt601_color_space(),
        VideoTransformation::None,
        Size::new(640, 360),
        Rect::new(0, 0, 640, 360),
        Size::new(640, 360),
        Vec::new(),
    );
    assert!(!unknown_codec.is_valid_config());
    unknown_codec.set_profile(VideoCodecProfile::H264Baseline);
    assert!(!unknown_codec.is_valid_config());

    let mut zero_coded = base_config();
    zero_coded.set_coded_size(Size::new(0, 360));
    assert!(!zero_coded.is_valid_config());

    let mut zero_natural = base_config();
    zero_natural.set_natural_size(Size::new(640, 0));
    assert!(!zero_natural.is_valid_config());
}

#[test]
fn visible_rect_must_stay_inside_the_coded_area() {
    let mut config = base_config();
    config.set_visible_rect(Rect::new(0, 0, 641, 360));
    assert!(!config.is_valid_config());

    let mut offset = base_config();
    offset.set_visible_rect(Rect::new(1, 0, 640, 360));
    assert!(!offset.is_valid_config());
}

#[test]
fn matches_compares_extra_data_by_content() {
    let mut a = base_config();
    let mut b = base_config();
    assert!(a.matches(&b));

    b.set_extra_data(vec![1, 2, 3]);
    assert!(!a.matches(&b));

    a.set_extra_data(vec![1, 2, 3]);
    assert!(a.matches(&b));

    b.set_visible_rect(Rect::new(0, 0, 320, 180));
    assert!(!a.matches(&b));
}

#[test]
fn matches_ignores_nothing_else() {
    let a = base_config();
    let mut b = base_config();
    b.set_is_encrypted(true);
    assert!(!a.matches(&b));
    b.set_is_encrypted(false);
    assert!(a.matches(&b));
}

#[test]
fn aspect_ratio_expands_the_display_size() {
    let mut config = base_config();
    config.set_visible_rect(Rect::new(0, 0, 640, 360));
    config.set_aspect_ratio(VideoAspectRatio::new(16, 9));
    // 640×360 已经是 16:9 ⇒ 不变。
    assert_eq!(config.display_size(), Size::new(640, 360));

    config.set_aspect_ratio(VideoAspectRatio::new(4, 3));
    // 4:3 比 16:9 更「窄」⇒ 像素宽高比 (16:9) 更宽，于是按 DAR **加高**
    // （Chromium `VideoAspectRatio::GetNaturalSize()` 的 DAR 分支）。
    assert_eq!(config.display_size(), Size::new(640, 480));
}

#[test]
fn invalid_aspect_ratio_falls_back_to_natural_size() {
    let mut config = base_config();
    config.set_aspect_ratio(VideoAspectRatio::new(0, 9));
    assert!(!config.aspect_ratio().is_valid());
    assert_eq!(config.display_size(), Size::new(640, 360));
}

#[test]
fn human_readable_string_mentions_the_essentials() {
    let text = base_config().as_human_readable_string();
    assert!(text.contains("codec=H264"), "{text}");
    assert!(text.contains("profile=H264Baseline"), "{text}");
    assert!(text.contains("coded=640x360"), "{text}");
    assert!(text.contains("extra_data=0B"), "{text}");
}

#[test]
fn bt601_helper_is_bt709_like_by_default() {
    // 便捷构造里两个都是 BT.709 的矩阵系数，只是命名不同 ——
    // 断言它们**确实**是同一个色彩空间，免得有人以为 bt601 真的改了矩阵。
    let a = bt601_color_space();
    let b = super::super::bt709_color_space();
    assert_eq!(a.matrix_coefficients, b.matrix_coefficients);
    assert!(!a.full_range);
}

#[test]
fn pixel_format_is_reachable_from_config_dependencies() {
    // 帧的像素格式与配置是两条线，但它们共用 `Size` / `Rect` 词汇。
    assert_eq!(VideoPixelFormat::NV12.num_planes(), 2);
}
