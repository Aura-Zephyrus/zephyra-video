//! `codec.rs` 的断言：编解码器 / profile / 色度采样 / level 的词汇表。

use super::super::{VideoChromaSampling, VideoCodec, VideoCodecLevel, VideoCodecProfile};

// codecs.rs 是后端接线（模块 / re-export / load_all）的唯一声明处；
// 漏加 Cargo feature 的症状原本是「silent no-op」，这条测试把它变成硬失败。
#[test]
fn codec_registry_features_are_declared_in_cargo_toml() {
    const MANIFEST: &str = include_str!("../../Cargo.toml");
    assert_eq!(
        super::super::CODEC_MANIFEST.len(),
        14,
        "codecs.rs 的声明条目数变化时需同步更新本断言"
    );
    for &(feature, module, decoder) in super::super::CODEC_MANIFEST {
        assert!(
            MANIFEST.lines().any(|line| line.trim_start().starts_with(&format!("{feature} ="))),
            "codec {module}/{decoder} 在 codecs.rs 里声明了 feature '{feature}'，但 Cargo.toml 没有它"
        );
    }
}

#[test]
fn h264_is_the_only_h264_family_codec() {
    assert!(VideoCodec::H264.is_h264_family());
    assert!(!VideoCodec::VP8.is_h264_family());
    assert!(!VideoCodec::HEVC.is_h264_family());
    assert!(!VideoCodec::Unknown.is_h264_family());
}

#[test]
fn codec_names_are_stable() {
    assert_eq!(VideoCodec::H264.as_str(), "H264");
    assert_eq!(VideoCodec::VP9.as_str(), "VP9");
    assert_eq!(VideoCodec::Unknown.as_str(), "unknown");
}

#[test]
fn profile_maps_back_to_its_codec() {
    assert_eq!(VideoCodecProfile::H264Baseline.codec(), VideoCodec::H264);
    assert_eq!(VideoCodecProfile::H264High.codec(), VideoCodec::H264);
    assert_eq!(VideoCodecProfile::VP9Profile0.codec(), VideoCodec::VP9);
    assert_eq!(VideoCodecProfile::AV1ProfileMain.codec(), VideoCodec::AV1);
    assert_eq!(VideoCodecProfile::HEVCMain10.codec(), VideoCodec::HEVC);
    assert_eq!(VideoCodecProfile::Unknown.codec(), VideoCodec::Unknown);
}

#[test]
fn profile_names_match_their_enum_spelling() {
    assert_eq!(
        VideoCodecProfile::H264High444Predictive.as_str(),
        "H264High444Predictive"
    );
    assert_eq!(
        VideoCodecProfile::DolbyVisionProfile5.as_str(),
        "DolbyVisionProfile5"
    );
    assert_eq!(VideoCodecProfile::Unknown.as_str(), "unknown");
}

#[test]
fn level_defaults_to_8bit_420() {
    let level = VideoCodecLevel::default();
    assert_eq!(level.level, 0);
    assert_eq!(level.bit_depth, 8);
    assert_eq!(level.chroma_sampling, VideoChromaSampling::K420);
}

#[test]
fn level_constructor_keeps_all_fields() {
    let level = VideoCodecLevel::new(31, 10, VideoChromaSampling::K422);
    assert_eq!(level.level, 31);
    assert_eq!(level.bit_depth, 10);
    assert_eq!(level.chroma_sampling, VideoChromaSampling::K422);
}

#[test]
fn chroma_sampling_values_match_chromium() {
    // 这些值会进日志与 UMA，改值等于改历史。
    assert_eq!(VideoChromaSampling::Unknown as u8, 0);
    assert_eq!(VideoChromaSampling::K420 as u8, 1);
    assert_eq!(VideoChromaSampling::K422 as u8, 2);
    assert_eq!(VideoChromaSampling::K444 as u8, 3);
    assert_eq!(VideoChromaSampling::K400 as u8, 4);
}
