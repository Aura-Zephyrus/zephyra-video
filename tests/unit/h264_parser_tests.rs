//! `h264/parser.rs` 的断言：SPS / PPS / 切片头。
//!
//! 夹具是**合成**的：用 [`H264BitstreamWriter`] 按 spec 7.4.2 / 7.4.3
//! 逐字段写出来。这样「某个字段的值」在测试里是**看得见的数字**，
//! 而不是一串十六进制常量。

use super::super::{
    parse_pps_payload, parse_slice_header_payload, parse_sps_payload, H264BitstreamWriter,
    H264ParseError, H264Parser, H264Sps, SliceType,
};

/// 写一个 Baseline SPS（64×64，1 个参考帧）。
fn baseline_sps() -> Vec<u8> {
    let mut w = H264BitstreamWriter::new();
    w.write_bits(66, 8); // profile_idc = Baseline
    w.write_bit(true); // constraint_set0_flag
    w.write_bit(true); // constraint_set1_flag
    w.write_bit(true); // constraint_set2_flag
    w.write_bit(false); // constraint_set3_flag
    w.write_bit(false); // constraint_set4_flag
    w.write_bit(false); // constraint_set5_flag
    w.write_bits(0, 2); // reserved_zero_2bits
    w.write_bits(30, 8); // level_idc
    w.write_ue(0); // seq_parameter_set_id
    w.write_ue(0); // log2_max_frame_num_minus4
    w.write_ue(0); // pic_order_cnt_type
    w.write_ue(0); // log2_max_pic_order_cnt_lsb_minus4
    w.write_ue(1); // max_num_ref_frames
    w.write_bit(false); // gaps_in_frame_num_value_allowed_flag
    w.write_ue(3); // pic_width_in_mbs_minus1 ⇒ 64 px
    w.write_ue(3); // pic_height_in_map_units_minus1 ⇒ 64 px
    w.write_bit(true); // frame_mbs_only_flag
    w.write_bit(true); // direct_8x8_inference_flag
    w.write_bit(false); // frame_cropping_flag
    w.write_bit(false); // vui_parameters_present_flag
    w.write_rbsp_trailing_bits();

    let mut nal = vec![0x67];
    nal.extend_from_slice(&w.finish());
    nal
}

/// 写一个 Baseline PPS。
fn baseline_pps() -> Vec<u8> {
    let mut w = H264BitstreamWriter::new();
    w.write_ue(0); // pic_parameter_set_id
    w.write_ue(0); // seq_parameter_set_id
    w.write_bit(false); // entropy_coding_mode_flag
    w.write_bit(false); // bottom_field_pic_order_in_frame_present_flag
    w.write_ue(0); // num_slice_groups_minus1
    w.write_ue(0); // num_ref_idx_l0_default_active_minus1
    w.write_ue(0); // num_ref_idx_l1_default_active_minus1
    w.write_bit(false); // weighted_pred_flag
    w.write_bits(0, 2); // weighted_bipred_idc
    w.write_se(0); // pic_init_qp_minus26
    w.write_se(0); // pic_init_qs_minus26
    w.write_se(0); // chroma_qp_index_offset
    w.write_bit(false); // deblocking_filter_control_present_flag
    w.write_bit(false); // constrained_intra_pred_flag
    w.write_bit(false); // redundant_pic_cnt_present_flag
    w.write_rbsp_trailing_bits();

    let mut nal = vec![0x68];
    nal.extend_from_slice(&w.finish());
    nal
}

/// 切片头的字段（spec 7.4.3）。
///
/// 默认值对应上面那份 Baseline SPS / PPS：`log2_max_frame_num_minus4 == 0`
/// （`frame_num` 4 位）、`log2_max_pic_order_cnt_lsb_minus4 == 0`
/// （`pic_order_cnt_lsb` 4 位）、`frame_mbs_only_flag == true`。
struct SliceSpec {
    nal_header: u8,
    first_mb: u32,
    slice_type: u32,
    pps_id: u32,
    frame_num: u32,
    idr_pic_id: u32,
    pic_order_cnt_lsb: u32,
    slice_qp_delta: i32,
}

impl SliceSpec {
    fn new(nal_header: u8, slice_type: u32) -> Self {
        Self {
            nal_header,
            first_mb: 0,
            slice_type,
            pps_id: 0,
            frame_num: 0,
            idr_pic_id: 0,
            pic_order_cnt_lsb: 0,
            slice_qp_delta: 0,
        }
    }

    fn is_idr(&self) -> bool {
        self.nal_header & 0x1f == 5
    }

