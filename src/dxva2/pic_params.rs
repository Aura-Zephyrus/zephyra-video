//! DXVA2 的 H.264 图片参数与量化矩阵。
//!
//! 对应 Chromium `media/gpu/windows/d3d_h264_accelerator.cc` 的
//! `FillPicParamsWithConstants()` / `PicParamsFromSPS()` / `PicParamsFromPPS()`
//! / `PicParamsFromSliceHeader()` / `PicParamsFromPic()` 与 `SetRefFrames()`。
//!
//! # 纯逻辑
//!
//! 本模块只**拼字节**，不碰任何 D3D11 对象，因此没有 unsafe，
//! 在任何平台上都能单测 —— 字节布局错了这件事，必须在没有 GPU 的机器上
//! 就能被抓住，而不是等到某台机器的驱动拒绝解码时才暴露。
//!
//! # 布局是被**量出来的**，不是推出来的
//!
//! `dxva.h` 里整个 H.264 部分被 `#pragma pack(push, BeforeDXVApacking, 1)`
//! 包着（1 字节对齐）。本 crate 用 `#[repr(C)]` 自然对齐 —— 两者的
//! **字段偏移与总长度完全一致**（实测：`sizeof(DXVA_PicParams_H264) == 1040`，
//! 各字段偏移见 `tests/unit/dxva2_pic_params_tests.rs`），但
//! `DXVA_Slice_H264_Short` 不一样：自然对齐会把它撑到 12 字节，
//! 而驱动只认 10 字节 —— 所以那个结构体单独用字节数组表示。

use crate::dxva2::dpb::DecodedPicture;
use crate::h264::parser::{H264Pps, H264SliceHeader, H264Sps};

/// `DXVA_PicParams_H264.RefFrameList` 的长度（也是 DXVA2 的 DPB 上限）。
pub const MAX_REF_FRAMES: usize = 16;

/// `DXVA_PicEntry_H264` —— 1 字节。
///
/// 位布局：bit 0..6 = `Index7Bits`，bit 7 = `AssociatedFlag`
/// （在 `RefFrameList` 里表示「长期参考帧」，在 `CurrPic` 里表示「底场」）。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DxvaPicEntryH264 {
    b_pic_entry: u8,
}

impl DxvaPicEntryH264 {
    /// 「无效」条目（`0xFF`）。
    ///
    /// Chromium 在建解码器时把 `ref_frame_list_` 全填成 `0xFF`
    /// （`d3d_h264_accelerator.cc` 的 `SetRefFrames` 上方），
    /// 表示「这个槽位没有参考帧」。
    pub const INVALID: Self = Self { b_pic_entry: 0xff };

    /// 按 `Index7Bits` + `AssociatedFlag` 造一个条目。
    pub fn new(index7_bits: u8, associated_flag: bool) -> Self {
        Self {
            b_pic_entry: (index7_bits & 0x7f) | if associated_flag { 0x80 } else { 0 },
        }
    }

    /// 图像索引（7 位）。
    pub fn index7_bits(&self) -> u8 {
        self.b_pic_entry & 0x7f
    }

    /// 关联标志（长期参考 / 底场）。
    pub fn associated_flag(&self) -> bool {
        self.b_pic_entry & 0x80 != 0
    }

    /// 原始字节。
    pub fn raw(&self) -> u8 {
        self.b_pic_entry
    }
}

impl Default for DxvaPicEntryH264 {
    fn default() -> Self {
        Self::INVALID
    }
}

/// `DXVA_PicParams_H264` 里的位域（`wBitFields`）。
///
/// 位序**实测**过（MSVC，小端）：第一个声明的位域占最低位。
/// 这不是可以「按直觉推」的东西 —— 写反一位，驱动就会把
/// `chroma_format_idc` 读成 `weighted_pred_flag`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct H264BitFields(u16);

