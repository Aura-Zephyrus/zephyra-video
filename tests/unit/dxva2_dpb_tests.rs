//! `dxva2/dpb.rs` 的断言：POC 计算、DPB 维护、MMCO 与参考帧列表。
//!
//! # 与 Chromium 的对应关系
//!
//! | 本 crate | Chromium |
//! |---|---|
//! | [`DecodedPictureBuffer::compute_picture_order_cnt`] | `media/gpu/h264_decoder.cc` 的 `CalculatePicOrderCounts()` |
//! | [`DecodedPictureBuffer::commit_picture`] | `media/gpu/h264_decoder.cc` 的 `FinishPicture()` / `ReferencePictureMarking()` |
//! | [`DecodedPictureBuffer::reference_lists`] | `media/gpu/h264_decoder.cc` 的 `ModifyRefPicLists()` |
//!
//! DPB 的行为**必须**与 Chromium 一致：DXVA2 的驱动不认「哪张是上一帧」，
//! 它只认 `RefFrameList` 里给出的索引 —— 列表算错，画面就是花的。

use super::{DecodedPicture, DecodedPictureBuffer, PictureOrderCnt, MAX_PICTURES};
use crate::h264::parser::{DecRefPicMarking, H264ParseError, H264SliceHeader, H264Sps, SliceType};

/// 一份最小的 SPS：16×16 宏块、`pic_order_cnt_type == 0`。
#[allow(clippy::field_reassign_with_default)]
fn sps_poc_type0() -> H264Sps {
    let mut sps = H264Sps::default();
    sps.pic_width_in_mbs_minus1 = 0;
    sps.pic_height_in_map_units_minus1 = 0;
    sps.log2_max_frame_num_minus4 = 0; // MaxFrameNum = 16
    sps.log2_max_pic_order_cnt_lsb_minus4 = 0; // MaxPicOrderCntLsb = 16
    sps.pic_order_cnt_type = 0;
    sps.max_num_ref_frames = 2;
    sps
}

/// 一份最小的切片头。
fn slice(frame_num: u32, pic_order_cnt_lsb: u32, nal_ref_idc: u8) -> H264SliceHeader {
    H264SliceHeader {
        frame_num,
        pic_order_cnt_lsb,
        nal_ref_idc,
        slice_type: SliceType::P,
        ..H264SliceHeader::default()
    }
}

/// 算 POC 并把它提交进 DPB —— 真实解码器就是这么用的。
fn commit(
    dpb: &mut DecodedPictureBuffer,
    sps: &H264Sps,
    slice: &H264SliceHeader,
    picture_index: u8,
) -> DecodedPicture {
    let order_cnt = dpb
        .compute_picture_order_cnt(sps, slice)
        .expect("the POC type is supported");
    dpb.commit_picture(sps, slice, picture_index, &order_cnt)
}

#[test]
fn poc_type_zero_tracks_the_lsb() {
    let sps = sps_poc_type0();
    let mut dpb = DecodedPictureBuffer::new();

    // IDR：POC 从 0 开始（Chromium 的 `prev_ref_*` 初值是 -1，
    // 但第一张一定是 IDR，那条分支不会走到）。
    let mut idr = slice(0, 0, 3);
    idr.idr_pic_flag = true;
    let order = dpb
        .compute_picture_order_cnt(&sps, &idr)
        .expect("POC type 0 is supported");
    assert_eq!(order.top_field_order_cnt, 0);
    assert_eq!(order.bottom_field_order_cnt, 0);
    dpb.commit_picture(&sps, &idr, 0, &order);

    let second = slice(1, 2, 3);
    let order = dpb
        .compute_picture_order_cnt(&sps, &second)
        .expect("POC type 0 is supported");
    assert_eq!(order.top_field_order_cnt, 2);
    assert_eq!(order.bottom_field_order_cnt, 2);
    assert_eq!(order.pic_order_cnt_lsb, 2);
}

