//! `dxva2/pic_params.rs` 的断言：DXVA2 结构体的**字节布局**与字段填充规则。
//!
//! # 布局是被量出来的
//!
//! 下面的偏移不是「按 `dxva.h` 推出来的」，而是用 MSVC 编译了一份
//! 探针程序（`offsetof` + `sizeof`）**实测**出来的：
//!
//! ```text
//! sizeof(DXVA_PicParams_H264) = 1040
//! sizeof(DXVA_PicEntry_H264)  = 1
//! sizeof(DXVA_Qmatrix_H264)   = 224
//! sizeof(DXVA_Slice_H264_Short) = 10
//! ```
//!
//! `dxva.h` 把整个 H.264 部分包在 `#pragma pack(push, BeforeDXVApacking, 1)`
//! 里（1 字节对齐）。本 crate 用 `#[repr(C)]` 自然对齐 —— 两者的字段
//! 偏移与总长度**完全一致**，唯独 `DXVA_Slice_H264_Short` 不一样
//! （自然对齐会把它撑到 12 字节，而驱动只认 10 字节），所以那个结构体
//! 单独用字节数组表示。

use super::super::dpb::{DecodedPicture, DecodedPictureBuffer};
use super::{
    build_pic_params, build_qmatrix, DxvaPicEntryH264, DxvaPicParamsError, DxvaPicParamsH264,
    DxvaQmatrixH264, DxvaSliceH264Short, H264BitFields, PicParamsContext, MAX_REF_FRAMES,
};
use crate::h264::parser::{H264Pps, H264SliceHeader, H264Sps, SliceType};

/// `std::mem::offset_of!` 的别名（测试里读起来短一点）。
macro_rules! offset_of {
    ($ty:ty, $field:ident) => {
        std::mem::offset_of!($ty, $field)
    };
}

#[test]
fn pic_params_layout_matches_the_sdk() {
    // 这些偏移是 MSVC 实测值（`offsetof(DXVA_PicParams_H264, ...)`）。
    assert_eq!(
        offset_of!(DxvaPicParamsH264, w_frame_width_in_mbs_minus1),
        0
    );
    assert_eq!(
        offset_of!(DxvaPicParamsH264, w_frame_height_in_mbs_minus1),
        2
    );
    assert_eq!(offset_of!(DxvaPicParamsH264, curr_pic), 4);
    assert_eq!(offset_of!(DxvaPicParamsH264, num_ref_frames), 5);
    assert_eq!(offset_of!(DxvaPicParamsH264, w_bit_fields), 6);
    assert_eq!(offset_of!(DxvaPicParamsH264, bit_depth_luma_minus8), 8);
    assert_eq!(offset_of!(DxvaPicParamsH264, bit_depth_chroma_minus8), 9);
    assert_eq!(offset_of!(DxvaPicParamsH264, reserved16_bits), 10);
    assert_eq!(
        offset_of!(DxvaPicParamsH264, status_report_feedback_number),
        12
    );
    assert_eq!(offset_of!(DxvaPicParamsH264, ref_frame_list), 16);
    assert_eq!(offset_of!(DxvaPicParamsH264, curr_field_order_cnt), 32);
    assert_eq!(offset_of!(DxvaPicParamsH264, field_order_cnt_list), 40);
    assert_eq!(offset_of!(DxvaPicParamsH264, pic_init_qs_minus26), 168);
    assert_eq!(offset_of!(DxvaPicParamsH264, chroma_qp_index_offset), 169);
    assert_eq!(
        offset_of!(DxvaPicParamsH264, second_chroma_qp_index_offset),
        170
    );
    assert_eq!(offset_of!(DxvaPicParamsH264, continuation_flag), 171);
    assert_eq!(offset_of!(DxvaPicParamsH264, pic_init_qp_minus26), 172);
    assert_eq!(
        offset_of!(DxvaPicParamsH264, num_ref_idx_l0_active_minus1),
        173
    );
    assert_eq!(
        offset_of!(DxvaPicParamsH264, num_ref_idx_l1_active_minus1),
        174
    );
    assert_eq!(offset_of!(DxvaPicParamsH264, reserved8_bits_a), 175);
    assert_eq!(offset_of!(DxvaPicParamsH264, frame_num_list), 176);
    assert_eq!(offset_of!(DxvaPicParamsH264, used_for_reference_flags), 208);
    assert_eq!(offset_of!(DxvaPicParamsH264, non_existing_frame_flags), 212);
    assert_eq!(offset_of!(DxvaPicParamsH264, frame_num), 214);
    assert_eq!(
        offset_of!(DxvaPicParamsH264, log2_max_frame_num_minus4),
        216
    );
    assert_eq!(offset_of!(DxvaPicParamsH264, pic_order_cnt_type), 217);
    assert_eq!(
        offset_of!(DxvaPicParamsH264, log2_max_pic_order_cnt_lsb_minus4),
        218
    );
    assert_eq!(
        offset_of!(DxvaPicParamsH264, delta_pic_order_always_zero_flag),
        219
    );
    assert_eq!(
        offset_of!(DxvaPicParamsH264, direct_8x8_inference_flag),
        220
    );
    assert_eq!(offset_of!(DxvaPicParamsH264, entropy_coding_mode_flag), 221);
    assert_eq!(offset_of!(DxvaPicParamsH264, pic_order_present_flag), 222);
    assert_eq!(offset_of!(DxvaPicParamsH264, num_slice_groups_minus1), 223);
    assert_eq!(offset_of!(DxvaPicParamsH264, slice_group_map_type), 224);
    assert_eq!(
        offset_of!(DxvaPicParamsH264, deblocking_filter_control_present_flag),
        225
    );
    assert_eq!(
        offset_of!(DxvaPicParamsH264, redundant_pic_cnt_present_flag),
        226
    );
    assert_eq!(offset_of!(DxvaPicParamsH264, reserved8_bits_b), 227);
    assert_eq!(
        offset_of!(DxvaPicParamsH264, slice_group_change_rate_minus1),
        228
    );
    assert_eq!(offset_of!(DxvaPicParamsH264, slice_group_map), 230);
}