impl H264BitFields {
    const FIELD_PIC_FLAG: u16 = 1 << 0;
    const MBAFF_FRAME_FLAG: u16 = 1 << 1;
    const RESIDUAL_COLOUR_TRANSFORM_FLAG: u16 = 1 << 2;
    const SP_FOR_SWITCH_FLAG: u16 = 1 << 3;
    const CHROMA_FORMAT_IDC_SHIFT: u32 = 4;
    const CHROMA_FORMAT_IDC_MASK: u16 = 0x03 << 4;
    const REF_PIC_FLAG: u16 = 1 << 6;
    const CONSTRAINED_INTRA_PRED_FLAG: u16 = 1 << 7;
    const WEIGHTED_PRED_FLAG: u16 = 1 << 8;
    const WEIGHTED_BIPRED_IDC_SHIFT: u32 = 9;
    const WEIGHTED_BIPRED_IDC_MASK: u16 = 0x03 << 9;
    const MBS_CONSECUTIVE_FLAG: u16 = 1 << 11;
    const FRAME_MBS_ONLY_FLAG: u16 = 1 << 12;
    const TRANSFORM_8X8_MODE_FLAG: u16 = 1 << 13;
    const MIN_LUMA_BIPRED_SIZE_8X8_FLAG: u16 = 1 << 14;
    const INTRA_PIC_FLAG: u16 = 1 << 15;

    /// 原始位域。
    pub fn raw(&self) -> u16 {
        self.0
    }

    /// `field_pic_flag`。
    pub fn field_pic_flag(&self) -> bool {
        self.0 & Self::FIELD_PIC_FLAG != 0
    }
    /// `MbaffFrameFlag`。
    pub fn mbaff_frame_flag(&self) -> bool {
        self.0 & Self::MBAFF_FRAME_FLAG != 0
    }
    /// `residual_colour_transform_flag`（来自 SPS 的 `separate_colour_plane_flag`）。
    pub fn residual_colour_transform_flag(&self) -> bool {
        self.0 & Self::RESIDUAL_COLOUR_TRANSFORM_FLAG != 0
    }
    /// `sp_for_switch_flag`。
    pub fn sp_for_switch_flag(&self) -> bool {
        self.0 & Self::SP_FOR_SWITCH_FLAG != 0
    }
    /// `chroma_format_idc`（2 位）。
    pub fn chroma_format_idc(&self) -> u16 {
        (self.0 & Self::CHROMA_FORMAT_IDC_MASK) >> Self::CHROMA_FORMAT_IDC_SHIFT
    }
    /// `RefPicFlag` —— 当前图像是不是参考帧。
    pub fn ref_pic_flag(&self) -> bool {
        self.0 & Self::REF_PIC_FLAG != 0
    }
    /// `constrained_intra_pred_flag`。
    pub fn constrained_intra_pred_flag(&self) -> bool {
        self.0 & Self::CONSTRAINED_INTRA_PRED_FLAG != 0
    }
    /// `weighted_pred_flag`。
    pub fn weighted_pred_flag(&self) -> bool {
        self.0 & Self::WEIGHTED_PRED_FLAG != 0
    }
    /// `weighted_bipred_idc`（2 位）。
    pub fn weighted_bipred_idc(&self) -> u16 {
        (self.0 & Self::WEIGHTED_BIPRED_IDC_MASK) >> Self::WEIGHTED_BIPRED_IDC_SHIFT
    }
    /// `MbsConsecutiveFlag`。
    pub fn mbs_consecutive_flag(&self) -> bool {
        self.0 & Self::MBS_CONSECUTIVE_FLAG != 0
    }
    /// `frame_mbs_only_flag`。
    pub fn frame_mbs_only_flag(&self) -> bool {
        self.0 & Self::FRAME_MBS_ONLY_FLAG != 0
    }
    /// `transform_8x8_mode_flag`。
    pub fn transform_8x8_mode_flag(&self) -> bool {
        self.0 & Self::TRANSFORM_8X8_MODE_FLAG != 0
    }
    /// `MinLumaBipredSize8x8Flag`。
    pub fn min_luma_bipred_size_8x8_flag(&self) -> bool {
        self.0 & Self::MIN_LUMA_BIPRED_SIZE_8X8_FLAG != 0
    }
    /// `IntraPicFlag` —— 当前图像是不是只含 I 切片。
    pub fn intra_pic_flag(&self) -> bool {
        self.0 & Self::INTRA_PIC_FLAG != 0
    }