#[test]
fn poc_type_zero_wraps_the_msb_forward() {
    // `MaxPicOrderCntLsb == 256` ⇒ 半个周期是 128。
    let mut sps = sps_poc_type0();
    sps.log2_max_pic_order_cnt_lsb_minus4 = 4;
    let mut dpb = DecodedPictureBuffer::new();

    let mut idr = slice(0, 0, 3);
    idr.idr_pic_flag = true;
    let order = dpb
        .compute_picture_order_cnt(&sps, &idr)
        .expect("POC type 0 is supported");
    assert_eq!(order.top_field_order_cnt, 0);
    assert_eq!(order.pic_order_cnt_msb, 0);
    dpb.commit_picture(&sps, &idr, 0, &order);

    // LSB 逐步上升（每次的增量都小于半个周期 ⇒ 不触发回绕）。
    for (step, lsb) in [(1u32, 100u32), (2, 200)] {
        let current = slice(step, lsb, 3);
        let order = dpb
            .compute_picture_order_cnt(&sps, &current)
            .expect("POC type 0 is supported");
        assert_eq!(order.pic_order_cnt_msb, 0);
        assert_eq!(order.top_field_order_cnt, lsb as i32);
        dpb.commit_picture(&sps, &current, step as u8, &order);
    }

    // LSB 回绕到 0：相对上一张（200）超过半个周期 ⇒ MSB 加一整轮。
    let wrapped = slice(3, 0, 3);
    let order = dpb
        .compute_picture_order_cnt(&sps, &wrapped)
        .expect("POC type 0 is supported");
    assert_eq!(order.pic_order_cnt_msb, 256);
    assert_eq!(order.top_field_order_cnt, 256);
}

#[test]
fn poc_type_zero_does_not_adjust_the_msb_twice() {
    // ⚠️ 这是本模块最容易写错的地方：把 MSB 的调整写进持久状态、再拿
    // 它去算 `msb`，等于同一轮调整做了两遍 ⇒ POC 整整差一个
    // `MaxPicOrderCntLsb`。
    let sps = sps_poc_type0();
    let mut dpb = DecodedPictureBuffer::new();

    let mut idr = slice(0, 0, 3);
    idr.idr_pic_flag = true;
    let order = dpb
        .compute_picture_order_cnt(&sps, &idr)
        .expect("POC type 0 is supported");
    assert_eq!(order.pic_order_cnt_msb, 0);
    dpb.commit_picture(&sps, &idr, 0, &order);

    // LSB 15 相对 0 已经「回绕」过一次（15 > 16 / 2）⇒ MSB = -16。
    let high = slice(1, 15, 3);
    let order = dpb
        .compute_picture_order_cnt(&sps, &high)
        .expect("POC type 0 is supported");
    assert_eq!(order.pic_order_cnt_msb, -16);
    assert_eq!(order.top_field_order_cnt, -1);
    dpb.commit_picture(&sps, &high, 1, &order);

    // 一次调整 ⇒ MSB = 0（-16 + 16），POC = 1。
    // 调整两遍的话这里会是 -16 ⇒ POC = -15。
    let low = slice(2, 1, 3);
    let order = dpb
        .compute_picture_order_cnt(&sps, &low)
        .expect("POC type 0 is supported");
    assert_eq!(order.pic_order_cnt_msb, 0);
    assert_eq!(order.top_field_order_cnt, 1);
}

#[test]
fn poc_type_one_uses_the_frame_number_offset() {
    let mut sps = sps_poc_type0();
    sps.pic_order_cnt_type = 1;
    sps.num_ref_frames_in_pic_order_cnt_cycle = 1;
    sps.expected_delta_per_pic_order_cnt_cycle = 2;
    sps.offset_for_ref_frame = vec![2];
    let mut dpb = DecodedPictureBuffer::new();

    let mut idr = slice(0, 0, 3);
    idr.idr_pic_flag = true;
    let order = dpb
        .compute_picture_order_cnt(&sps, &idr)
        .expect("POC type 1 is supported");
    assert_eq!(order.top_field_order_cnt, 0);
    dpb.commit_picture(&sps, &idr, 0, &order);

    let second = slice(1, 0, 3);
    let order = dpb
        .compute_picture_order_cnt(&sps, &second)
        .expect("POC type 1 is supported");
    // `frameNumInPicOrderCntCycle` 从 **0** 开始数 ⇒ 第一张的 delta 是 2。
    assert_eq!(order.top_field_order_cnt, 2);
}