#[test]
fn pic_params_total_size_matches_the_sdk() {
    // 实测：1040 字节。
    assert_eq!(std::mem::size_of::<DxvaPicParamsH264>(), 1040);
    assert_eq!(std::mem::size_of::<DxvaPicEntryH264>(), 1);
    assert_eq!(std::mem::size_of::<DxvaQmatrixH264>(), 224);
    // ⚠️ 这个必须是 10 —— `dxva.h` 把结构体包在 `#pragma pack(1)` 里，
    // 自然对齐会撑到 12，驱动按 10 字节一个条目读。
    assert_eq!(std::mem::size_of::<DxvaSliceH264Short>(), 10);
}

#[test]
fn pic_entry_packs_index_and_flag_into_one_byte() {
    let entry = DxvaPicEntryH264::new(0x7f, true);
    assert_eq!(entry.raw(), 0xff);
    assert_eq!(entry.index7_bits(), 0x7f);
    assert!(entry.associated_flag());

    let entry = DxvaPicEntryH264::new(3, false);
    assert_eq!(entry.raw(), 3);
    assert_eq!(entry.index7_bits(), 3);
    assert!(!entry.associated_flag());

    // 无效条目是 0xFF（Chromium 建解码器时把整张表填成它）。
    assert_eq!(DxvaPicEntryH264::INVALID.raw(), 0xff);
}

#[test]
fn bit_fields_use_the_sdk_bit_order() {
    // 位序是**实测**的（MSVC，小端）：第一个声明的位域占最低位。
    let mut bits = H264BitFields::default();
    bits.set_field_pic_flag(true);
    assert_eq!(bits.raw(), 0x0001);
    assert!(bits.field_pic_flag());

    let mut bits = H264BitFields::default();
    bits.set_mbaff_frame_flag(true);
    assert_eq!(bits.raw(), 0x0002);

    let mut bits = H264BitFields::default();
    bits.set_chroma_format_idc(3);
    assert_eq!(bits.raw(), 0x0030);
    assert_eq!(bits.chroma_format_idc(), 3);

    let mut bits = H264BitFields::default();
    bits.set_weighted_bipred_idc(2);
    assert_eq!(bits.raw(), 0x0400);
    assert_eq!(bits.weighted_bipred_idc(), 2);

    let mut bits = H264BitFields::default();
    bits.set_transform_8x8_mode_flag(true);
    assert_eq!(bits.raw(), 0x2000);

    let mut bits = H264BitFields::default();
    bits.set_min_luma_bipred_size_8x8_flag(true);
    assert_eq!(bits.raw(), 0x4000);

    let mut bits = H264BitFields::default();
    bits.set_intra_pic_flag(true);
    assert_eq!(bits.raw(), 0x8000);
    assert!(bits.intra_pic_flag());
}