    /// 置 `field_pic_flag`。
    pub fn set_field_pic_flag(&mut self, value: bool) {
        self.set(Self::FIELD_PIC_FLAG, value);
    }
    /// 置 `MbaffFrameFlag`。
    pub fn set_mbaff_frame_flag(&mut self, value: bool) {
        self.set(Self::MBAFF_FRAME_FLAG, value);
    }
    /// 置 `residual_colour_transform_flag`。
    pub fn set_residual_colour_transform_flag(&mut self, value: bool) {
        self.set(Self::RESIDUAL_COLOUR_TRANSFORM_FLAG, value);
    }
    /// 置 `sp_for_switch_flag`。
    pub fn set_sp_for_switch_flag(&mut self, value: bool) {
        self.set(Self::SP_FOR_SWITCH_FLAG, value);
    }
    /// 置 `chroma_format_idc`（只保留 2 位）。
    pub fn set_chroma_format_idc(&mut self, value: u16) {
        self.0 = (self.0 & !Self::CHROMA_FORMAT_IDC_MASK)
            | ((value & 0x03) << Self::CHROMA_FORMAT_IDC_SHIFT);
    }
    /// 置 `RefPicFlag`。
    pub fn set_ref_pic_flag(&mut self, value: bool) {
        self.set(Self::REF_PIC_FLAG, value);
    }
    /// 置 `constrained_intra_pred_flag`。
    pub fn set_constrained_intra_pred_flag(&mut self, value: bool) {
        self.set(Self::CONSTRAINED_INTRA_PRED_FLAG, value);
    }
    /// 置 `weighted_pred_flag`。
    pub fn set_weighted_pred_flag(&mut self, value: bool) {
        self.set(Self::WEIGHTED_PRED_FLAG, value);
    }
    /// 置 `weighted_bipred_idc`（只保留 2 位）。
    pub fn set_weighted_bipred_idc(&mut self, value: u16) {
        self.0 = (self.0 & !Self::WEIGHTED_BIPRED_IDC_MASK)
            | ((value & 0x03) << Self::WEIGHTED_BIPRED_IDC_SHIFT);
    }
    /// 置 `MbsConsecutiveFlag`。
    pub fn set_mbs_consecutive_flag(&mut self, value: bool) {
        self.set(Self::MBS_CONSECUTIVE_FLAG, value);
    }
    /// 置 `frame_mbs_only_flag`。
    pub fn set_frame_mbs_only_flag(&mut self, value: bool) {
        self.set(Self::FRAME_MBS_ONLY_FLAG, value);
    }
    /// 置 `transform_8x8_mode_flag`。
    pub fn set_transform_8x8_mode_flag(&mut self, value: bool) {
        self.set(Self::TRANSFORM_8X8_MODE_FLAG, value);
    }
    /// 置 `MinLumaBipredSize8x8Flag`。
    pub fn set_min_luma_bipred_size_8x8_flag(&mut self, value: bool) {
        self.set(Self::MIN_LUMA_BIPRED_SIZE_8X8_FLAG, value);
    }
    /// 置 `IntraPicFlag`。
    pub fn set_intra_pic_flag(&mut self, value: bool) {
        self.set(Self::INTRA_PIC_FLAG, value);
    }

    fn set(&mut self, mask: u16, value: bool) {
        if value {
            self.0 |= mask;
        } else {
            self.0 &= !mask;
        }
    }
}

