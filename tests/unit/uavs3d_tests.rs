//! `uavs3d` 模块的断言：Annex-B 切分、错误码映射，以及（开 feature 时）FFI 烟测。

use super::{is_nal_type, map_uavs3d_error, split_annex_b_nals};
use crate::status::DecoderStatusCode;

/// AVS3 的 Annex-B 起始码（三字节）。
const SC: &[u8] = &[0x00, 0x00, 0x01];

#[test]
fn nal_types_match_the_c_decoders_branches() {
    // 序列头 / 序列尾 / 图像头 / 切片。
    for byte in [0xB0u8, 0xB1, 0xB3, 0xB6, 0x00, 0x01, 0x8E] {
        assert!(is_nal_type(byte), "{byte:#04x} should be a NAL type");
    }
    // 切片类型只到 0x8E，其余是保留值。
    assert!(!is_nal_type(0x8F));
    assert!(!is_nal_type(0xB2));
    assert!(!is_nal_type(0xFF));
}

#[test]
fn splits_two_nals_and_keeps_their_start_codes() {
    let mut stream = Vec::new();
    stream.extend_from_slice(SC);
    stream.extend_from_slice(&[0xB0, 0x10, 0x20]); // 序列头
    stream.extend_from_slice(SC);
    stream.extend_from_slice(&[0xB3, 0x30, 0x40]); // 图像头

    let nalus = split_annex_b_nals(&stream);
    assert_eq!(nalus.len(), 2);
    assert_eq!(nalus[0], &[0x00, 0x00, 0x01, 0xB0, 0x10, 0x20]);
    assert_eq!(nalus[1], &[0x00, 0x00, 0x01, 0xB3, 0x30, 0x40]);
}

#[test]
fn a_four_byte_start_code_loses_its_leading_zero() {
    // uavs3d 的 `uavs3d_decode()` 读 `bs[3]` 当类型字节，所以每个 NAL 必须
    // 以三字节起始码开头 —— 四字节起始码前面那个 0 要被丢掉。
    let stream = [0x00, 0x00, 0x00, 0x01, 0xB0, 0x7F];
    let nalus = split_annex_b_nals(&stream);
    assert_eq!(nalus.len(), 1);
    assert_eq!(nalus[0], &[0x00, 0x00, 0x01, 0xB0, 0x7F]);
}

#[test]
fn emulation_prevention_inside_a_nal_is_not_a_boundary() {
    // `00 00 03 01` 是 RBSP 里的仿真 Prevention 字节序列，不是起始码。
    let mut stream = Vec::new();
    stream.extend_from_slice(SC);
    stream.extend_from_slice(&[0xB0, 0x00, 0x00, 0x03, 0x01, 0x11]);
    let nalus = split_annex_b_nals(&stream);
    assert_eq!(nalus.len(), 1);
}

#[test]
fn a_stream_without_start_codes_produces_nothing() {
    assert!(split_annex_b_nals(&[0x01, 0xB0, 0x10]).is_empty());
}

#[test]
fn error_codes_carry_their_symbolic_names() {
    let cases = [
        (-1, DecoderStatusCode::OutOfMemory, "ERR_OUT_OF_MEMORY"),
        (-4, DecoderStatusCode::MalformedBitstream, "ERR_UNKNOWN_NAL"),
        (
            -8,
            DecoderStatusCode::UnsupportedConfig,
            "ERR_SEQ_MAININFO_CHANGED",
        ),
        (-100, DecoderStatusCode::Failed, "ERR_UNKNOWN"),
    ];
    for (code, expected, name) in cases {
        let status = map_uavs3d_error(code).expect("negative codes are errors");
        assert_eq!(status.code(), expected);
        assert!(status.message().contains(name));
    }
}

#[test]
fn success_codes_are_not_errors() {
    for code in [0, 1, 2] {
        assert!(map_uavs3d_error(code).is_none());
    }
}

#[cfg(feature = "uavs3d")]
mod with_c {
    use crate::buffer::DecoderBuffer;
    use crate::decoder::VideoDecoder;
    use crate::status::DecoderStatusCode;
    use crate::Uavs3dDecoder;
    use crate::{
        bt709_color_space, level_8bit_420, AlphaMode, Rect, Size, VideoCodec, VideoCodecProfile,
        VideoDecoderConfig, VideoTransformation,
    };

    fn avs3_config() -> VideoDecoderConfig {
        VideoDecoderConfig::new(
            VideoCodec::Av3,
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
    fn create_and_drop_a_decoder() {
        let mut decoder = Uavs3dDecoder::new();
        assert!(decoder.initialize(&avs3_config(), false).is_ok());
        assert_eq!(decoder.decoder_type(), crate::VideoDecoderType::Uavs3d);
        // Drop 会调 uavs3d_delete —— 这一条主要钉住 FFI 的 struct 布局没有错位。
    }

    #[test]
    fn a_decoder_rejects_non_avs3_configs() {
        let config = VideoDecoderConfig::new(
            VideoCodec::H264,
            VideoCodecProfile::H264Baseline,
            level_8bit_420(),
            AlphaMode::IsOpaque,
            bt709_color_space(),
            VideoTransformation::None,
            Size::new(64, 64),
            Rect::new(0, 0, 64, 64),
            Size::new(64, 64),
            Vec::new(),
        );
        let mut decoder = Uavs3dDecoder::new();
        let status = decoder.initialize(&config, false);
        assert_eq!(status.code(), DecoderStatusCode::UnsupportedCodec);
    }

    #[test]
    fn garbage_bitstream_returns_an_error_not_a_panic() {
        let mut decoder = Uavs3dDecoder::new();
        assert!(decoder.initialize(&avs3_config(), false).is_ok());
        // 合法切片 NAL，但没先喂序列头 ⇒ C 侧返回 ERR_SEQ_HEADER_UNREADY。
        // （类型字节 0xFF 会被切分器先丢掉，到不了 C，所以用 0x01。）
        let buffer = DecoderBuffer::new(vec![0x00, 0x00, 0x01, 0x01, 0xFF, 0xFF], 0, 0);
        let result = decoder.decode(&buffer);
        assert!(result.is_err());
    }
}