    /// 写出 NAL（含头字节）。
    ///
    /// `extra` 写**类型相关**的那一段（`num_ref_idx_active_override_flag`、
    /// `ref_pic_list_modification_flag_lX`、加权预测表……）—— 它们的
    /// 位置在 POC 之后、`dec_ref_pic_marking` 之前，由规格钉死。
    fn write(&self, extra: impl FnOnce(&mut H264BitstreamWriter)) -> Vec<u8> {
        let mut w = H264BitstreamWriter::new();
        w.write_ue(self.first_mb);
        w.write_ue(self.slice_type);
        w.write_ue(self.pps_id);
        w.write_bits(self.frame_num, 4);
        if self.is_idr() {
            w.write_ue(self.idr_pic_id);
        }
        w.write_bits(self.pic_order_cnt_lsb, 4);
        extra(&mut w);
        if self.is_idr() {
            w.write_bit(false); // no_output_of_prior_pics_flag
            w.write_bit(false); // long_term_reference_flag
        } else {
            w.write_bit(false); // adaptive_ref_pic_marking_mode_flag
        }
        w.write_se(self.slice_qp_delta);
        w.write_rbsp_trailing_bits();

        let mut nal = vec![self.nal_header];
        nal.extend_from_slice(&w.finish());
        nal
    }
}

#[test]
fn slice_type_from_raw_folds_the_all_slices_variant() {
    assert_eq!(SliceType::from_raw(0), Some(SliceType::P));
    assert_eq!(SliceType::from_raw(5), Some(SliceType::P));
    assert_eq!(SliceType::from_raw(1), Some(SliceType::B));
    assert_eq!(SliceType::from_raw(6), Some(SliceType::B));
    assert_eq!(SliceType::from_raw(2), Some(SliceType::I));
    assert_eq!(SliceType::from_raw(7), Some(SliceType::I));
    assert_eq!(SliceType::from_raw(3), Some(SliceType::Sp));
    assert_eq!(SliceType::from_raw(4), Some(SliceType::Si));
    assert_eq!(SliceType::from_raw(10), None);
}

#[test]
fn slice_type_predicates_follow_chromium() {
    // Chromium：`slice_type % 5`。
    assert!(SliceType::P.is_p());
    assert!(!SliceType::Sp.is_p());
    assert!(SliceType::Sp.is_sp());
    assert!(SliceType::B.is_b());
    assert!(SliceType::I.is_i());
    assert!(!SliceType::Si.is_i());
    assert!(SliceType::Si.is_si());
}

#[test]
fn baseline_sps_is_parsed() {
    let sps = parse_sps_payload(&baseline_sps()).unwrap();
    assert_eq!(sps.profile_idc, 66);
    assert_eq!(sps.level_idc, 30);
    assert_eq!(sps.seq_parameter_set_id, 0);
    assert_eq!(sps.chroma_format_idc, 1);
    assert_eq!(sps.log2_max_frame_num_minus4, 0);
    assert_eq!(sps.pic_order_cnt_type, 0);
    assert_eq!(sps.max_num_ref_frames, 1);
    assert_eq!(sps.coded_size(), (64, 64));
    assert!(sps.frame_mbs_only_flag);
    assert_eq!(sps.chroma_array_type(), 1);
}

#[test]
fn baseline_sps_scaling_lists_are_flat_sixteen() {
    // Chromium 的 `FillDefaultSeqScalingLists()` 把它们全填 16。
    let sps = parse_sps_payload(&baseline_sps()).unwrap();
    assert!(sps
        .scaling_list4x4
        .iter()
        .all(|list| list.iter().all(|v| *v == 16)));
    assert!(sps
        .scaling_list8x8
        .iter()
        .all(|list| list.iter().all(|v| *v == 16)));
}