/// `DXVA_PicParams_H264` —— 1040 字节（实测值）。
///
/// 字段顺序与 `dxva.h` 完全一致；偏移由
/// `tests/unit/dxva2_pic_params_tests.rs` 钉住。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DxvaPicParamsH264 {
    pub w_frame_width_in_mbs_minus1: u16,
    pub w_frame_height_in_mbs_minus1: u16,
    pub curr_pic: DxvaPicEntryH264,
    pub num_ref_frames: u8,
    pub w_bit_fields: H264BitFields,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub reserved16_bits: u16,
    pub status_report_feedback_number: u32,
    pub ref_frame_list: [DxvaPicEntryH264; MAX_REF_FRAMES],
    pub curr_field_order_cnt: [i32; 2],
    pub field_order_cnt_list: [[i32; 2]; MAX_REF_FRAMES],
    pub pic_init_qs_minus26: i8,
    pub chroma_qp_index_offset: i8,
    pub second_chroma_qp_index_offset: i8,
    pub continuation_flag: u8,
    pub pic_init_qp_minus26: i8,
    pub num_ref_idx_l0_active_minus1: u8,
    pub num_ref_idx_l1_active_minus1: u8,
    pub reserved8_bits_a: u8,
    pub frame_num_list: [u16; MAX_REF_FRAMES],
    pub used_for_reference_flags: u32,
    pub non_existing_frame_flags: u16,
    pub frame_num: u16,
    pub log2_max_frame_num_minus4: u8,
    pub pic_order_cnt_type: u8,
    pub log2_max_pic_order_cnt_lsb_minus4: u8,
    pub delta_pic_order_always_zero_flag: u8,
    pub direct_8x8_inference_flag: u8,
    pub entropy_coding_mode_flag: u8,
    pub pic_order_present_flag: u8,
    pub num_slice_groups_minus1: u8,
    pub slice_group_map_type: u8,
    pub deblocking_filter_control_present_flag: u8,
    pub redundant_pic_cnt_present_flag: u8,
    pub reserved8_bits_b: u8,
    pub slice_group_change_rate_minus1: u16,
    pub slice_group_map: [u8; 810],
}

impl Default for DxvaPicParamsH264 {
    fn default() -> Self {
        // SAFETY: 结构体里的每个字段都是**任何**位模式都合法的
        // （整数、位域、数组），所以全零初始化是合法的。
        // `DXVA_PicEntry_H264` 的全零是「索引 0、不是长期帧」，
        // 与 SDK 的 `0xFF`（无效）不同 —— 无效条目由
        // `set_ref_frames()` 单独填。
        Self {
            w_frame_width_in_mbs_minus1: 0,
            w_frame_height_in_mbs_minus1: 0,
            curr_pic: DxvaPicEntryH264::INVALID,
            num_ref_frames: 0,
            w_bit_fields: H264BitFields::default(),
            bit_depth_luma_minus8: 0,
            bit_depth_chroma_minus8: 0,
            reserved16_bits: 0,
            status_report_feedback_number: 0,
            ref_frame_list: [DxvaPicEntryH264::INVALID; MAX_REF_FRAMES],
            curr_field_order_cnt: [0; 2],
            field_order_cnt_list: [[0; 2]; MAX_REF_FRAMES],
            pic_init_qs_minus26: 0,
            chroma_qp_index_offset: 0,
            second_chroma_qp_index_offset: 0,
            continuation_flag: 0,
            pic_init_qp_minus26: 0,
            num_ref_idx_l0_active_minus1: 0,
            num_ref_idx_l1_active_minus1: 0,
            reserved8_bits_a: 0,
            frame_num_list: [0; MAX_REF_FRAMES],
            used_for_reference_flags: 0,
            non_existing_frame_flags: 0,
            frame_num: 0,
            log2_max_frame_num_minus4: 0,
            pic_order_cnt_type: 0,
            log2_max_pic_order_cnt_lsb_minus4: 0,
            delta_pic_order_always_zero_flag: 0,
            direct_8x8_inference_flag: 0,
            entropy_coding_mode_flag: 0,
            pic_order_present_flag: 0,
            num_slice_groups_minus1: 0,
            slice_group_map_type: 0,
            deblocking_filter_control_present_flag: 0,
            redundant_pic_cnt_present_flag: 0,
            reserved8_bits_b: 0,
            slice_group_change_rate_minus1: 0,
            slice_group_map: [0; 810],
        }
    }
}