#[test]
fn slice_control_entry_is_ten_bytes_little_endian() {
    let entry = DxvaSliceH264Short::new(0x1234_5678, 0x9abc_def0, 3);
    assert_eq!(entry.bs_nal_unit_data_location(), 0x1234_5678);
    assert_eq!(entry.slice_bytes_in_buffer(), 0x9abc_def0);
    assert_eq!(entry.w_bad_slice_chopping(), 3);
    assert_eq!(entry.as_bytes().len(), 10);
    // 小端：低位在前。
    assert_eq!(&entry.as_bytes()[0..4], &0x1234_5678u32.to_le_bytes());
    assert_eq!(&entry.as_bytes()[8..10], &3u16.to_le_bytes());
}

/// 一份最小的 SPS / PPS / 切片头。
#[allow(clippy::field_reassign_with_default)]
fn baseline_inputs() -> (H264Sps, H264Pps, H264SliceHeader) {
    let mut sps = H264Sps::default();
    sps.profile_idc = 66;
    sps.level_idc = 30;
    sps.pic_width_in_mbs_minus1 = 3; // 64 px
    sps.pic_height_in_map_units_minus1 = 3; // 64 px
    sps.max_num_ref_frames = 2;
    sps.chroma_format_idc = 1;
    sps.frame_mbs_only_flag = true;

    let mut pps = H264Pps::default();
    pps.pic_parameter_set_id = 0;
    pps.seq_parameter_set_id = 0;

    let mut slice = H264SliceHeader {
        slice_type: SliceType::I,
        ..H264SliceHeader::default()
    };
    slice.nal_ref_idc = 3;
    slice.frame_num = 7;

    (sps, pps, slice)
}

#[test]
fn constants_are_filled_in() {
    let (sps, pps, slice) = baseline_inputs();
    let current = DecodedPicture {
        picture_index: 2,
        frame_num: 7,
        long_term_frame_idx: None,
        top_field_order_cnt: 11,
        bottom_field_order_cnt: 22,
        is_reference: true,
        non_existing: false,
    };
    let params = build_pic_params(&PicParamsContext {
        sps: &sps,
        pps: &pps,
        slice: &slice,
        current: &current,
        dpb: &[],
    })
    .unwrap();

    // `FillPicParamsWithConstants()`。
    assert!(params.w_bit_fields.mbs_consecutive_flag());
    assert_eq!(params.reserved16_bits, 3);
    assert_eq!(params.continuation_flag, 1);
    assert_eq!(params.reserved8_bits_a, 0);
    assert_eq!(params.reserved8_bits_b, 0);
    assert_eq!(params.status_report_feedback_number, 1);
}

#[test]
fn sps_fields_are_copied() {
    let (mut sps, pps, slice) = baseline_inputs();
    sps.separate_colour_plane_flag = true;
    sps.bit_depth_luma_minus8 = 2;
    sps.bit_depth_chroma_minus8 = 1;
    sps.log2_max_frame_num_minus4 = 1;
    sps.pic_order_cnt_type = 2;
    sps.log2_max_pic_order_cnt_lsb_minus4 = 3;
    sps.delta_pic_order_always_zero_flag = true;
    let current = DecodedPicture {
        picture_index: 0,
        frame_num: 7,
        long_term_frame_idx: None,
        top_field_order_cnt: 0,
        bottom_field_order_cnt: 0,
        is_reference: true,
        non_existing: false,
    };
    let params = build_pic_params(&PicParamsContext {
        sps: &sps,
        pps: &pps,
        slice: &slice,
        current: &current,
        dpb: &[],
    })
    .unwrap();

    assert_eq!(params.num_ref_frames, 2);
    assert_eq!(params.w_frame_width_in_mbs_minus1, 3);
    assert_eq!(params.w_frame_height_in_mbs_minus1, 3);
    assert!(params.w_bit_fields.residual_colour_transform_flag());
    assert_eq!(params.w_bit_fields.chroma_format_idc(), 1);
    assert!(params.w_bit_fields.frame_mbs_only_flag());
    assert_eq!(params.bit_depth_luma_minus8, 2);
    assert_eq!(params.bit_depth_chroma_minus8, 1);
    assert_eq!(params.log2_max_frame_num_minus4, 1);
    assert_eq!(params.pic_order_cnt_type, 2);
    assert_eq!(params.log2_max_pic_order_cnt_lsb_minus4, 3);
    assert_eq!(params.delta_pic_order_always_zero_flag, 1);
}