#[test]
fn sps_visible_rect_applies_the_crop() {
    let mut sps = parse_sps_payload(&baseline_sps()).unwrap();
    assert_eq!(sps.visible_rect(), (0, 0, 64, 64));

    sps.frame_cropping_flag = true;
    sps.frame_crop_right_offset = 1;
    sps.frame_crop_bottom_offset = 1;
    // 4:2:0 的裁剪单位：横向 2 像素、纵向 2 行。
    assert_eq!(sps.visible_rect(), (0, 0, 62, 62));
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn monochrome_sps_has_no_chroma() {
    let mut sps = H264Sps::default();
    sps.chroma_format_idc = 0;
    assert_eq!(sps.chroma_array_type(), 0);
    sps.chroma_format_idc = 1;
    assert_eq!(sps.chroma_array_type(), 1);
    sps.separate_colour_plane_flag = true;
    assert_eq!(sps.chroma_array_type(), 0);
}

#[test]
fn truncated_sps_is_rejected() {
    assert_eq!(
        parse_sps_payload(&[0x67]),
        Err(H264ParseError::InvalidStream)
    );
    assert_eq!(
        parse_sps_payload(&[0x67, 0x00]),
        Err(H264ParseError::InvalidStream)
    );
}

#[test]
fn baseline_pps_is_parsed() {
    let sps = parse_sps_payload(&baseline_sps()).unwrap();
    let pps = parse_pps_payload(&baseline_pps(), &sps).unwrap();
    assert_eq!(pps.pic_parameter_set_id, 0);
    assert_eq!(pps.seq_parameter_set_id, 0);
    assert!(!pps.entropy_coding_mode_flag);
    assert_eq!(pps.num_ref_idx_l0_default_active_minus1, 0);
    assert_eq!(pps.num_ref_idx_l1_default_active_minus1, 0);
    assert!(!pps.weighted_pred_flag);
    assert_eq!(pps.weighted_bipred_idc, 0);
    assert_eq!(pps.pic_init_qp_minus26, 0);
    assert_eq!(pps.chroma_qp_index_offset, 0);
    // Baseline 的 PPS 后面没有更多语法元素 ⇒ second offset 等于第一个。
    assert_eq!(
        pps.second_chroma_qp_index_offset,
        pps.chroma_qp_index_offset
    );
}

#[test]
fn i_slice_header_is_parsed() {
    let sps = parse_sps_payload(&baseline_sps()).unwrap();
    let pps = parse_pps_payload(&baseline_pps(), &sps).unwrap();
    let mut spec = SliceSpec::new(0x65, 2);
    spec.frame_num = 5;
    let header = parse_slice_header_payload(&spec.write(|_w| {}), &sps, &pps).unwrap();
    assert_eq!(header.slice_type, SliceType::I);
    assert_eq!(header.first_mb_in_slice, 0);
    assert_eq!(header.pic_parameter_set_id, 0);
    assert_eq!(header.frame_num, 5);
    assert!(header.idr_pic_flag);
    assert_eq!(header.nal_ref_idc, 3);
    // I 切片**没有**参考帧列表修改，也没有 num_ref_idx。
    assert_eq!(header.num_ref_idx_l0_active_minus1, 0);
    assert_eq!(header.num_ref_idx_l1_active_minus1, 0);
    assert_eq!(header.slice_qp_delta, 0);
}

#[test]
fn p_slice_header_reads_the_reference_list_flag() {
    let sps = parse_sps_payload(&baseline_sps()).unwrap();
    let pps = parse_pps_payload(&baseline_pps(), &sps).unwrap();
    let mut spec = SliceSpec::new(0x41, 0);
    spec.frame_num = 7;
    let nal = spec.write(|w| {
        // num_ref_idx_active_override_flag
        w.write_bit(false);
        // ref_pic_list_modification_flag_l0
        w.write_bit(false);
    });
    let header = parse_slice_header_payload(&nal, &sps, &pps).unwrap();
    assert_eq!(header.slice_type, SliceType::P);
    assert!(!header.idr_pic_flag);
    assert_eq!(header.nal_ref_idc, 2);
    assert!(!header.num_ref_idx_active_override_flag);
    assert_eq!(header.num_ref_idx_l0_active_minus1, 0);
    // 滑动窗口模式 ⇒ 没有 MMCO。
    assert!(!header.adaptive_ref_pic_marking_mode_flag);
    assert!(header.dec_ref_pic_marking.is_empty());
}

#[test]
fn b_slice_header_reads_direct_spatial_mv_pred_flag() {
    let sps = parse_sps_payload(&baseline_sps()).unwrap();
    let pps = parse_pps_payload(&baseline_pps(), &sps).unwrap();
    let mut spec = SliceSpec::new(0x41, 1);
    spec.frame_num = 9;
    let nal = spec.write(|w| {
        // direct_spatial_mv_pred_flag
        w.write_bit(true);
        // num_ref_idx_active_override_flag
        w.write_bit(false);
        // ref_pic_list_modification_flag_l0
        w.write_bit(false);
        // ref_pic_list_modification_flag_l1
        w.write_bit(false);
    });
    let header = parse_slice_header_payload(&nal, &sps, &pps).unwrap();
    assert_eq!(header.slice_type, SliceType::B);
    assert!(header.direct_spatial_mv_pred_flag);
    assert_eq!(header.num_ref_idx_l1_active_minus1, 0);
}

#[test]
fn idr_slice_reads_idr_pic_id() {
    let sps = parse_sps_payload(&baseline_sps()).unwrap();
    let pps = parse_pps_payload(&baseline_pps(), &sps).unwrap();
    let mut spec = SliceSpec::new(0x65, 2);
    spec.idr_pic_id = 7;
    spec.pic_order_cnt_lsb = 3;
    let header = parse_slice_header_payload(&spec.write(|_w| {}), &sps, &pps).unwrap();
    assert!(header.idr_pic_flag);
    assert_eq!(header.idr_pic_id, 7);
    assert_eq!(header.pic_order_cnt_lsb, 3);
}

#[test]
fn slice_header_rejects_out_of_range_first_mb() {
    let sps = parse_sps_payload(&baseline_sps()).unwrap();
    let pps = parse_pps_payload(&baseline_pps(), &sps).unwrap();
    // 图像是 4×4 = 16 个宏块 ⇒ first_mb_in_slice 必须 < 16。
    let mut spec = SliceSpec::new(0x65, 2);
    spec.first_mb = 16;
    assert_eq!(
        parse_slice_header_payload(&spec.write(|_w| {}), &sps, &pps),
        Err(H264ParseError::InvalidStream)
    );
}

#[test]
fn parser_records_parameter_sets_and_slices() {
    let mut parser = H264Parser::new();
    let mut stream = Vec::new();
    stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    stream.extend_from_slice(&baseline_sps());
    stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    stream.extend_from_slice(&baseline_pps());
    stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    stream.extend_from_slice(&SliceSpec::new(0x65, 2).write(|_w| {}));

    let slices = parser.parse_stream(&stream).unwrap();
    assert_eq!(slices.len(), 1);
    assert_eq!(slices[0].header.slice_type, SliceType::I);
    assert!(parser.get_sps(0).is_some());
    assert!(parser.get_pps(0).is_some());
}

#[test]
fn parser_reset_forgets_parameter_sets() {
    let mut parser = H264Parser::new();
    let mut stream = Vec::new();
    stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    stream.extend_from_slice(&baseline_sps());
    parser.parse_stream(&stream).unwrap();
    assert!(parser.get_sps(0).is_some());
    parser.reset();
    assert!(parser.get_sps(0).is_none());
}

#[test]
fn pps_without_its_sps_is_rejected() {
    let mut parser = H264Parser::new();
    let mut stream = Vec::new();
    stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    stream.extend_from_slice(&baseline_pps());
    // PPS 引用了 SPS 0，但流里没有它 ⇒ 非法（Chromium 同款）。
    assert_eq!(
        parser.parse_stream(&stream),
        Err(H264ParseError::InvalidStream)
    );
}

#[test]
fn slice_without_its_pps_is_rejected() {
    let mut parser = H264Parser::new();
    let mut stream = Vec::new();
    stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    stream.extend_from_slice(&baseline_sps());
    stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    stream.extend_from_slice(&SliceSpec::new(0x65, 2).write(|_w| {}));
    assert_eq!(
        parser.parse_stream(&stream),
        Err(H264ParseError::InvalidStream)
    );
}

#[test]
fn parser_groups_slices_of_one_picture_into_one_access_unit() {
    // 第一张图像：两个切片，第二个的 first_mb_in_slice 不为 0。
    let mut parser = H264Parser::new();
    let mut stream = Vec::new();
    stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    stream.extend_from_slice(&baseline_sps());
    stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    stream.extend_from_slice(&baseline_pps());
    stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    stream.extend_from_slice(&SliceSpec::new(0x65, 2).write(|_w| {}));

    let mut second_slice = SliceSpec::new(0x65, 2);
    second_slice.first_mb = 4; // 同一图像的后续切片
    stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    stream.extend_from_slice(&second_slice.write(|_w| {}));

    // 第三张图像：first_mb_in_slice 回到 0。
    stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    stream.extend_from_slice(&SliceSpec::new(0x65, 2).write(|_w| {}));

    // 先让解析器吃下参数集 —— 按图像分组要靠 SPS / PPS 才能解析切片头。
    parser.parse_stream(&stream).unwrap();
    let nalus = super::super::parse_nalus(&stream);
    let units = parser.split_access_units(&nalus);
    assert_eq!(units.len(), 2);
    // 第一个访问单元含参数集（SPS + PPS）+ 两个切片。
    assert_eq!(units[0].len(), 4);
    assert_eq!(units[1].len(), 1);
}