/// `DXVA_Qmatrix_H264` —— 224 字节（实测值）。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DxvaQmatrixH264 {
    pub b_scaling_lists4x4: [[u8; 16]; 6],
    pub b_scaling_lists8x8: [[u8; 64]; 2],
}

impl Default for DxvaQmatrixH264 {
    fn default() -> Self {
        Self {
            b_scaling_lists4x4: [[0; 16]; 6],
            b_scaling_lists8x8: [[0; 64]; 2],
        }
    }
}

/// `DXVA_Slice_H264_Short` —— 10 字节（实测值）。
///
/// 用字节数组而不是字段：SDK 的 `dxva.h` 把这个结构体包在
/// `#pragma pack(1)` 里，自然对齐会多出 2 字节尾部填充，
/// 而驱动按 10 字节一个条目来读切片控制数据。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DxvaSliceH264Short {
    bytes: [u8; 10],
}

impl DxvaSliceH264Short {
    /// 结构体长度（字节）。
    pub const SIZE: usize = 10;

    /// 造一个切片控制条目。
    ///
    /// - `bs_nal_unit_data_location`：NAL 数据在位流缓冲里的**字节偏移**；
    /// - `slice_bytes_in_buffer`：这个切片在位流缓冲里占多少字节；
    /// - `w_bad_slice_chopping`：0 = 完整；1 = 缺结尾；2 = 缺开头；3 = 都缺。
    pub fn new(
        bs_nal_unit_data_location: u32,
        slice_bytes_in_buffer: u32,
        w_bad_slice_chopping: u16,
    ) -> Self {
        let mut bytes = [0u8; Self::SIZE];
        bytes[0..4].copy_from_slice(&bs_nal_unit_data_location.to_le_bytes());
        bytes[4..8].copy_from_slice(&slice_bytes_in_buffer.to_le_bytes());
        bytes[8..10].copy_from_slice(&w_bad_slice_chopping.to_le_bytes());
        Self { bytes }
    }

    /// `BSNALunitDataLocation`。
    pub fn bs_nal_unit_data_location(&self) -> u32 {
        u32::from_le_bytes(self.bytes[0..4].try_into().expect("4 bytes"))
    }
    /// `SliceBytesInBuffer`。
    pub fn slice_bytes_in_buffer(&self) -> u32 {
        u32::from_le_bytes(self.bytes[4..8].try_into().expect("4 bytes"))
    }
    /// `wBadSliceChopping`。
    pub fn w_bad_slice_chopping(&self) -> u16 {
        u16::from_le_bytes(self.bytes[8..10].try_into().expect("2 bytes"))
    }

    /// 原始字节。
    pub fn as_bytes(&self) -> &[u8; 10] {
        &self.bytes
    }
}

impl Default for DxvaSliceH264Short {
    fn default() -> Self {
        Self::new(0, 0, 0)
    }
}

/// 拼 `DXVA_PicParams_H264` 时的失败原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DxvaPicParamsError {
    /// FMO（切片组，`num_slice_groups_minus1 != 0`）不被 DXVA2 支持。
    ///
    /// Chromium 也在同一处直接失败（`PicParamsFromPPS()` 返回 `false`）。
    UnsupportedSliceGroups,
    /// DPB 里的图片超过 16 张 —— `RefFrameList` 装不下。
    TooManyPictures,
}

/// 拼图片参数需要的全部输入。
#[derive(Debug, Clone, Copy)]
pub struct PicParamsContext<'a> {
    pub sps: &'a H264Sps,
    pub pps: &'a H264Pps,
    pub slice: &'a H264SliceHeader,
    /// 当前正在解码的图片（索引 / 帧号 / POC / 是否参考帧）。
    pub current: &'a DecodedPicture,
    /// DPB 里的图片，**按 DPB 顺序**（不是按参考列表顺序）。
    pub dpb: &'a [DecodedPicture],
}