#[test]
fn min_luma_bipred_size_flag_follows_the_level() {
    // Chromium 的规则：`level_idc >= 31`（level 3.1 起）才允许 8×8 双预测。
    let (mut sps, pps, slice) = baseline_inputs();
    let current = DecodedPicture {
        picture_index: 0,
        frame_num: 0,
        long_term_frame_idx: None,
        top_field_order_cnt: 0,
        bottom_field_order_cnt: 0,
        is_reference: true,
        non_existing: false,
    };
    for (level, expected) in [(30u8, false), (31, true), (40, true)] {
        sps.level_idc = level;
        let params = build_pic_params(&PicParamsContext {
            sps: &sps,
            pps: &pps,
            slice: &slice,
            current: &current,
            dpb: &[],
        })
        .unwrap();
        assert_eq!(
            params.w_bit_fields.min_luma_bipred_size_8x8_flag(),
            expected,
            "level_idc = {level}"
        );
    }
}

#[test]
fn slice_groups_are_rejected() {
    // FMO 不被 DXVA2 支持 —— Chromium 在同一处直接失败。
    let (sps, mut pps, slice) = baseline_inputs();
    pps.num_slice_groups_minus1 = 1;
    let current = DecodedPicture {
        picture_index: 0,
        frame_num: 0,
        long_term_frame_idx: None,
        top_field_order_cnt: 0,
        bottom_field_order_cnt: 0,
        is_reference: true,
        non_existing: false,
    };
    assert_eq!(
        build_pic_params(&PicParamsContext {
            sps: &sps,
            pps: &pps,
            slice: &slice,
            current: &current,
            dpb: &[],
        }),
        Err(DxvaPicParamsError::UnsupportedSliceGroups)
    );
}

#[test]
fn ref_frame_list_follows_the_dpb_order_not_the_reference_list() {
    // ⚠️ 这是最容易搞错的一处：`RefFrameList` 按 **DPB 顺序**填，
    // 不是按 H.264 参考列表 0 / 1 的顺序。驱动会自己构造参考列表。
    let (sps, pps, slice) = baseline_inputs();
    let dpb = vec![
        DecodedPicture {
            picture_index: 5,
            frame_num: 3,
            long_term_frame_idx: None,
            top_field_order_cnt: 30,
            bottom_field_order_cnt: 31,
            is_reference: true,
            non_existing: false,
        },
        DecodedPicture {
            picture_index: 6,
            frame_num: 4,
            long_term_frame_idx: None,
            top_field_order_cnt: 40,
            bottom_field_order_cnt: 41,
            is_reference: false, // 非参考帧：占位置但条目留 0xFF
            non_existing: false,
        },
        DecodedPicture {
            picture_index: 7,
            frame_num: 5,
            long_term_frame_idx: Some(1),
            top_field_order_cnt: 50,
            bottom_field_order_cnt: 51,
            is_reference: true,
            non_existing: false,
        },
    ];
    let current = DecodedPicture {
        picture_index: 8,
        frame_num: 7,
        long_term_frame_idx: None,
        top_field_order_cnt: 70,
        bottom_field_order_cnt: 71,
        is_reference: true,
        non_existing: false,
    };
    let params = build_pic_params(&PicParamsContext {
        sps: &sps,
        pps: &pps,
        slice: &slice,
        current: &current,
        dpb: &dpb,
    })
    .unwrap();

    // 槽位 0 = DPB 的第 0 张。
    assert_eq!(params.ref_frame_list[0].index7_bits(), 5);
    assert!(!params.ref_frame_list[0].associated_flag());
    // 槽位 1 = 非参考帧 ⇒ 留 `0xFF`（INVALID）。
    assert_eq!(params.ref_frame_list[1], DxvaPicEntryH264::INVALID);
    // 槽位 2 = 长期帧 ⇒ AssociatedFlag = 1。
    assert_eq!(params.ref_frame_list[2].index7_bits(), 7);
    assert!(params.ref_frame_list[2].associated_flag());
    // 槽位 3 = DPB 之外 ⇒ INVALID。
    assert_eq!(params.ref_frame_list[3], DxvaPicEntryH264::INVALID);

    // `UsedForReferenceFlags`：每张参考帧占两个位。
    assert_eq!(params.used_for_reference_flags & 0b11, 0b11);
    assert_eq!((params.used_for_reference_flags >> 2) & 0b11, 0);
    assert_eq!((params.used_for_reference_flags >> 4) & 0b11, 0b11);

    // `FrameNumList`：长期帧用 `long_term_pic_num`（Chromium 把它等同于
    // `long_term_frame_idx`），短期帧用 `frame_num`。
    assert_eq!(params.frame_num_list[0], 3);
    assert_eq!(params.frame_num_list[2], 1);

    assert_eq!(params.field_order_cnt_list[0], [30, 31]);
    assert_eq!(params.field_order_cnt_list[2], [50, 51]);
}