#[test]
fn poc_type_two_derives_from_the_frame_number() {
    let mut sps = sps_poc_type0();
    sps.pic_order_cnt_type = 2;
    let dpb = DecodedPictureBuffer::new();

    let first = slice(3, 0, 3);
    let order = dpb
        .compute_picture_order_cnt(&sps, &first)
        .expect("POC type 2 is supported");
    assert_eq!(order.top_field_order_cnt, 6);
    assert_eq!(order.bottom_field_order_cnt, 6);

    // 非参考帧的 POC 比参考帧小 1。
    let non_ref = slice(4, 0, 0);
    let order = dpb
        .compute_picture_order_cnt(&sps, &non_ref)
        .expect("POC type 2 is supported");
    assert_eq!(order.top_field_order_cnt, 7);
}

#[test]
fn an_idr_clears_the_dpb() {
    let sps = sps_poc_type0();
    let mut dpb = DecodedPictureBuffer::new();

    commit(&mut dpb, &sps, &slice(0, 0, 3), 0);
    assert_eq!(dpb.pictures().len(), 1);

    let mut idr = slice(1, 0, 3);
    idr.idr_pic_flag = true;
    commit(&mut dpb, &sps, &idr, 1);
    // IDR 之后 DPB 里只剩 IDR 自己。
    assert_eq!(dpb.pictures().len(), 1);
    assert_eq!(dpb.pictures()[0].picture_index, 1);
}

#[test]
fn a_non_reference_picture_does_not_stay_in_the_dpb() {
    let sps = sps_poc_type0();
    let mut dpb = DecodedPictureBuffer::new();

    // 参考帧。
    commit(&mut dpb, &sps, &slice(0, 0, 3), 0);
    // 非参考帧（`nal_ref_idc == 0`）。
    commit(&mut dpb, &sps, &slice(1, 1, 0), 1);
    // 再来一张参考帧 ⇒ 上一张非参考帧被清掉。
    commit(&mut dpb, &sps, &slice(2, 2, 3), 2);

    assert!(dpb.pictures().iter().all(|p| p.is_reference));
    assert!(!dpb.pictures().iter().any(|p| p.picture_index == 1));
}

#[test]
fn the_sliding_window_evicts_the_oldest_short_term_picture() {
    let mut sps = sps_poc_type0();
    sps.max_num_ref_frames = 2;
    let mut dpb = DecodedPictureBuffer::new();

    commit(&mut dpb, &sps, &slice(0, 0, 3), 0);
    commit(&mut dpb, &sps, &slice(1, 1, 3), 1);
    // 第三张 ⇒ 容量已满 ⇒ 淘汰最老的一张。
    commit(&mut dpb, &sps, &slice(2, 2, 3), 2);

    let referenced: Vec<u8> = dpb
        .pictures()
        .iter()
        .filter(|p| p.is_reference)
        .map(|p| p.picture_index)
        .collect();
    assert_eq!(referenced, vec![1, 2]);
}

#[test]
fn mmco_one_drops_the_unmarked_picture_from_the_dpb() {
    let sps = sps_poc_type0();
    let mut dpb = DecodedPictureBuffer::new();

    commit(&mut dpb, &sps, &slice(0, 0, 3), 0);
    commit(&mut dpb, &sps, &slice(1, 1, 3), 1);

    // MMCO 1：把 `frame_num = 当前 - diff - 1` 的短期帧标记为不用。
    let mut adaptive = slice(2, 2, 3);
    adaptive.adaptive_ref_pic_marking_mode_flag = true;
    adaptive.dec_ref_pic_marking = vec![DecRefPicMarking {
        memory_management_control_operation: 1,
        difference_of_pic_nums_minus1: 1,
        ..DecRefPicMarking::default()
    }];
    commit(&mut dpb, &sps, &adaptive, 2);

    // 帧号 0 的那张被标记为不用 ⇒ 规格与 Chromium 都会把它移出 DPB。
    assert!(!dpb.pictures().iter().any(|p| p.frame_num == 0));
    assert!(dpb.pictures().iter().any(|p| p.frame_num == 1));
}

