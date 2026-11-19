//! `h264/nal.rs` 的断言：NAL 类型、Annex-B 切分、访问单元分组、avcC。

use super::super::{
    find_start_code, parse_nalus, split_access_units, AvcDecoderConfigurationRecord, H264Nalu,
    NalUnitType,
};

fn nalu(nal_ref_idc: u8, nal_unit_type: u8, payload: &[u8]) -> H264Nalu {
    let mut data = Vec::with_capacity(payload.len() + 1);
    data.push((nal_ref_idc << 5) | (nal_unit_type & 0x1f));
    data.extend_from_slice(payload);
    H264Nalu {
        nal_ref_idc,
        nal_unit_type: NalUnitType::from_byte(nal_unit_type),
        data,
    }
}

#[test]
fn nal_types_are_read_from_the_low_five_bits() {
    assert_eq!(NalUnitType::from_byte(0x67), NalUnitType::Sps);
    assert_eq!(NalUnitType::from_byte(0x68), NalUnitType::Pps);
    assert_eq!(NalUnitType::from_byte(0x65), NalUnitType::IdrSlice);
    assert_eq!(NalUnitType::from_byte(0x41), NalUnitType::NonIdrSlice);
    assert_eq!(NalUnitType::from_byte(0x09), NalUnitType::Aud);
    // 高位（nal_ref_idc）不参与类型判定。
    assert_eq!(NalUnitType::from_byte(0xE0), NalUnitType::Unspecified);
}

#[test]
fn vcl_and_parameter_set_classification() {
    assert!(NalUnitType::NonIdrSlice.is_vcl());
    assert!(NalUnitType::IdrSlice.is_vcl());
    assert!(NalUnitType::SliceDataA.is_vcl());
    assert!(!NalUnitType::Sps.is_vcl());
    assert!(!NalUnitType::Pps.is_vcl());
    assert!(!NalUnitType::SeiMessage.is_vcl());

    assert!(NalUnitType::Sps.is_parameter_set());
    assert!(NalUnitType::Pps.is_parameter_set());
    assert!(!NalUnitType::IdrSlice.is_parameter_set());
}

#[test]
fn nalu_header_byte_round_trips() {
    let n = nalu(3, 7, &[0xAA]);
    assert_eq!(n.nal_ref_idc, 3);
    assert_eq!(n.nal_unit_type, NalUnitType::Sps);
    assert_eq!(n.header_byte(), 0x67);
}

#[test]
fn finds_three_byte_start_code() {
    let stream = [0x00, 0x00, 0x01, 0x67, 0x42];
    assert_eq!(find_start_code(&stream), Some((0, 3)));
}

#[test]
fn finds_four_byte_start_code() {
    let stream = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42];
    assert_eq!(find_start_code(&stream), Some((0, 4)));
}

#[test]
fn no_start_code_returns_none() {
    assert_eq!(find_start_code(&[0x67, 0x42]), None);
}

#[test]
fn parses_annex_b_nalus() {
    let stream = [
        0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x00, 0x00, 0x01, 0x65,
        0x88,
    ];
    let nalus = parse_nalus(&stream);
    assert_eq!(nalus.len(), 3);
    assert_eq!(nalus[0].nal_unit_type, NalUnitType::Sps);
    assert_eq!(nalus[0].data, vec![0x67, 0x42]);
    assert_eq!(nalus[1].nal_unit_type, NalUnitType::Pps);
    assert_eq!(nalus[2].nal_unit_type, NalUnitType::IdrSlice);
    assert_eq!(nalus[2].data, vec![0x65, 0x88]);
}

#[test]
fn empty_nalus_are_skipped() {
    let stream = [0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x67, 0x42];
    let nalus = parse_nalus(&stream);
    assert_eq!(nalus.len(), 1);
    assert_eq!(nalus[0].nal_unit_type, NalUnitType::Sps);
}

#[test]
fn access_units_group_parameter_sets_with_the_following_picture() {
    let nalus = vec![
        nalu(3, 7, &[0x42]),
        nalu(3, 8, &[0xCE]),
        nalu(3, 5, &[0x88]),
        nalu(2, 1, &[0x88]),
    ];
    let units = split_access_units(&nalus);
    assert_eq!(units.len(), 2);
    // 参数集落在**它后面那个**访问单元里。
    assert_eq!(units[0].len(), 3);
    assert_eq!(units[0][0].nal_unit_type, NalUnitType::Sps);
    assert_eq!(units[0][2].nal_unit_type, NalUnitType::IdrSlice);
    assert_eq!(units[1].len(), 1);
    assert_eq!(units[1][0].nal_unit_type, NalUnitType::NonIdrSlice);
}

#[test]
fn nal_level_split_cannot_see_picture_boundaries() {
    // 规格里「一个访问单元可以有多个切片」；NAL 类型 2/3/4 都属于 VCL。
    //
    // ⚠️ NAL 头里**没有** `first_mb_in_slice` —— 分不出「同一图像的第二个
    // 切片」与「下一张图像的第一个切片」。所以 NAL 级的切分把每个 VCL
    // 都当成一个新单元的起点（见 `split_access_units` 的文档）。
    // 要按图像分组用 `H264Parser::split_access_units`。
    let nalus = vec![nalu(3, 5, &[0x88]), nalu(3, 5, &[0x88])];
    let units = split_access_units(&nalus);
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].len(), 1);
    assert_eq!(units[1].len(), 1);
}

#[test]
fn avcc_record_is_parsed() {
    // configurationVersion=1, profile=0x42, compat=0xE0, level=0x1F,
    // lengthSizeMinusOne=3, numSPS=1, SPS len=4, numPPS=1, PPS len=3
    let record = [
        0x01, 0x42, 0xE0, 0x1F, 0xFF, 0xE1, 0x00, 0x04, 0x67, 0x42, 0xE0, 0x1F, 0x01, 0x00, 0x03,
        0x68, 0xCE, 0x3C,
    ];
    let parsed = AvcDecoderConfigurationRecord::parse(&record).unwrap();
    assert_eq!(parsed.profile_indication, 0x42);
    assert_eq!(parsed.profile_compatibility, 0xE0);
    assert_eq!(parsed.level_indication, 0x1F);
    assert_eq!(parsed.nal_unit_length_size, 4);
    assert_eq!(parsed.sps_list.len(), 1);
    assert_eq!(parsed.sps_list[0], vec![0x67, 0x42, 0xE0, 0x1F]);
    assert_eq!(parsed.pps_list.len(), 1);
    assert_eq!(parsed.pps_list[0], vec![0x68, 0xCE, 0x3C]);
}

#[test]
fn avcc_record_rejects_truncated_input() {
    assert!(AvcDecoderConfigurationRecord::parse(&[0x01, 0x42]).is_none());
    // 声明的 SPS 长度超出实际数据。
    let bad = [0x01, 0x42, 0xE0, 0x1F, 0xFF, 0xE1, 0x00, 0x40, 0x67];
    assert!(AvcDecoderConfigurationRecord::parse(&bad).is_none());
}