/// 按 Chromium 的规则拼出 `DXVA_PicParams_H264`。
///
/// 字段填充顺序照抄 `d3d_h264_accelerator.cc`：
/// 常量 → SPS → PPS → 切片头 → 当前图片 → 参考帧列表。
pub fn build_pic_params(
    ctx: &PicParamsContext<'_>,
) -> Result<DxvaPicParamsH264, DxvaPicParamsError> {
    let mut params = DxvaPicParamsH264::default();

    fill_pic_params_with_constants(&mut params);
    pic_params_from_sps(&mut params, ctx.sps, ctx.slice.field_pic_flag);
    pic_params_from_pps(&mut params, ctx.pps)?;
    pic_params_from_slice_header(&mut params, ctx.slice);
    pic_params_from_pic(&mut params, ctx.current);
    set_ref_frames(&mut params, ctx.dpb);

    Ok(params)
}

/// `FillPicParamsWithConstants()`。
fn fill_pic_params_with_constants(params: &mut DxvaPicParamsH264) {
    // DXVA H.264 规范：「除非受限模式 profile 明确支持 0，否则为 1」。
    params.w_bit_fields.set_mbs_consecutive_flag(true);
    // 最新的 DXVA 解码指南要求：跟着软件解码器走就填 3。
    params.reserved16_bits = 3;
    // `ContinuationFlag` 表示「其余字段已填完」。
    params.continuation_flag = 1;
    // 除非 `ConfigDecoderSpecific` 的第 13 位被置起，否则必须是 0。
    params.reserved8_bits_a = 0;
    // 没有用到，必须是 0。
    params.reserved8_bits_b = 0;
    // 始终为 1。
    params.status_report_feedback_number = 1;
}

/// `PicParamsFromSPS()`。
fn pic_params_from_sps(params: &mut DxvaPicParamsH264, sps: &H264Sps, field_pic: bool) {
    // H.264 规格里叫 `max_num_ref_frames`，DXVA 沿用旧名 `num_ref_frames`。
    params.num_ref_frames = sps.max_num_ref_frames.min(u8::MAX as u32) as u8;
    params.w_frame_width_in_mbs_minus1 = sps.pic_width_in_mbs_minus1.min(u16::MAX as u32) as u16;
    params.w_frame_height_in_mbs_minus1 =
        sps.pic_height_in_map_units_minus1.min(u16::MAX as u32) as u16;
    params
        .w_bit_fields
        .set_residual_colour_transform_flag(sps.separate_colour_plane_flag);
    params
        .w_bit_fields
        .set_chroma_format_idc(sps.chroma_format_idc as u16);
    params
        .w_bit_fields
        .set_frame_mbs_only_flag(sps.frame_mbs_only_flag);
    params.bit_depth_luma_minus8 = sps.bit_depth_luma_minus8.min(u8::MAX as u32) as u8;
    params.bit_depth_chroma_minus8 = sps.bit_depth_chroma_minus8.min(u8::MAX as u32) as u8;
    params.log2_max_frame_num_minus4 = sps.log2_max_frame_num_minus4.min(u8::MAX as u32) as u8;
    params.pic_order_cnt_type = sps.pic_order_cnt_type.min(u8::MAX as u32) as u8;
    params.log2_max_pic_order_cnt_lsb_minus4 =
        sps.log2_max_pic_order_cnt_lsb_minus4.min(u8::MAX as u32) as u8;
    params.delta_pic_order_always_zero_flag = u8::from(sps.delta_pic_order_always_zero_flag);
    params.direct_8x8_inference_flag = u8::from(sps.direct_8x8_inference_flag);

    params
        .w_bit_fields
        .set_mbaff_frame_flag(sps.mb_adaptive_frame_field_flag && field_pic);
    params.w_bit_fields.set_field_pic_flag(field_pic);
    // Chromium 的规则：`level_idc >= 31`（即 level 3.1 及以上）才允许
    // 8×8 双预测。
    params
        .w_bit_fields
        .set_min_luma_bipred_size_8x8_flag(sps.level_idc >= 31);
}