#[test]
fn current_picture_fields_are_copied() {
    let (sps, pps, slice) = baseline_inputs();
    let current = DecodedPicture {
        picture_index: 9,
        frame_num: 13,
        long_term_frame_idx: None,
        top_field_order_cnt: 100,
        bottom_field_order_cnt: 200,
        is_reference: true,
        non_existing: false,
    };
    let params = build_pic_params(&PicParamsContext {
        sps: &sps,
        pps: &pps,
        slice: &slice,
        current: &current,
        dpb: &[],
    })
    .unwrap();
    assert_eq!(params.curr_pic.index7_bits(), 9);
    assert!(params.w_bit_fields.ref_pic_flag());
    assert_eq!(params.frame_num, 13);
    assert_eq!(params.curr_field_order_cnt, [100, 200]);
}

#[test]
fn intra_slice_sets_the_intra_pic_flag() {
    let (sps, pps, mut slice) = baseline_inputs();
    let current = DecodedPicture {
        picture_index: 0,
        frame_num: 0,
        long_term_frame_idx: None,
        top_field_order_cnt: 0,
        bottom_field_order_cnt: 0,
        is_reference: true,
        non_existing: false,
    };
    slice.slice_type = SliceType::I;
    let params = build_pic_params(&PicParamsContext {
        sps: &sps,
        pps: &pps,
        slice: &slice,
        current: &current,
        dpb: &[],
    })
    .unwrap();
    assert!(params.w_bit_fields.intra_pic_flag());

    slice.slice_type = SliceType::P;
    let params = build_pic_params(&PicParamsContext {
        sps: &sps,
        pps: &pps,
        slice: &slice,
        current: &current,
        dpb: &[],
    })
    .unwrap();
    assert!(!params.w_bit_fields.intra_pic_flag());
}

#[test]
fn qmatrix_prefers_the_pps_scaling_lists() {
    let (mut sps, mut pps, _slice) = baseline_inputs();
    sps.scaling_list4x4[0] = [1; 16];
    sps.scaling_list8x8[0] = [2; 64];
    pps.scaling_list4x4[0] = [3; 16];
    pps.scaling_list8x8[0] = [4; 64];

    // PPS 没有缩放列表 ⇒ 用 SPS 的。
    let matrix = build_qmatrix(&sps, &pps);
    assert_eq!(matrix.b_scaling_lists4x4[0], [1; 16]);
    assert_eq!(matrix.b_scaling_lists8x8[0], [2; 64]);

    // PPS 有缩放列表 ⇒ 用 PPS 的。
    pps.pic_scaling_matrix_present_flag = true;
    let matrix = build_qmatrix(&sps, &pps);
    assert_eq!(matrix.b_scaling_lists4x4[0], [3; 16]);
    assert_eq!(matrix.b_scaling_lists8x8[0], [4; 64]);
}

#[test]
fn qmatrix_only_carries_two_of_the_six_8x8_lists() {
    // DXVA2 只带两张 8×8 缩放列表 —— Chromium 拷的也是前两张。
    let (mut sps, pps, _slice) = baseline_inputs();
    for i in 0..6 {
        sps.scaling_list8x8[i] = [i as u8; 64];
    }
    let matrix = build_qmatrix(&sps, &pps);
    assert_eq!(matrix.b_scaling_lists8x8[0], [0; 64]);
    assert_eq!(matrix.b_scaling_lists8x8[1], [1; 64]);
}

#[test]
fn dpb_allocates_unique_picture_indices() {
    let mut dpb = DecodedPictureBuffer::new();
    let first = dpb.allocate_picture_index();
    let second = dpb.allocate_picture_index();
    assert_ne!(first, second);
    assert!(first < MAX_REF_FRAMES as u8);
    assert!(second < MAX_REF_FRAMES as u8);
}
