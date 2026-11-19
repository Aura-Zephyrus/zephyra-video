//! `h264/converter.rs` 的断言：AVCC ↔ Annex-B。

use super::super::{
    annex_b_to_avcc, extract_parameter_sets, is_access_unit_boundary_nal,
    AvcDecoderConfigurationRecord, H264ToAnnexBConverter, NalUnitType,
};

fn record() -> AvcDecoderConfigurationRecord {
    AvcDecoderConfigurationRecord::parse(&[
        0x01, 0x42, 0xE0, 0x1F, 0xFF, 0xE1, 0x00, 0x04, 0x67, 0x42, 0xE0, 0x1F, 0x01, 0x00, 0x03,
        0x68, 0xCE, 0x3C,
    ])
    .unwrap()
}

/// 一个 AVCC 包：两个 4 字节长度前缀的 NAL。
fn avcc_packet() -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend_from_slice(&3u32.to_be_bytes());
    packet.extend_from_slice(&[0x65, 0x88, 0x84]);
    packet.extend_from_slice(&2u32.to_be_bytes());
    packet.extend_from_slice(&[0x41, 0x9A]);
    packet
}

#[test]
fn access_unit_boundary_nals_match_chromium() {
    assert!(is_access_unit_boundary_nal(NalUnitType::Sps));
    assert!(is_access_unit_boundary_nal(NalUnitType::Pps));
    assert!(is_access_unit_boundary_nal(NalUnitType::SeiMessage));
    assert!(is_access_unit_boundary_nal(NalUnitType::Aud));
    assert!(is_access_unit_boundary_nal(NalUnitType::Prefix));
    assert!(is_access_unit_boundary_nal(NalUnitType::SubsetSps));
    assert!(is_access_unit_boundary_nal(NalUnitType::Dps));
    assert!(!is_access_unit_boundary_nal(NalUnitType::IdrSlice));
    assert!(!is_access_unit_boundary_nal(NalUnitType::NonIdrSlice));
}

#[test]
fn converter_writes_parameter_sets_first() {
    let mut converter = H264ToAnnexBConverter::new(record());
    let output = converter.convert(&avcc_packet()).unwrap();
    // 第一个 NAL 用四字节起始码。
    assert!(output.starts_with(&[0x00, 0x00, 0x00, 0x01]));
    assert_eq!(&output[4..8], &[0x67, 0x42, 0xE0, 0x1F]);
    // 参数集只写一次。
    let sps_occurrences = output
        .windows(4)
        .filter(|window| *window == [0x67, 0x42, 0xE0, 0x1F])
        .count();
    assert_eq!(sps_occurrences, 1);
}

#[test]
fn converter_uses_three_byte_start_code_after_the_first_nal() {
    let mut converter = H264ToAnnexBConverter::new(record());
    let output = converter.convert(&avcc_packet()).unwrap();
    // 第一个 NAL（IDR）是访问单元的第一个 ⇒ 四字节起始码。
    let idr_at = output
        .windows(3)
        .position(|window| window == [0x65, 0x88, 0x84])
        .unwrap();
    assert_eq!(&output[idr_at - 4..idr_at], &[0x00, 0x00, 0x00, 0x01]);
    // 第二个 NAL ⇒ 三字节起始码。
    assert!(output.ends_with(&[0x00, 0x00, 0x01, 0x41, 0x9A]));
}

#[test]
fn access_unit_boundary_nal_gets_a_four_byte_start_code() {
    // SPS / PPS / SEI / AUD 是访问单元的**开始** —— 它们自己拿四字节起始码。
    let mut converter = H264ToAnnexBConverter::new(record());
    let mut packet = Vec::new();
    packet.extend_from_slice(&6u32.to_be_bytes());
    packet.extend_from_slice(&[0x06, 0x05, 0x01, 0x80, 0x00, 0x00]); // SEI
    let output = converter.convert(&packet).unwrap();
    let sei_at = output
        .windows(6)
        .position(|window| window == [0x06, 0x05, 0x01, 0x80, 0x00, 0x00])
        .unwrap();
    assert_eq!(&output[sei_at - 4..sei_at], &[0x00, 0x00, 0x00, 0x01]);
}

#[test]
fn converter_output_parses_back_as_nalus() {
    let mut converter = H264ToAnnexBConverter::new(record());
    let output = converter.convert(&avcc_packet()).unwrap();
    let nalus = super::super::parse_nalus(&output);
    // SPS + PPS + 两个 VCL。
    assert_eq!(nalus.len(), 4);
    assert_eq!(nalus[0].nal_unit_type, NalUnitType::Sps);
    assert_eq!(nalus[1].nal_unit_type, NalUnitType::Pps);
    assert_eq!(nalus[2].nal_unit_type, NalUnitType::IdrSlice);
    assert_eq!(nalus[3].nal_unit_type, NalUnitType::NonIdrSlice);
    assert_eq!(nalus[2].data, vec![0x65, 0x88, 0x84]);
}

#[test]
fn converter_only_writes_parameter_sets_once() {
    let mut converter = H264ToAnnexBConverter::new(record());
    let first = converter.convert(&avcc_packet()).unwrap();
    let second = converter.convert(&avcc_packet()).unwrap();
    assert!(first.len() > second.len());
    // 第二个包不再带 SPS / PPS。
    assert!(!second.windows(4).any(|w| w == [0x67, 0x42, 0xE0, 0x1F]));
}

#[test]
fn converter_reset_puts_parameter_sets_back() {
    let mut converter = H264ToAnnexBConverter::new(record());
    let _ = converter.convert(&avcc_packet()).unwrap();
    converter.reset();
    let output = converter.convert(&avcc_packet()).unwrap();
    assert!(output.windows(4).any(|w| w == [0x67, 0x42, 0xE0, 0x1F]));
}

#[test]
fn converter_rejects_empty_packets() {
    let mut converter = H264ToAnnexBConverter::new(record());
    assert!(converter.convert(&[]).is_none());
}

#[test]
fn converter_rejects_truncated_packets() {
    let mut converter = H264ToAnnexBConverter::new(record());
    // 长度前缀说 0x10 字节，实际只有 2 字节。
    let mut packet = Vec::new();
    packet.extend_from_slice(&0x10u32.to_be_bytes());
    packet.extend_from_slice(&[0x65, 0x88]);
    assert!(converter.convert(&packet).is_none());
}

#[test]
fn annex_b_to_avcc_uses_four_byte_lengths() {
    let stream = [
        0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x00, 0x00, 0x01, 0x68, 0xCE,
    ];
    let packets = annex_b_to_avcc(&stream);
    assert_eq!(packets.len(), 2);
    assert_eq!(packets[0], vec![0, 0, 0, 2, 0x67, 0x42]);
    assert_eq!(packets[1], vec![0, 0, 0, 2, 0x68, 0xCE]);
}

#[test]
fn parameter_sets_are_extracted_in_order() {
    let stream = [
        0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x00, 0x00, 0x00,
        0x01, 0x65, 0x88,
    ];
    let (sps, pps) = extract_parameter_sets(&stream);
    assert_eq!(sps.len(), 1);
    assert_eq!(sps[0], vec![0x67, 0x42]);
    assert_eq!(pps.len(), 1);
    assert_eq!(pps[0], vec![0x68, 0xCE]);
}