#[test]
fn mmco_three_turns_the_current_picture_into_a_long_term_one() {
    let sps = sps_poc_type0();
    let mut dpb = DecodedPictureBuffer::new();

    let mut adaptive = slice(0, 0, 3);
    adaptive.adaptive_ref_pic_marking_mode_flag = true;
    adaptive.dec_ref_pic_marking = vec![DecRefPicMarking {
        memory_management_control_operation: 3,
        long_term_frame_idx: 2,
        ..DecRefPicMarking::default()
    }];
    commit(&mut dpb, &sps, &adaptive, 0);

    let current = dpb
        .pictures()
        .iter()
        .find(|p| p.picture_index == 0)
        .expect("the current picture is in the DPB");
    assert!(current.is_long_term());
    assert_eq!(current.long_term_frame_idx, Some(2));
    // Chromium 把 `long_term_pic_num` 等同于 `long_term_frame_idx`。
    assert_eq!(current.long_term_pic_num(), 2);
}

#[test]
fn mmco_five_unmarks_every_long_term_picture() {
    let sps = sps_poc_type0();
    let mut dpb = DecodedPictureBuffer::new();

    let mut adaptive = slice(0, 0, 3);
    adaptive.adaptive_ref_pic_marking_mode_flag = true;
    adaptive.dec_ref_pic_marking = vec![DecRefPicMarking {
        memory_management_control_operation: 3,
        long_term_frame_idx: 0,
        ..DecRefPicMarking::default()
    }];
    commit(&mut dpb, &sps, &adaptive, 0);

    let mut adaptive = slice(1, 1, 3);
    adaptive.adaptive_ref_pic_marking_mode_flag = true;
    adaptive.dec_ref_pic_marking = vec![DecRefPicMarking {
        memory_management_control_operation: 5,
        ..DecRefPicMarking::default()
    }];
    commit(&mut dpb, &sps, &adaptive, 1);

    assert!(dpb
        .pictures()
        .iter()
        .all(|p| !p.is_long_term() || !p.is_reference));
}

#[test]
fn reference_lists_put_the_newest_short_term_picture_first() {
    // P 切片：列表 0 = 短期帧按 `pic_num` 递减（新的在前），列表 1 不用。
    let mut sps = sps_poc_type0();
    sps.max_num_ref_frames = 4;
    let mut dpb = DecodedPictureBuffer::new();

    commit(&mut dpb, &sps, &slice(0, 0, 3), 0);
    commit(&mut dpb, &sps, &slice(1, 1, 3), 1);
    commit(&mut dpb, &sps, &slice(2, 2, 3), 2);

    let current = slice(3, 3, 3);
    let (list0, list1) = dpb
        .reference_lists(&sps, &current, 3)
        .expect("the reference lists can be built");
    assert_eq!(
        list0.iter().map(|p| p.frame_num).collect::<Vec<_>>(),
        vec![2, 1, 0]
    );
    // P 切片没有列表 1。
    assert!(list1.is_empty());
}