/// `PicParamsFromPPS()`。
fn pic_params_from_pps(
    params: &mut DxvaPicParamsH264,
    pps: &H264Pps,
) -> Result<(), DxvaPicParamsError> {
    params
        .w_bit_fields
        .set_constrained_intra_pred_flag(pps.constrained_intra_pred_flag);
    params
        .w_bit_fields
        .set_weighted_pred_flag(pps.weighted_pred_flag);
    params
        .w_bit_fields
        .set_weighted_bipred_idc(pps.weighted_bipred_idc as u16);
    params
        .w_bit_fields
        .set_transform_8x8_mode_flag(pps.transform_8x8_mode_flag);
    params.pic_init_qs_minus26 = pps.pic_init_qs_minus26.clamp(-128, 127) as i8;
    params.chroma_qp_index_offset = pps.chroma_qp_index_offset.clamp(-128, 127) as i8;
    params.second_chroma_qp_index_offset = pps.second_chroma_qp_index_offset.clamp(-128, 127) as i8;
    params.pic_init_qp_minus26 = pps.pic_init_qp_minus26.clamp(-128, 127) as i8;
    params.num_ref_idx_l0_active_minus1 =
        pps.num_ref_idx_l0_default_active_minus1.min(u8::MAX as u32) as u8;
    params.num_ref_idx_l1_active_minus1 =
        pps.num_ref_idx_l1_default_active_minus1.min(u8::MAX as u32) as u8;
    params.entropy_coding_mode_flag = u8::from(pps.entropy_coding_mode_flag);
    params.pic_order_present_flag = u8::from(pps.bottom_field_pic_order_in_frame_present_flag);
    params.deblocking_filter_control_present_flag =
        u8::from(pps.deblocking_filter_control_present_flag);
    params.redundant_pic_cnt_present_flag = u8::from(pps.redundant_pic_cnt_present_flag);
    params.num_slice_groups_minus1 = pps.num_slice_groups_minus1.min(u8::MAX as u32) as u8;

    if params.num_slice_groups_minus1 != 0 {
        // FMO 不被 DXVA2 支持 —— Chromium 在同一处直接失败。
        return Err(DxvaPicParamsError::UnsupportedSliceGroups);
    }
    Ok(())
}

/// `PicParamsFromSliceHeader()`。
fn pic_params_from_slice_header(params: &mut DxvaPicParamsH264, slice: &H264SliceHeader) {
    params
        .w_bit_fields
        .set_sp_for_switch_flag(slice.sp_for_switch_flag);
    params.w_bit_fields.set_field_pic_flag(slice.field_pic_flag);
    // `CurrPic.AssociatedFlag` 在这里是「底场标志」。
    params.curr_pic = DxvaPicEntryH264::new(params.curr_pic.index7_bits(), slice.bottom_field_flag);
    params
        .w_bit_fields
        .set_intra_pic_flag(slice.slice_type.is_i());
}

/// `PicParamsFromPic()`。
fn pic_params_from_pic(params: &mut DxvaPicParamsH264, current: &DecodedPicture) {
    params.curr_pic = DxvaPicEntryH264::new(current.picture_index, current.is_long_term());
    params.w_bit_fields.set_ref_pic_flag(current.is_reference);
    params.frame_num = current.frame_num;

    if params.w_bit_fields.field_pic_flag() && params.curr_pic.associated_flag() {
        params.curr_field_order_cnt[1] = current.bottom_field_order_cnt;
        params.curr_field_order_cnt[0] = 0;
    } else if params.w_bit_fields.field_pic_flag() && !params.curr_pic.associated_flag() {
        params.curr_field_order_cnt[0] = current.top_field_order_cnt;
        params.curr_field_order_cnt[1] = 0;
    } else {
        params.curr_field_order_cnt[0] = current.top_field_order_cnt;
        params.curr_field_order_cnt[1] = current.bottom_field_order_cnt;
    }
}

/// `SetRefFrames()` —— 按 **DPB 顺序**填参考帧列表。
///
/// ⚠️ 这里填的是 **DPB 的顺序**，不是 H.264 参考列表 0 / 1 的顺序。
/// Chromium 也是这么做的（`d3d_h264_accelerator.cc` 遍历 `dpb`），
/// 驱动会**自己**按 `ref_pic_list_modification` 构造参考列表 ——
/// 应用层替驱动排一遍，出来的画面反而是错的。
///
/// 槽位下标 `i` 是 DPB 里的**位置**，不是「第几个参考帧」：
/// 非参考帧占着位置但条目留 `0xFF`（`INVALID`），这一点必须照抄，
/// 否则 `UsedForReferenceFlags` 的位就与 `RefFrameList` 对不上了。
fn set_ref_frames(params: &mut DxvaPicParamsH264, dpb: &[DecodedPicture]) {
    params.ref_frame_list = [DxvaPicEntryH264::INVALID; MAX_REF_FRAMES];
    params.field_order_cnt_list = [[0i32; 2]; MAX_REF_FRAMES];
    params.frame_num_list = [0u16; MAX_REF_FRAMES];
    params.used_for_reference_flags = 0;
    params.non_existing_frame_flags = 0;

    for (i, picture) in dpb.iter().enumerate() {
        if i >= MAX_REF_FRAMES {
            break;
        }
        // 非参考帧：不填条目，但**位置照占**。
        if !picture.is_reference {
            continue;
        }
        params.ref_frame_list[i] =
            DxvaPicEntryH264::new(picture.picture_index, picture.is_long_term());
        params.field_order_cnt_list[i][0] = picture.top_field_order_cnt;
        params.field_order_cnt_list[i][1] = picture.bottom_field_order_cnt;
        // 长期帧用 `long_term_pic_num`，短期帧用 `frame_num`。
        params.frame_num_list[i] = if picture.is_long_term() {
            picture.long_term_pic_num()
        } else {
            picture.frame_num
        };
        // 每张参考帧占两个位：0 = 短期，1 = 长期。
        params.used_for_reference_flags |= 3 << (2 * i);
        params.non_existing_frame_flags |= u16::from(picture.non_existing) << i;
    }
}

/// 按 Chromium 的规则拼 `DXVA_Qmatrix_H264`。
///
/// `pps.pic_scaling_matrix_present_flag` 为真时用 PPS 的缩放列表，
/// 否则用 SPS 的（`d3d_h264_accelerator.cc` 的 `SubmitSlice()`）。
pub fn build_qmatrix(sps: &H264Sps, pps: &H264Pps) -> DxvaQmatrixH264 {
    let use_pps = pps.pic_scaling_matrix_present_flag;
    let list4x4 = if use_pps {
        &pps.scaling_list4x4
    } else {
        &sps.scaling_list4x4
    };
    let list8x8 = if use_pps {
        &pps.scaling_list8x8
    } else {
        &sps.scaling_list8x8
    };
    // ⚠️ DXVA2 只带**两张** 8×8 缩放列表，而 H.264 有六张。
    // Chromium 拷的是前两张（`memcpy` 的长度正好是
    // `sizeof(iq_matrix.bScalingLists8x8)`），这里跟它一样 ——
    // 两边对不上时，出了问题没法判断是谁的锅。
    DxvaQmatrixH264 {
        b_scaling_lists4x4: *list4x4,
        b_scaling_lists8x8: [list8x8[0], list8x8[1]],
    }
}

#[cfg(test)]
#[path = "../../tests/unit/dxva2_pic_params_tests.rs"]
mod tests;