#[test]
fn reference_list_modification_moves_a_picture_to_the_front() {
    let sps = sps_poc_type0();
    let mut dpb = DecodedPictureBuffer::new();

    commit(&mut dpb, &sps, &slice(0, 0, 3), 0);
    commit(&mut dpb, &sps, &slice(1, 1, 3), 1);

    // 把帧号 0 挪到列表 0 的最前面。
    let mut current = slice(2, 2, 3);
    current.ref_pic_list_modification_flag_l0 = true;
    current.ref_list_l0_modifications = vec![crate::h264::parser::RefPicListModification {
        modification_of_pic_nums_idc: 0,
        abs_diff_pic_num_minus1: 1,
        long_term_pic_num: 0,
    }];
    let (list0, _) = dpb
        .reference_lists(&sps, &current, 2)
        .expect("the reference lists can be built");
    assert_eq!(list0[0].frame_num, 0);
}

#[test]
fn a_modification_that_finds_nothing_fails_like_chromium() {
    // Chromium 遇到「引用了一张不在 DPB 里的图」直接失败，不塞占位。
    let sps = sps_poc_type0();
    let mut dpb = DecodedPictureBuffer::new();
    commit(&mut dpb, &sps, &slice(0, 0, 3), 0);

    let mut current = slice(1, 1, 3);
    current.ref_pic_list_modification_flag_l0 = true;
    current.ref_list_l0_modifications = vec![crate::h264::parser::RefPicListModification {
        modification_of_pic_nums_idc: 0,
        abs_diff_pic_num_minus1: 5,
        long_term_pic_num: 0,
    }];
    assert_eq!(
        dpb.reference_lists(&sps, &current, 1),
        Err(H264ParseError::InvalidStream)
    );
}

#[test]
fn picture_indices_do_not_repeat_before_the_dpb_is_full() {
    let mut dpb = DecodedPictureBuffer::new();
    let first = dpb.allocate_picture_index();
    let second = dpb.allocate_picture_index();
    assert_ne!(first, second);
    assert!(first < MAX_PICTURES as u8);
    assert!(second < MAX_PICTURES as u8);
}

#[test]
fn a_full_dpb_reports_the_conflict_instead_of_reusing_an_index() {
    let mut sps = sps_poc_type0();
    sps.max_num_ref_frames = MAX_PICTURES as u32;
    let mut dpb = DecodedPictureBuffer::new();
    for i in 0..MAX_PICTURES {
        commit(&mut dpb, &sps, &slice(i as u32, i as u32, 3), i as u8);
    }
    assert_eq!(dpb.pictures().len(), MAX_PICTURES);
    assert_eq!(dpb.allocate_picture_index(), u8::MAX);
}

#[test]
fn reset_forgets_everything() {
    let sps = sps_poc_type0();
    let mut dpb = DecodedPictureBuffer::new();
    commit(&mut dpb, &sps, &slice(0, 0, 3), 0);
    dpb.reset();
    assert!(dpb.pictures().is_empty());
    // 复位后索引从头开始。
    assert_eq!(dpb.allocate_picture_index(), 0);
}

#[test]
fn a_picture_copy_is_independent_of_the_buffer() {
    let picture = DecodedPicture {
        picture_index: 3,
        frame_num: 7,
        long_term_frame_idx: Some(1),
        top_field_order_cnt: 10,
        bottom_field_order_cnt: 20,
        is_reference: true,
        non_existing: false,
    };
    let copy = picture;
    assert_eq!(picture, copy);
    assert!(copy.is_long_term());
    assert_eq!(copy.long_term_pic_num(), 1);
}

#[test]
fn the_order_count_carries_the_state_the_next_picture_needs() {
    // `PictureOrderCnt` 把「同一张图算出来的五个值」捆在一起 ——
    // 拆开就会丢掉它们同属一张图这个事实。
    let order = PictureOrderCnt {
        top_field_order_cnt: 1,
        bottom_field_order_cnt: 2,
        pic_order_cnt_msb: 3,
        pic_order_cnt_lsb: 4,
        frame_num_offset: 5,
    };
    assert_eq!(order.top_field_order_cnt, 1);
    assert_eq!(order.bottom_field_order_cnt, 2);
    assert_eq!(order.pic_order_cnt_msb, 3);
    assert_eq!(order.pic_order_cnt_lsb, 4);
    assert_eq!(order.frame_num_offset, 5);
}
