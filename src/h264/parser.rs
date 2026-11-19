//! H.264 参数集与切片头解析。
//!
//! 移植自 Chromium `media/parsers/h264_parser.{h,cc}`。
//!
//! # 为什么必须自己解析
//!
//! DXVA2 的 `ID3D11VideoDecoder` **不吃整帧码流**：它要的是
//! `DXVA_PicParams_H264`（帧级参数）与 `DXVA_SliceContainer_Header`
//! （切片级参数），而这两个结构里的每个字段都来自 SPS / PPS / 切片头的
//! 语法元素。于是「解析位流」不是可选的附加能力，而是硬件解码的**前提**。
//!
//! Chromium 的软件 H.264 解码器（`media/gpu/h264_decoder.cc`）做的是同一件事，
//! 本 crate 的字段命名与它保持一致，便于对照。

use std::collections::HashMap;

use super::bit_reader::H264BitReader;
use super::nal::{H264Nalu, NalUnitType};

/// 4×4 缩放列表长度。
pub const SCALING_LIST_4X4_LEN: usize = 16;
/// 8×8 缩放列表长度。
pub const SCALING_LIST_8X8_LEN: usize = 64;

/// 解析失败的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264ParseError {
    /// 位流不合法（字段越界、语法元素缺失……）。
    InvalidStream,
    /// 位流合法，但本实现**不支持**（隔行、slice group > 1 等）。
    UnsupportedStream,
}

/// 序列参数集（SPS）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H264Sps {
    pub profile_idc: u8,
    pub constraint_set0_flag: bool,
    pub constraint_set1_flag: bool,
    pub constraint_set2_flag: bool,
    pub constraint_set3_flag: bool,
    pub constraint_set4_flag: bool,
    pub constraint_set5_flag: bool,
    pub level_idc: u8,
    pub seq_parameter_set_id: u32,

    pub chroma_format_idc: u32,
    pub separate_colour_plane_flag: bool,
    pub bit_depth_luma_minus8: u32,
    pub bit_depth_chroma_minus8: u32,
    pub qpprime_y_zero_transform_bypass_flag: bool,

    pub seq_scaling_matrix_present_flag: bool,
    pub scaling_list4x4: [[u8; SCALING_LIST_4X4_LEN]; 6],
    pub scaling_list8x8: [[u8; SCALING_LIST_8X8_LEN]; 6],

    pub log2_max_frame_num_minus4: u32,
    pub pic_order_cnt_type: u32,
    pub log2_max_pic_order_cnt_lsb_minus4: u32,
    pub delta_pic_order_always_zero_flag: bool,
    pub offset_for_non_ref_pic: i32,
    pub offset_for_top_to_bottom_field: i32,
    pub num_ref_frames_in_pic_order_cnt_cycle: u32,
    pub offset_for_ref_frame: Vec<i32>,
    pub expected_delta_per_pic_order_cnt_cycle: i32,
    pub max_num_ref_frames: u32,
    pub gaps_in_frame_num_value_allowed_flag: bool,
    pub pic_width_in_mbs_minus1: u32,
    pub pic_height_in_map_units_minus1: u32,
    pub frame_mbs_only_flag: bool,
    pub mb_adaptive_frame_field_flag: bool,
    pub direct_8x8_inference_flag: bool,
    pub frame_cropping_flag: bool,
    pub frame_crop_left_offset: u32,
    pub frame_crop_right_offset: u32,
    pub frame_crop_top_offset: u32,
    pub frame_crop_bottom_offset: u32,

    pub vui_parameters_present_flag: bool,
    /// 像素宽高比（`sar_width` / `sar_height`），0 表示未指定。
    pub sar_width: u32,
    pub sar_height: u32,
    pub video_signal_type_present_flag: bool,
    pub video_full_range_flag: bool,
    pub colour_description_present_flag: bool,
    pub colour_primaries: u32,
    pub transfer_characteristics: u32,
    pub matrix_coefficients: u32,
}

impl Default for H264Sps {
    fn default() -> Self {
        Self {
            profile_idc: 0,
            constraint_set0_flag: false,
            constraint_set1_flag: false,
            constraint_set2_flag: false,
            constraint_set3_flag: false,
            constraint_set4_flag: false,
            constraint_set5_flag: false,
            level_idc: 0,
            seq_parameter_set_id: 0,
            chroma_format_idc: 1,
            separate_colour_plane_flag: false,
            bit_depth_luma_minus8: 0,
            bit_depth_chroma_minus8: 0,
            qpprime_y_zero_transform_bypass_flag: false,
            seq_scaling_matrix_present_flag: false,
            scaling_list4x4: [[16; SCALING_LIST_4X4_LEN]; 6],
            scaling_list8x8: [[16; SCALING_LIST_8X8_LEN]; 6],
            log2_max_frame_num_minus4: 0,
            pic_order_cnt_type: 0,
            log2_max_pic_order_cnt_lsb_minus4: 0,
            delta_pic_order_always_zero_flag: false,
            offset_for_non_ref_pic: 0,
            offset_for_top_to_bottom_field: 0,
            num_ref_frames_in_pic_order_cnt_cycle: 0,
            offset_for_ref_frame: Vec::new(),
            expected_delta_per_pic_order_cnt_cycle: 0,
            max_num_ref_frames: 0,
            gaps_in_frame_num_value_allowed_flag: false,
            pic_width_in_mbs_minus1: 0,
            pic_height_in_map_units_minus1: 0,
            frame_mbs_only_flag: true,
            mb_adaptive_frame_field_flag: false,
            direct_8x8_inference_flag: true,
            frame_cropping_flag: false,
            frame_crop_left_offset: 0,
            frame_crop_right_offset: 0,
            frame_crop_top_offset: 0,
            frame_crop_bottom_offset: 0,
            vui_parameters_present_flag: false,
            sar_width: 0,
            sar_height: 0,
            video_signal_type_present_flag: false,
            video_full_range_flag: false,
            colour_description_present_flag: false,
            colour_primaries: 2,
            transfer_characteristics: 2,
            matrix_coefficients: 2,
        }
    }
}

impl H264Sps {
    /// 编码尺寸（像素）。
    pub fn coded_size(&self) -> (u32, u32) {
        let width = (self.pic_width_in_mbs_minus1 + 1) * 16;
        let height = (self.pic_height_in_map_units_minus1 + 1)
            * 16
            * if self.frame_mbs_only_flag { 1 } else { 2 };
        (width, height)
    }

    /// 色度数组类型（spec A.1.3 的 `ChromaArrayType`）。
    ///
    /// 对应 Chromium `H264SPS::chroma_array_type`：单色（`chroma_format_idc
    /// == 0`）或独立色度平面（`separate_colour_plane_flag`）时为 0，
    /// 否则等于 `chroma_format_idc`。
    ///
    /// 这个值决定**规格里哪些语法元素存在**：加权预测表的色度分量、
    /// `chroma_log2_weight_denom` 都以它为条件。
    pub fn chroma_array_type(&self) -> u32 {
        if self.chroma_format_idc == 0 || self.separate_colour_plane_flag {
            0
        } else {
            self.chroma_format_idc
        }
    }

    /// 可见区域（已应用裁剪）。
    ///
    /// 对应 Chromium `H264SPS::GetVisibleRect()`：裁剪偏移的单位取决于
    /// 色度格式（4:2:0 / 4:2:2 的横向单位是 2 像素，纵向是 2 行）。
    pub fn visible_rect(&self) -> (u32, u32, u32, u32) {
        let (width, height) = self.coded_size();
        if !self.frame_cropping_flag {
            return (0, 0, width, height);
        }
        let (crop_unit_x, crop_unit_y) = match self.chroma_format_idc {
            0 => (1u32, 1u32), // 单色
            1 => (2, 2),       // 4:2:0
            2 => (2, 1),       // 4:2:2
            _ => (1, 1),       // 4:4:4
        };
        let sub_width_x = if self.separate_colour_plane_flag {
            1
        } else {
            crop_unit_x
        };
        let sub_height_y = crop_unit_y * if self.frame_mbs_only_flag { 1 } else { 2 };
        let crop_left = self.frame_crop_left_offset * sub_width_x;
        let crop_right = self.frame_crop_right_offset * sub_width_x;
        let crop_top = self.frame_crop_top_offset * sub_height_y;
        let crop_bottom = self.frame_crop_bottom_offset * sub_height_y;
        (
            crop_left,
            crop_top,
            width.saturating_sub(crop_left + crop_right),
            height.saturating_sub(crop_top + crop_bottom),
        )
    }
}

/// 图像参数集（PPS）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H264Pps {
    pub pic_parameter_set_id: u32,
    pub seq_parameter_set_id: u32,
    pub entropy_coding_mode_flag: bool,
    pub bottom_field_pic_order_in_frame_present_flag: bool,
    pub num_slice_groups_minus1: u32,
    pub num_ref_idx_l0_default_active_minus1: u32,
    pub num_ref_idx_l1_default_active_minus1: u32,
    pub weighted_pred_flag: bool,
    pub weighted_bipred_idc: u32,
    pub pic_init_qp_minus26: i32,
    pub pic_init_qs_minus26: i32,
    pub chroma_qp_index_offset: i32,
    pub deblocking_filter_control_present_flag: bool,
    pub constrained_intra_pred_flag: bool,
    pub redundant_pic_cnt_present_flag: bool,
    pub transform_8x8_mode_flag: bool,
    pub pic_scaling_matrix_present_flag: bool,
    pub scaling_list4x4: [[u8; SCALING_LIST_4X4_LEN]; 6],
    pub scaling_list8x8: [[u8; SCALING_LIST_8X8_LEN]; 6],
    pub second_chroma_qp_index_offset: i32,
}

impl Default for H264Pps {
    fn default() -> Self {
        Self {
            pic_parameter_set_id: 0,
            seq_parameter_set_id: 0,
            entropy_coding_mode_flag: false,
            bottom_field_pic_order_in_frame_present_flag: false,
            num_slice_groups_minus1: 0,
            num_ref_idx_l0_default_active_minus1: 0,
            num_ref_idx_l1_default_active_minus1: 0,
            weighted_pred_flag: false,
            weighted_bipred_idc: 0,
            pic_init_qp_minus26: 0,
            pic_init_qs_minus26: 0,
            chroma_qp_index_offset: 0,
            deblocking_filter_control_present_flag: false,
            constrained_intra_pred_flag: false,
            redundant_pic_cnt_present_flag: false,
            transform_8x8_mode_flag: false,
            pic_scaling_matrix_present_flag: false,
            scaling_list4x4: [[16; SCALING_LIST_4X4_LEN]; 6],
            scaling_list8x8: [[16; SCALING_LIST_8X8_LEN]; 6],
            second_chroma_qp_index_offset: 0,
        }
    }
}

/// 切片类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SliceType {
    #[default]
    P,
    B,
    I,
    Sp,
    Si,
}

impl SliceType {
    /// 从 H.264 的 `slice_type` 语法元素（0..10）解析。
    ///
    /// 5..9 是 0..4 的「所有切片同类型」变体，语义上相同。
    pub fn from_raw(value: u32) -> Option<Self> {
        match value {
            0 | 5 => Some(SliceType::P),
            1 | 6 => Some(SliceType::B),
            2 | 7 => Some(SliceType::I),
            3 | 8 => Some(SliceType::Sp),
            4 | 9 => Some(SliceType::Si),
            _ => None,
        }
    }

    /// 是否是 **P** 切片。
    ///
    /// 对应 Chromium `H264SliceHeader::IsPSlice()`（`slice_type % 5 == 0`）。
    /// 注意 SP 切片**不算** P 切片 —— 它在规格里走的是另一条路
    /// （`sp_for_switch_flag` / `slice_qs_delta`），把它混进 P 会让
    /// 「哪些字段存在」的判断整体错位。
    pub fn is_p(self) -> bool {
        matches!(self, SliceType::P)
    }
    /// 是否是 **SP** 切片（对应 Chromium `IsSPSlice()`）。
    pub fn is_sp(self) -> bool {
        matches!(self, SliceType::Sp)
    }
    pub fn is_b(self) -> bool {
        matches!(self, SliceType::B)
    }
    /// 是否是 **I** 切片（对应 Chromium `IsISlice()`）。
    pub fn is_i(self) -> bool {
        matches!(self, SliceType::I)
    }
    /// 是否是 **SI** 切片（对应 Chromium `IsSISlice()`）。
    pub fn is_si(self) -> bool {
        matches!(self, SliceType::Si)
    }
}

/// 切片头。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct H264SliceHeader {
    /// 是否 IDR（来自 NAL 头）。
    pub idr_pic_flag: bool,
    /// `nal_ref_idc`（来自 NAL 头）。
    pub nal_ref_idc: u8,
    pub first_mb_in_slice: u32,
    pub slice_type: SliceType,
    pub pic_parameter_set_id: u32,
    pub frame_num: u32,
    pub field_pic_flag: bool,
    pub bottom_field_flag: bool,
    pub idr_pic_id: u32,
    pub pic_order_cnt_lsb: u32,
    pub delta_pic_order_cnt_bottom: i32,
    pub delta_pic_order_cnt0: i32,
    pub delta_pic_order_cnt1: i32,
    pub redundant_pic_cnt: u32,
    /// B 切片的 `direct_spatial_mv_pred_flag`。
    pub direct_spatial_mv_pred_flag: bool,
    pub num_ref_idx_active_override_flag: bool,
    pub num_ref_idx_l0_active_minus1: u32,
    pub num_ref_idx_l1_active_minus1: u32,
    /// 参考帧列表 0 是否被修改过。
    pub ref_pic_list_modification_flag_l0: bool,
    /// 参考帧列表 1 是否被修改过。
    pub ref_pic_list_modification_flag_l1: bool,
    /// 参考帧列表 0 的修改指令。
    pub ref_list_l0_modifications: Vec<RefPicListModification>,
    /// 参考帧列表 1 的修改指令。
    pub ref_list_l1_modifications: Vec<RefPicListModification>,
    /// 加权预测表里的两个分母（spec 7.4.3.2）。
    pub luma_log2_weight_denom: u32,
    pub chroma_log2_weight_denom: u32,
    /// CABAC 初始化 idc（熵编码模式且非 I / SI 切片时存在）。
    pub cabac_init_idc: u32,
    /// `slice_qp_delta`（spec 7.4.3.4）。
    pub slice_qp_delta: i32,
    pub sp_for_switch_flag: bool,
    /// `slice_qs_delta`（SP / SI 切片）。
    pub slice_qs_delta: i32,
    /// 去块滤波参数（`deblocking_filter_control_present_flag` 时存在）。
    pub disable_deblocking_filter_idc: u32,
    pub slice_alpha_c0_offset_div2: i32,
    pub slice_beta_offset_div2: i32,
    /// 是否自适应引用帧标记（`adaptive_ref_pic_marking_mode_flag`）。
    pub adaptive_ref_pic_marking_mode_flag: bool,
    /// 引用帧标记（MMCO）列表 —— 解码后要用来维护 DPB。
    ///
    /// 滑动窗口模式下是**空的**（规格里没有 MMCO）；自适应模式下是
    /// 一串以 op 0 结尾的操作。
    pub dec_ref_pic_marking: Vec<DecRefPicMarking>,
    /// `no_output_of_prior_pics_flag`。
    pub no_output_of_prior_pics_flag: bool,
    /// `long_term_reference_flag`。
    pub long_term_reference_flag: bool,
}

/// 参考帧列表修改指令（spec 7.4.3.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RefPicListModification {
    /// 0 / 1 = 短期帧，2 = 长期帧，3 = 结束。
    pub modification_of_pic_nums_idc: u32,
    /// idc 0 / 1 时的 `abs_diff_pic_num_minus1`。
    pub abs_diff_pic_num_minus1: u32,
    /// idc 2 时的 `long_term_pic_num`。
    pub long_term_pic_num: u32,
}

/// 解码后引用帧标记。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DecRefPicMarking {
    /// 0 = 滑动窗口，1 = 自适应（MMCO）。
    pub memory_management_control_operation: u32,
    pub difference_of_pic_nums_minus1: u32,
    pub long_term_pic_num: u32,
    pub long_term_frame_idx: u32,
    pub max_long_term_frame_idx_plus1: u32,
}

/// 一个切片（NAL 单元 + 解析出的切片头）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H264Slice {
    pub nalu: H264Nalu,
    pub header: H264SliceHeader,
}

/// H.264 解析器：持有当前生效的参数集。
#[derive(Debug, Default)]
pub struct H264Parser {
    active_sps: HashMap<u32, H264Sps>,
    active_pps: HashMap<u32, H264Pps>,
}

impl H264Parser {
    pub fn new() -> Self {
        Self::default()
    }

    /// 丢弃所有参数集（对应「新流开始」）。
    pub fn reset(&mut self) {
        self.active_sps.clear();
        self.active_pps.clear();
    }

    pub fn get_sps(&self, id: u32) -> Option<&H264Sps> {
        self.active_sps.get(&id)
    }

    pub fn get_pps(&self, id: u32) -> Option<&H264Pps> {
        self.active_pps.get(&id)
    }

    /// 解析一个 Annex-B 码流，返回其中所有切片的（NAL + 切片头）。
    ///
    /// 参数集在解析过程中被记录进解析器（`get_sps` / `get_pps` 可查）。
    pub fn parse_stream(&mut self, stream: &[u8]) -> Result<Vec<H264Slice>, H264ParseError> {
        let nalus = super::nal::parse_nalus(stream);
        let mut slices = Vec::new();
        for nalu in nalus {
            match nalu.nal_unit_type {
                NalUnitType::Sps => {
                    self.parse_sps(&nalu)?;
                }
                NalUnitType::Pps => {
                    self.parse_pps(&nalu)?;
                }
                NalUnitType::Aud | NalUnitType::SeiMessage | NalUnitType::Filler => {}
                t if t.is_vcl() => {
                    let header = self.parse_slice_header(&nalu)?;
                    slices.push(H264Slice { nalu, header });
                }
                _ => {}
            }
        }
        Ok(slices)
    }

    /// 解析 SPS，返回它的 id。
    pub fn parse_sps(&mut self, nalu: &H264Nalu) -> Result<u32, H264ParseError> {
        let sps = parse_sps_payload(&nalu.data)?;
        let id = sps.seq_parameter_set_id;
        // 同 id 的旧 SPS 被替换；依赖它的 PPS 也一并作废（校验关系已变）。
        self.active_sps.insert(id, sps);
        self.active_pps
            .retain(|_, pps| pps.seq_parameter_set_id != id);
        Ok(id)
    }

    /// 解析 PPS，返回它的 id。
    pub fn parse_pps(&mut self, nalu: &H264Nalu) -> Result<u32, H264ParseError> {
        // PPS 的前两个语法元素是它自己的 id 与它引用的 SPS id。
        // 必须**先**读出 SPS id 才能找到 SPS —— 用 PPS id 去查 SPS 是错的
        // （两者只是恰好常常都是 0，于是错误被藏住了）。
        let (pps_id, sps_id) = peek_pps_refs(&nalu.data)?;
        let sps = self
            .active_sps
            .get(&sps_id)
            .ok_or(H264ParseError::InvalidStream)?;
        let pps = parse_pps_payload(&nalu.data, sps)?;
        self.active_pps.insert(pps_id, pps);
        Ok(pps_id)
    }

    /// 解析切片头。
    pub fn parse_slice_header(&self, nalu: &H264Nalu) -> Result<H264SliceHeader, H264ParseError> {
        // 先把 PPS / SPS 找出来（切片头要靠它们才能解析）。
        let (pps_id, _) = peek_slice_refs(&nalu.data)?;
        let pps = self
            .active_pps
            .get(&pps_id)
            .ok_or(H264ParseError::InvalidStream)?;
        let sps = self
            .active_sps
            .get(&pps.seq_parameter_set_id)
            .ok_or(H264ParseError::InvalidStream)?;
        parse_slice_header_payload(&nalu.data, sps, pps)
    }

    /// 按**图像**分组 NAL —— 真正的访问单元切分。
    ///
    /// 与 [`super::nal::split_access_units`] 的区别：那一侧只看 NAL 头，
    /// 分不出「同一图像的第二个切片」与「下一张图像的第一个切片」；
    /// 这一侧会解析切片头，用 `first_mb_in_slice == 0` 判定图像边界
    /// （spec 7.4.3：一幅图像的第一个切片 `first_mb_in_slice` 为 0，
    /// 后续切片的值大于 0）。
    ///
    /// 解析切片头失败时**保守**地当作新图像的起点 —— 宁可多切，
    /// 也不能把两张图像合成一个访问单元（那会让硬件解码器拿到
    /// 一帧里混着两幅画的数据）。
    pub fn split_access_units(&self, nalus: &[H264Nalu]) -> Vec<Vec<H264Nalu>> {
        let mut units: Vec<Vec<H264Nalu>> = Vec::new();
        let mut current: Vec<H264Nalu> = Vec::new();
        let mut pending: Vec<H264Nalu> = Vec::new();
        for nalu in nalus {
            if !nalu.nal_unit_type.is_vcl() {
                // 参数集 / SEI 属于**它后面**那个访问单元。
                pending.push(nalu.clone());
                continue;
            }
            let starts_new_picture = match self.parse_slice_header(nalu) {
                Ok(header) => header.first_mb_in_slice == 0,
                Err(_) => true,
            };
            if starts_new_picture && current.iter().any(|n| n.nal_unit_type.is_vcl()) {
                units.push(std::mem::take(&mut current));
            }
            current.append(&mut pending);
            current.push(nalu.clone());
        }
        if !current.is_empty() {
            units.push(current);
        }
        units
    }
}

/// 从 PPS 负载里读出 `pic_parameter_set_id` 与它引用的 `seq_parameter_set_id`。
fn peek_pps_refs(data: &[u8]) -> Result<(u32, u32), H264ParseError> {
    let payload = strip_nal_header(data)?;
    let mut br = H264BitReader::new(payload).ok_or(H264ParseError::InvalidStream)?;
    let pps_id = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    let sps_id = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    Ok((pps_id, sps_id))
}

/// 从切片负载里读出 `pic_parameter_set_id`。
///
/// SPS id **不在这里**解析：它藏在 PPS 里，而 PPS 要先按 id 查表才能拿到。
fn peek_slice_refs(data: &[u8]) -> Result<(u32, u32), H264ParseError> {
    let mut br = H264BitReader::new(&data[1..]).ok_or(H264ParseError::InvalidStream)?;
    let _first_mb_in_slice = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    let slice_type = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if slice_type >= 10 {
        return Err(H264ParseError::InvalidStream);
    }
    let pps_id = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    Ok((pps_id, pps_id))
}

// —— 默认缩放列表（spec Table 7-2 / 8-5）——

const DEFAULT_4X4_INTRA: [u8; SCALING_LIST_4X4_LEN] = [
    6, 13, 13, 20, 20, 20, 28, 28, 28, 28, 32, 32, 32, 37, 37, 42,
];
const DEFAULT_4X4_INTER: [u8; SCALING_LIST_4X4_LEN] = [
    10, 14, 14, 20, 20, 20, 24, 24, 24, 24, 27, 27, 27, 30, 30, 34,
];
const DEFAULT_8X8_INTRA: [u8; SCALING_LIST_8X8_LEN] = [
    6, 10, 10, 13, 11, 13, 16, 16, //
    16, 16, 18, 18, 18, 18, 18, 23, //
    23, 23, 23, 23, 23, 25, 25, 25, //
    25, 25, 25, 25, 27, 27, 27, 27, //
    27, 27, 27, 29, 29, 29, 29, 29, //
    29, 29, 31, 31, 31, 31, 31, 31, //
    31, 33, 33, 33, 33, 33, 36, 36, //
    36, 36, 38, 38, 38, 40, 40, 42,
];
const DEFAULT_8X8_INTER: [u8; SCALING_LIST_8X8_LEN] = [
    9, 13, 13, 15, 13, 15, 17, 17, //
    17, 17, 19, 19, 19, 19, 19, 21, //
    21, 21, 21, 21, 21, 22, 22, 22, //
    22, 22, 22, 24, 24, 24, 24, 24, //
    24, 24, 24, 25, 25, 25, 25, 25, //
    25, 25, 27, 27, 27, 27, 27, 27, //
    27, 28, 28, 28, 28, 28, 30, 30, //
    30, 30, 32, 32, 32, 33, 33, 35,
];

fn default_scaling_list4x4(i: usize) -> [u8; SCALING_LIST_4X4_LEN] {
    if i < 3 {
        DEFAULT_4X4_INTRA
    } else {
        DEFAULT_4X4_INTER
    }
}

fn default_scaling_list8x8(i: usize) -> [u8; SCALING_LIST_8X8_LEN] {
    if i.is_multiple_of(2) {
        DEFAULT_8X8_INTRA
    } else {
        DEFAULT_8X8_INTER
    }
}

/// Table 7-2 的 fallback 规则 A/B（4×4）。
fn fallback_scaling_list4x4(
    i: usize,
    default_intra: [u8; SCALING_LIST_4X4_LEN],
    default_inter: [u8; SCALING_LIST_4X4_LEN],
    lists: &mut [[u8; SCALING_LIST_4X4_LEN]; 6],
) {
    match i {
        0 => lists[0] = default_intra,
        1 => lists[1] = lists[0],
        2 => lists[2] = lists[1],
        3 => lists[3] = default_inter,
        4 => lists[4] = lists[3],
        5 => lists[5] = lists[4],
        _ => {}
    }
}

/// Table 7-2 的 fallback 规则 A/B（8×8）。
fn fallback_scaling_list8x8(
    i: usize,
    default_intra: [u8; SCALING_LIST_8X8_LEN],
    default_inter: [u8; SCALING_LIST_8X8_LEN],
    lists: &mut [[u8; SCALING_LIST_8X8_LEN]; 6],
) {
    match i {
        0 => lists[0] = default_intra,
        1 => lists[1] = default_inter,
        2 => lists[2] = lists[0],
        3 => lists[3] = lists[1],
        4 => lists[4] = lists[2],
        5 => lists[5] = lists[3],
        _ => {}
    }
}

/// 解析一个缩放列表（spec 7.3.2.1.1.1）。
///
/// 返回 `Ok(None)` 表示「首个元素为 0 ⇒ 用默认列表」。
fn parse_scaling_list(
    br: &mut H264BitReader<'_>,
    length: usize,
) -> Result<Option<Vec<u8>>, H264ParseError> {
    let mut last_scale: i32 = 8;
    let mut next_scale: i32 = 8;
    let mut out = vec![0u8; length];
    for (j, slot) in out.iter_mut().enumerate() {
        if next_scale != 0 {
            let delta = br.read_se().ok_or(H264ParseError::InvalidStream)?;
            if !(-128..=127).contains(&delta) {
                return Err(H264ParseError::InvalidStream);
            }
            next_scale = (last_scale + delta + 256) & 0xff;
            if j == 0 && next_scale == 0 {
                return Ok(None);
            }
        }
        *slot = if next_scale == 0 {
            last_scale as u8
        } else {
            next_scale as u8
        };
        last_scale = i32::from(*slot);
    }
    Ok(Some(out))
}

/// 解析 SPS 负载（NAL 头之后的部分）。
// 解析器是**边读位流边填字段**的：一个字段的值依赖前面读到的值，
// 所以先 `Default::default()` 再逐个赋值是这里的自然写法 ——
// 改成一个大结构体字面量只会把「读的顺序」藏进初始化器里，更难读。
#[allow(clippy::field_reassign_with_default)]
pub fn parse_sps_payload(data: &[u8]) -> Result<H264Sps, H264ParseError> {
    let payload = strip_nal_header(data)?;
    let mut br = H264BitReader::new(payload).ok_or(H264ParseError::InvalidStream)?;
    let mut sps = H264Sps::default();

    sps.profile_idc = br.read_bits(8).ok_or(H264ParseError::InvalidStream)? as u8;
    sps.constraint_set0_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    sps.constraint_set1_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    sps.constraint_set2_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    sps.constraint_set3_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    sps.constraint_set4_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    sps.constraint_set5_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    let _reserved_zero_2bits = br.read_bits(2).ok_or(H264ParseError::InvalidStream)?;
    sps.level_idc = br.read_bits(8).ok_or(H264ParseError::InvalidStream)? as u8;
    sps.seq_parameter_set_id = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if sps.seq_parameter_set_id >= 32 {
        return Err(H264ParseError::InvalidStream);
    }

    // 只有「高」profile 族才有 chroma_format_idc 等字段。
    let high_profile = matches!(
        sps.profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128
    );
    if high_profile {
        sps.chroma_format_idc = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        if sps.chroma_format_idc >= 4 {
            return Err(H264ParseError::InvalidStream);
        }
        if sps.chroma_format_idc == 3 {
            sps.separate_colour_plane_flag =
                br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        }
        sps.bit_depth_luma_minus8 = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        if sps.bit_depth_luma_minus8 >= 7 {
            return Err(H264ParseError::InvalidStream);
        }
        sps.bit_depth_chroma_minus8 = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        if sps.bit_depth_chroma_minus8 >= 7 {
            return Err(H264ParseError::InvalidStream);
        }
        sps.qpprime_y_zero_transform_bypass_flag =
            br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        sps.seq_scaling_matrix_present_flag =
            br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        if sps.seq_scaling_matrix_present_flag {
            parse_sps_scaling_lists(&mut br, &mut sps)?;
        }
    } else {
        sps.chroma_format_idc = 1;
        // 非高 profile 不带缩放矩阵 ⇒ 全部填平 16。
        sps.scaling_list4x4 = [[16; SCALING_LIST_4X4_LEN]; 6];
        sps.scaling_list8x8 = [[16; SCALING_LIST_8X8_LEN]; 6];
    }

    sps.log2_max_frame_num_minus4 = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if sps.log2_max_frame_num_minus4 >= 13 {
        return Err(H264ParseError::InvalidStream);
    }
    sps.pic_order_cnt_type = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if sps.pic_order_cnt_type >= 3 {
        return Err(H264ParseError::InvalidStream);
    }
    if sps.pic_order_cnt_type == 0 {
        sps.log2_max_pic_order_cnt_lsb_minus4 =
            br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        if sps.log2_max_pic_order_cnt_lsb_minus4 >= 13 {
            return Err(H264ParseError::InvalidStream);
        }
    } else if sps.pic_order_cnt_type == 1 {
        sps.delta_pic_order_always_zero_flag =
            br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        sps.offset_for_non_ref_pic = br.read_se().ok_or(H264ParseError::InvalidStream)?;
        sps.offset_for_top_to_bottom_field = br.read_se().ok_or(H264ParseError::InvalidStream)?;
        sps.num_ref_frames_in_pic_order_cnt_cycle =
            br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        if sps.num_ref_frames_in_pic_order_cnt_cycle >= 255 {
            return Err(H264ParseError::InvalidStream);
        }
        let mut acc: i32 = 0;
        for _ in 0..sps.num_ref_frames_in_pic_order_cnt_cycle {
            let offset = br.read_se().ok_or(H264ParseError::InvalidStream)?;
            acc = acc
                .checked_add(offset)
                .ok_or(H264ParseError::InvalidStream)?;
            sps.offset_for_ref_frame.push(offset);
        }
        sps.expected_delta_per_pic_order_cnt_cycle = acc;
    }

    sps.max_num_ref_frames = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if sps.max_num_ref_frames > 16 {
        return Err(H264ParseError::InvalidStream);
    }
    sps.gaps_in_frame_num_value_allowed_flag =
        br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    sps.pic_width_in_mbs_minus1 = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if sps.pic_width_in_mbs_minus1 > 543 {
        return Err(H264ParseError::InvalidStream);
    }
    sps.pic_height_in_map_units_minus1 = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if sps.pic_height_in_map_units_minus1 > 543 {
        return Err(H264ParseError::InvalidStream);
    }
    sps.frame_mbs_only_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    if !sps.frame_mbs_only_flag {
        sps.mb_adaptive_frame_field_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    }
    sps.direct_8x8_inference_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    sps.frame_cropping_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    if sps.frame_cropping_flag {
        sps.frame_crop_left_offset = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        sps.frame_crop_right_offset = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        sps.frame_crop_top_offset = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        sps.frame_crop_bottom_offset = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    }
    sps.vui_parameters_present_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    if sps.vui_parameters_present_flag {
        parse_vui_parameters(&mut br, &mut sps)?;
    }
    Ok(sps)
}

/// 解析 PPS 负载（NAL 头之后的部分）。
///
/// `sps` 是该 PPS 引用的 SPS —— 解析 PPS **需要**它：缩放矩阵的 fallback
/// 规则以 SPS 的缩放列表为源，而「PPS 后面还有没有更多语法元素」取决于
/// SPS 的 profile。对应 Chromium `H264Parser::ParsePPS()`：SPS 不存在时
/// Chromium 直接返回 `kInvalidStream`，本实现同样不接受 `None`。
#[allow(clippy::field_reassign_with_default)]
pub fn parse_pps_payload(data: &[u8], sps: &H264Sps) -> Result<H264Pps, H264ParseError> {
    let payload = strip_nal_header(data)?;
    let mut br = H264BitReader::new(payload).ok_or(H264ParseError::InvalidStream)?;
    let mut pps = H264Pps::default();

    pps.pic_parameter_set_id = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if pps.pic_parameter_set_id > 255 {
        return Err(H264ParseError::InvalidStream);
    }
    pps.seq_parameter_set_id = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if pps.seq_parameter_set_id >= 32 {
        return Err(H264ParseError::InvalidStream);
    }

    pps.entropy_coding_mode_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    pps.bottom_field_pic_order_in_frame_present_flag =
        br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    pps.num_slice_groups_minus1 = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if pps.num_slice_groups_minus1 > 1 {
        // slice group 未实现（Chromium 同样返回 kUnsupportedStream）。
        return Err(H264ParseError::UnsupportedStream);
    }
    pps.num_ref_idx_l0_default_active_minus1 = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if pps.num_ref_idx_l0_default_active_minus1 >= 32 {
        return Err(H264ParseError::InvalidStream);
    }
    pps.num_ref_idx_l1_default_active_minus1 = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if pps.num_ref_idx_l1_default_active_minus1 >= 32 {
        return Err(H264ParseError::InvalidStream);
    }
    pps.weighted_pred_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    pps.weighted_bipred_idc = br.read_bits(2).ok_or(H264ParseError::InvalidStream)?;
    if pps.weighted_bipred_idc >= 3 {
        return Err(H264ParseError::InvalidStream);
    }
    pps.pic_init_qp_minus26 = br.read_se().ok_or(H264ParseError::InvalidStream)?;
    if !(-26..=25).contains(&pps.pic_init_qp_minus26) {
        return Err(H264ParseError::InvalidStream);
    }
    pps.pic_init_qs_minus26 = br.read_se().ok_or(H264ParseError::InvalidStream)?;
    if !(-26..=25).contains(&pps.pic_init_qs_minus26) {
        return Err(H264ParseError::InvalidStream);
    }
    pps.chroma_qp_index_offset = br.read_se().ok_or(H264ParseError::InvalidStream)?;
    if !(-12..=12).contains(&pps.chroma_qp_index_offset) {
        return Err(H264ParseError::InvalidStream);
    }
    // `second_chroma_qp_index_offset` 在「PPS 后面没有更多语法元素」时
    // 等于 `chroma_qp_index_offset`（spec 7.4.2.2 的缺省语义，
    // Chromium 在读取可选字段前先这样赋初值）。
    pps.second_chroma_qp_index_offset = pps.chroma_qp_index_offset;
    pps.deblocking_filter_control_present_flag =
        br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    pps.constrained_intra_pred_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    pps.redundant_pic_cnt_present_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;

    // Baseline / ConstrainedBaseline / Main 的 PPS 后面没有更多语法元素。
    let baseline_like = matches!(sps.profile_idc, 66 | 77 | 88);
    if !baseline_like && br.has_more_rbsp_data() {
        pps.transform_8x8_mode_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        pps.pic_scaling_matrix_present_flag =
            br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        if pps.pic_scaling_matrix_present_flag {
            parse_pps_scaling_lists(&mut br, sps, &mut pps)?;
        }
        pps.second_chroma_qp_index_offset = br.read_se().ok_or(H264ParseError::InvalidStream)?;
        if !(-12..=12).contains(&pps.second_chroma_qp_index_offset) {
            return Err(H264ParseError::InvalidStream);
        }
    }
    Ok(pps)
}

/// 解析切片头负载（NAL 头之后的部分）。
pub fn parse_slice_header_payload(
    data: &[u8],
    sps: &H264Sps,
    pps: &H264Pps,
) -> Result<H264SliceHeader, H264ParseError> {
    let payload = strip_nal_header(data)?;
    let mut br = H264BitReader::new(payload).ok_or(H264ParseError::InvalidStream)?;
    let mut header = H264SliceHeader {
        idr_pic_flag: false,
        nal_ref_idc: 0,
        first_mb_in_slice: 0,
        slice_type: SliceType::I,
        pic_parameter_set_id: 0,
        frame_num: 0,
        field_pic_flag: false,
        bottom_field_flag: false,
        idr_pic_id: 0,
        pic_order_cnt_lsb: 0,
        delta_pic_order_cnt_bottom: 0,
        delta_pic_order_cnt0: 0,
        delta_pic_order_cnt1: 0,
        redundant_pic_cnt: 0,
        direct_spatial_mv_pred_flag: false,
        num_ref_idx_active_override_flag: false,
        num_ref_idx_l0_active_minus1: 0,
        num_ref_idx_l1_active_minus1: 0,
        ref_pic_list_modification_flag_l0: false,
        ref_pic_list_modification_flag_l1: false,
        ref_list_l0_modifications: Vec::new(),
        ref_list_l1_modifications: Vec::new(),
        luma_log2_weight_denom: 0,
        chroma_log2_weight_denom: 0,
        cabac_init_idc: 0,
        slice_qp_delta: 0,
        sp_for_switch_flag: false,
        slice_qs_delta: 0,
        disable_deblocking_filter_idc: 0,
        slice_alpha_c0_offset_div2: 0,
        slice_beta_offset_div2: 0,
        adaptive_ref_pic_marking_mode_flag: false,
        dec_ref_pic_marking: Vec::new(),
        no_output_of_prior_pics_flag: false,
        long_term_reference_flag: false,
    };

    header.first_mb_in_slice = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    let raw_slice_type = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if raw_slice_type >= 10 {
        return Err(H264ParseError::InvalidStream);
    }
    header.slice_type = SliceType::from_raw(raw_slice_type).ok_or(H264ParseError::InvalidStream)?;
    header.pic_parameter_set_id = br.read_ue().ok_or(H264ParseError::InvalidStream)?;

    // NAL 头里的两个字段决定后面**哪些语法元素存在**：
    // `idr_pic_flag` 决定有没有 `idr_pic_id`，`nal_ref_idc != 0` 决定有没有
    // `dec_ref_pic_marking`。漏读它们不是「少一个字段」，而是后面整段错位。
    let nal_header = data[0];
    header.nal_ref_idc = (nal_header >> 5) & 0x03;
    header.idr_pic_flag = nal_header & 0x1f == 5;

    if sps.separate_colour_plane_flag {
        // 隔行（separate colour plane）不支持。
        return Err(H264ParseError::UnsupportedStream);
    }

    header.frame_num = br
        .read_bits((sps.log2_max_frame_num_minus4 + 4) as usize)
        .ok_or(H264ParseError::InvalidStream)?;
    if !sps.frame_mbs_only_flag {
        header.field_pic_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        if header.field_pic_flag {
            // 隔行场编码不支持。
            return Err(H264ParseError::UnsupportedStream);
        }
    }

    // first_mb_in_slice 必须落在图像范围内（否则会成为解码器的写越界偏移）。
    let pic_size_in_mbs = (sps.pic_width_in_mbs_minus1 + 1)
        * (sps.pic_height_in_map_units_minus1 + 1)
        * (2 - u32::from(sps.frame_mbs_only_flag));
    if pic_size_in_mbs == 0 || header.first_mb_in_slice > pic_size_in_mbs - 1 {
        return Err(H264ParseError::InvalidStream);
    }

    if header.idr_pic_flag {
        header.idr_pic_id = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        if header.idr_pic_id > 65535 {
            return Err(H264ParseError::InvalidStream);
        }
    }

    if sps.pic_order_cnt_type == 0 {
        header.pic_order_cnt_lsb = br
            .read_bits((sps.log2_max_pic_order_cnt_lsb_minus4 + 4) as usize)
            .ok_or(H264ParseError::InvalidStream)?;
        if pps.bottom_field_pic_order_in_frame_present_flag && !header.field_pic_flag {
            header.delta_pic_order_cnt_bottom =
                br.read_se().ok_or(H264ParseError::InvalidStream)?;
        }
    }
    if sps.pic_order_cnt_type == 1 && !sps.delta_pic_order_always_zero_flag {
        header.delta_pic_order_cnt0 = br.read_se().ok_or(H264ParseError::InvalidStream)?;
        if pps.bottom_field_pic_order_in_frame_present_flag && !header.field_pic_flag {
            header.delta_pic_order_cnt1 = br.read_se().ok_or(H264ParseError::InvalidStream)?;
        }
    }

    if pps.redundant_pic_cnt_present_flag {
        header.redundant_pic_cnt = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        if header.redundant_pic_cnt >= 128 {
            return Err(H264ParseError::InvalidStream);
        }
    }

    // B 切片在 num_ref_idx 之前有 `direct_spatial_mv_pred_flag` —— 漏读它
    // 会让后面**所有**字段错位一位，而且症状是「P 帧解出来是花的」，
    // 很难联想到是这一个布尔位。
    if header.slice_type.is_b() {
        header.direct_spatial_mv_pred_flag =
            br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    }

    if header.slice_type.is_p() || header.slice_type.is_sp() || header.slice_type.is_b() {
        header.num_ref_idx_active_override_flag =
            br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        if header.num_ref_idx_active_override_flag {
            header.num_ref_idx_l0_active_minus1 =
                br.read_ue().ok_or(H264ParseError::InvalidStream)?;
            if header.slice_type.is_b() {
                header.num_ref_idx_l1_active_minus1 =
                    br.read_ue().ok_or(H264ParseError::InvalidStream)?;
            }
        } else {
            header.num_ref_idx_l0_active_minus1 = pps.num_ref_idx_l0_default_active_minus1;
            if header.slice_type.is_b() {
                header.num_ref_idx_l1_active_minus1 = pps.num_ref_idx_l1_default_active_minus1;
            }
        }
    }

    // 场编码时活跃参考数可以到 31，帧编码时到 15。
    let max_ref_idx = if header.field_pic_flag { 32 } else { 16 };
    if header.num_ref_idx_l0_active_minus1 >= max_ref_idx
        || header.num_ref_idx_l1_active_minus1 >= max_ref_idx
    {
        return Err(H264ParseError::InvalidStream);
    }

    // 参考帧列表修改（DXVA2 需要它来确定 `RefFrameList` 的顺序）。
    //
    // ⚠️ I / SI 切片**整个 `ref_pic_list_modification()` 都不存在**
    // （spec 7.4.3.1：`slice_type % 5 != 2 && slice_type % 5 != 4` 才有
    // `ref_pic_list_modification_flag_l0`）。无条件读那个标志位会让
    // I 切片的每一个后续字段都错一位。
    if !header.slice_type.is_i() && !header.slice_type.is_si() {
        header.ref_pic_list_modification_flag_l0 =
            br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        if header.ref_pic_list_modification_flag_l0 {
            header.ref_list_l0_modifications =
                parse_ref_pic_list_modification(&mut br, header.num_ref_idx_l0_active_minus1)?;
        }
    }
    if header.slice_type.is_b() {
        header.ref_pic_list_modification_flag_l1 =
            br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        if header.ref_pic_list_modification_flag_l1 {
            header.ref_list_l1_modifications =
                parse_ref_pic_list_modification(&mut br, header.num_ref_idx_l1_active_minus1)?;
        }
    }

    // 加权预测表（同样只消耗、不使用）。
    if pps.weighted_pred_flag && (header.slice_type.is_p() || header.slice_type.is_sp())
        || pps.weighted_bipred_idc == 1 && header.slice_type.is_b()
    {
        parse_pred_weight_table(&mut br, sps, &mut header)?;
    }

    if header.nal_ref_idc != 0 {
        parse_dec_ref_pic_marking(&mut br, &mut header)?;
    }

    // CABAC 初始化 idc：熵编码模式且**不是** I / SI 切片。
    if pps.entropy_coding_mode_flag && !header.slice_type.is_i() && !header.slice_type.is_si() {
        header.cabac_init_idc = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        if header.cabac_init_idc >= 3 {
            return Err(H264ParseError::InvalidStream);
        }
    }

    // slice_qp_delta：SliceQPY = 26 + pic_init_qp_minus26 + slice_qp_delta。
    header.slice_qp_delta = br.read_se().ok_or(H264ParseError::InvalidStream)?;
    let qp_bd_offset_y = 6 * sps.bit_depth_luma_minus8 as i32;
    let base_qp = 26 + pps.pic_init_qp_minus26;
    if header.slice_qp_delta < -qp_bd_offset_y - base_qp || header.slice_qp_delta > 51 - base_qp {
        return Err(H264ParseError::InvalidStream);
    }

    // SP / SI 切片有 `slice_qs_delta`；SP 还多一个 `sp_for_switch_flag`。
    if header.slice_type.is_sp() || header.slice_type.is_si() {
        if header.slice_type.is_sp() {
            header.sp_for_switch_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        }
        header.slice_qs_delta = br.read_se().ok_or(H264ParseError::InvalidStream)?;
        let base_qs = 26 + pps.pic_init_qs_minus26;
        if header.slice_qs_delta < -base_qs || header.slice_qs_delta > 51 - base_qs {
            return Err(H264ParseError::InvalidStream);
        }
    }

    // 去块滤波参数。
    if pps.deblocking_filter_control_present_flag {
        header.disable_deblocking_filter_idc = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        if header.disable_deblocking_filter_idc >= 3 {
            return Err(H264ParseError::InvalidStream);
        }
        if header.disable_deblocking_filter_idc != 1 {
            header.slice_alpha_c0_offset_div2 =
                br.read_se().ok_or(H264ParseError::InvalidStream)?;
            if !(-6..=6).contains(&header.slice_alpha_c0_offset_div2) {
                return Err(H264ParseError::InvalidStream);
            }
            header.slice_beta_offset_div2 = br.read_se().ok_or(H264ParseError::InvalidStream)?;
            if !(-6..=6).contains(&header.slice_beta_offset_div2) {
                return Err(H264ParseError::InvalidStream);
            }
        }
    }

    if pps.num_slice_groups_minus1 > 0 {
        // slice group 未实现（Chromium 同样返回 kUnsupportedStream）。
        return Err(H264ParseError::UnsupportedStream);
    }
    Ok(header)
}

/// 丢掉 NAL 头字节，返回 RBSP 负载。
fn strip_nal_header(data: &[u8]) -> Result<&[u8], H264ParseError> {
    if data.len() < 2 {
        return Err(H264ParseError::InvalidStream);
    }
    Ok(&data[1..])
}

fn parse_sps_scaling_lists(
    br: &mut H264BitReader<'_>,
    sps: &mut H264Sps,
) -> Result<(), H264ParseError> {
    for i in 0..6 {
        let present = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        if present {
            match parse_scaling_list(br, SCALING_LIST_4X4_LEN)? {
                Some(list) => sps.scaling_list4x4[i] = list.try_into().unwrap_or([0; 16]),
                None => sps.scaling_list4x4[i] = default_scaling_list4x4(i),
            }
        } else {
            fallback_scaling_list4x4(
                i,
                DEFAULT_4X4_INTRA,
                DEFAULT_4X4_INTER,
                &mut sps.scaling_list4x4,
            );
        }
    }
    let count = if sps.chroma_format_idc != 3 { 2 } else { 6 };
    for i in 0..count {
        let present = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        if present {
            match parse_scaling_list(br, SCALING_LIST_8X8_LEN)? {
                Some(list) => sps.scaling_list8x8[i] = list.try_into().unwrap_or([0; 64]),
                None => sps.scaling_list8x8[i] = default_scaling_list8x8(i),
            }
        } else {
            fallback_scaling_list8x8(
                i,
                DEFAULT_8X8_INTRA,
                DEFAULT_8X8_INTER,
                &mut sps.scaling_list8x8,
            );
        }
    }
    Ok(())
}

fn parse_pps_scaling_lists(
    br: &mut H264BitReader<'_>,
    sps: &H264Sps,
    pps: &mut H264Pps,
) -> Result<(), H264ParseError> {
    for i in 0..6 {
        let present = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        if present {
            match parse_scaling_list(br, SCALING_LIST_4X4_LEN)? {
                Some(list) => pps.scaling_list4x4[i] = list.try_into().unwrap_or([0; 16]),
                None => pps.scaling_list4x4[i] = default_scaling_list4x4(i),
            }
        } else if !sps.seq_scaling_matrix_present_flag {
            // Table 7-2 fallback rule A。
            fallback_scaling_list4x4(
                i,
                DEFAULT_4X4_INTRA,
                DEFAULT_4X4_INTER,
                &mut pps.scaling_list4x4,
            );
        } else {
            // Table 7-2 fallback rule B。
            let intra = sps.scaling_list4x4[0];
            let inter = sps.scaling_list4x4[3];
            fallback_scaling_list4x4(i, intra, inter, &mut pps.scaling_list4x4);
        }
    }
    if pps.transform_8x8_mode_flag {
        let count = if sps.chroma_format_idc != 3 { 2 } else { 6 };
        for i in 0..count {
            let present = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
            if present {
                match parse_scaling_list(br, SCALING_LIST_8X8_LEN)? {
                    Some(list) => pps.scaling_list8x8[i] = list.try_into().unwrap_or([0; 64]),
                    None => pps.scaling_list8x8[i] = default_scaling_list8x8(i),
                }
            } else if !sps.seq_scaling_matrix_present_flag {
                fallback_scaling_list8x8(
                    i,
                    DEFAULT_8X8_INTRA,
                    DEFAULT_8X8_INTER,
                    &mut pps.scaling_list8x8,
                );
            } else {
                let intra = sps.scaling_list8x8[0];
                let inter = sps.scaling_list8x8[1];
                fallback_scaling_list8x8(i, intra, inter, &mut pps.scaling_list8x8);
            }
        }
    }
    Ok(())
}

/// 解析并忽略 VUI 参数。
///
/// 逐字对应 Chromium 的 `ParseVUIParameters()`：每一项都要**准确消费**位数，
/// 否则后面全错。
fn parse_vui_parameters(
    br: &mut H264BitReader<'_>,
    sps: &mut H264Sps,
) -> Result<(), H264ParseError> {
    const SAR_WIDTH: [u32; 17] = [
        0, 1, 12, 10, 16, 40, 24, 20, 32, 80, 18, 15, 64, 160, 4, 3, 2,
    ];
    const SAR_HEIGHT: [u32; 17] = [
        0, 1, 11, 11, 33, 33, 11, 11, 11, 33, 11, 10, 30, 30, 3, 2, 1,
    ];

    let aspect_ratio_info_present = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    if aspect_ratio_info_present {
        let aspect_ratio_idc = br.read_bits(8).ok_or(H264ParseError::InvalidStream)?;
        if aspect_ratio_idc == 255 {
            sps.sar_width = br.read_bits(16).ok_or(H264ParseError::InvalidStream)?;
            sps.sar_height = br.read_bits(16).ok_or(H264ParseError::InvalidStream)?;
        } else {
            let index = aspect_ratio_idc as usize;
            if index >= SAR_WIDTH.len() {
                return Err(H264ParseError::InvalidStream);
            }
            sps.sar_width = SAR_WIDTH[index];
            sps.sar_height = SAR_HEIGHT[index];
        }
    }

    let overscan_info_present = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    if overscan_info_present {
        let _overscan_appropriate = br.read_bit().ok_or(H264ParseError::InvalidStream)?;
    }

    sps.video_signal_type_present_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    if sps.video_signal_type_present_flag {
        let _video_format = br.read_bits(3).ok_or(H264ParseError::InvalidStream)?;
        sps.video_full_range_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        sps.colour_description_present_flag =
            br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        if sps.colour_description_present_flag {
            sps.colour_primaries = br.read_bits(8).ok_or(H264ParseError::InvalidStream)?;
            sps.transfer_characteristics = br.read_bits(8).ok_or(H264ParseError::InvalidStream)?;
            sps.matrix_coefficients = br.read_bits(8).ok_or(H264ParseError::InvalidStream)?;
        }
    }

    let chroma_loc_info_present = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    if chroma_loc_info_present {
        let _top = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        let _bottom = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    }

    let timing_info_present = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    if timing_info_present {
        let _num_units_in_tick = br.read_bits(16).ok_or(H264ParseError::InvalidStream)?;
        let _time_scale = br.read_bits(16).ok_or(H264ParseError::InvalidStream)?;
        let _fixed_frame_rate = br.read_bit().ok_or(H264ParseError::InvalidStream)?;
    }

    let nal_hrd_present = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    parse_hrd_parameters(br, nal_hrd_present)?;
    let vcl_hrd_present = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    parse_hrd_parameters(br, vcl_hrd_present)?;

    if nal_hrd_present || vcl_hrd_present {
        let _low_delay_hrd_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)?;
    }

    let pic_struct_present = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    if pic_struct_present {
        let _num_bits_in_pic_order_cnt_cycle = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    }

    let bitstream_restriction_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    if bitstream_restriction_flag {
        let _motion_vectors_over_pic_boundaries =
            br.read_bit().ok_or(H264ParseError::InvalidStream)?;
        let _max_bytes_per_pic_denom = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        let _max_bits_per_mb_denom = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        let _log2_max_mv_length_horizontal = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        let _log2_max_mv_length_vertical = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        let _max_num_reorder_frames = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        let _max_dec_frame_buffering = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    }
    Ok(())
}

/// 解析并忽略 HRD 参数（spec E.2.2）。
fn parse_hrd_parameters(br: &mut H264BitReader<'_>, present: bool) -> Result<(), H264ParseError> {
    if !present {
        return Ok(());
    }
    let cpb_cnt_minus1 = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if cpb_cnt_minus1 > 31 {
        return Err(H264ParseError::InvalidStream);
    }
    let _bit_rate_scale = br.read_bits(8).ok_or(H264ParseError::InvalidStream)?;
    let _cpb_size_scale = br.read_bits(8).ok_or(H264ParseError::InvalidStream)?;
    for _ in 0..=cpb_cnt_minus1 {
        let _bit_rate_value_minus1 = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        let _cpb_size_value_minus1 = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        let _cbr_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)?;
    }
    let _initial_cpb_removal_delay_length_minus1 =
        br.read_bits(5).ok_or(H264ParseError::InvalidStream)?;
    let _cpb_removal_delay_length_minus1 = br.read_bits(5).ok_or(H264ParseError::InvalidStream)?;
    let _dpb_output_delay_length_minus1 = br.read_bits(5).ok_or(H264ParseError::InvalidStream)?;
    let _time_offset_length = br.read_bits(5).ok_or(H264ParseError::InvalidStream)?;
    Ok(())
}

/// 解析参考帧列表修改（spec 7.4.3.1）。
///
/// 「结束」由 `modification_of_pic_nums_idc == 3` 表示 —— 它**不是**一个
/// 额外的终止字段，而是列表本身的最后一个元素。对应 Chromium
/// `H264Parser::ParseRefPicListModification()`：读到 idc == 3 就停，
/// 最多 32 个元素；32 个都没遇到 3 时，末尾还必须再有一个 3。
fn parse_ref_pic_list_modification(
    br: &mut H264BitReader<'_>,
    num_ref_idx_active_minus1: u32,
) -> Result<Vec<RefPicListModification>, H264ParseError> {
    if num_ref_idx_active_minus1 >= 32 {
        return Err(H264ParseError::InvalidStream);
    }
    let mut modifications = Vec::new();
    for i in 0..32u32 {
        let idc = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        if idc >= 4 {
            return Err(H264ParseError::InvalidStream);
        }
        match idc {
            0 | 1 => {
                let abs_diff = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
                if abs_diff > 65535 {
                    return Err(H264ParseError::InvalidStream);
                }
                modifications.push(RefPicListModification {
                    modification_of_pic_nums_idc: idc,
                    abs_diff_pic_num_minus1: abs_diff,
                    long_term_pic_num: 0,
                });
            }
            2 => {
                let long_term = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
                if long_term > 15 {
                    return Err(H264ParseError::InvalidStream);
                }
                modifications.push(RefPicListModification {
                    modification_of_pic_nums_idc: idc,
                    abs_diff_pic_num_minus1: 0,
                    long_term_pic_num: long_term,
                });
            }
            3 => {
                // Per spec, list cannot be empty.
                if i == 0 {
                    return Err(H264ParseError::InvalidStream);
                }
                return Ok(modifications);
            }
            _ => return Err(H264ParseError::InvalidStream),
        }
    }
    // 32 次都没遇到结束标记 ⇒ 末尾必须有一个 idc == 3。
    let idc = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if idc != 3 {
        return Err(H264ParseError::InvalidStream);
    }
    Ok(modifications)
}

/// 解析并忽略加权预测表（spec 7.4.3.2）。
///
/// ⚠️ **色度分量是「逐参考索引交错」的**，不是一个循环读完 luma 再读 chroma：
///
/// ```text
/// for (i = 0..=num_ref_idx_l0_active_minus1) {
///     luma_weight_l0_flag[i]   u(1)
///     if (luma_weight_l0_flag[i])   { luma_weight, luma_offset }
///     if (ChromaArrayType != 0) {
///         chroma_weight_l0_flag[i] u(1)
///         if (chroma_weight_l0_flag[i]) { 2×(chroma_weight, chroma_offset) }
///     }
/// }
/// ```
///
/// 把两个循环拆开（先读完所有 luma 再读所有 chroma）会让位流错位，
/// 而且错位后的症状是「解出花屏」，不报错。
fn parse_pred_weight_table(
    br: &mut H264BitReader<'_>,
    sps: &H264Sps,
    header: &mut H264SliceHeader,
) -> Result<(), H264ParseError> {
    header.luma_log2_weight_denom = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
    if header.luma_log2_weight_denom >= 8 {
        return Err(H264ParseError::InvalidStream);
    }
    // `chroma_log2_weight_denom` 的存在条件是 `ChromaArrayType != 0`
    // —— **不是**「切片类型是 B」。P 切片带加权预测时它同样存在。
    let has_chroma = sps.chroma_array_type() != 0;
    if has_chroma {
        header.chroma_log2_weight_denom = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        if header.chroma_log2_weight_denom >= 8 {
            return Err(H264ParseError::InvalidStream);
        }
    }
    parse_weighting_factors(br, header.num_ref_idx_l0_active_minus1, has_chroma)?;
    if header.slice_type.is_b() {
        parse_weighting_factors(br, header.num_ref_idx_l1_active_minus1, has_chroma)?;
    }
    Ok(())
}

/// 读一组加权因子（luma + 可选 chroma），逐参考索引交错。
fn parse_weighting_factors(
    br: &mut H264BitReader<'_>,
    num_ref_idx_active_minus1: u32,
    has_chroma: bool,
) -> Result<(), H264ParseError> {
    for _ in 0..=num_ref_idx_active_minus1 {
        let luma_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)?;
        if luma_flag != 0 {
            read_weighting_factor(br, 1)?;
        }
        if has_chroma {
            let chroma_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)?;
            if chroma_flag != 0 {
                // 两个色度分量各有权重与偏移。
                read_weighting_factor(br, 2)?;
            }
        }
    }
    Ok(())
}

/// 读 `count` 组 (权重, 偏移) 对。
fn read_weighting_factor(br: &mut H264BitReader<'_>, count: u32) -> Result<(), H264ParseError> {
    for _ in 0..count {
        let weight = br.read_se().ok_or(H264ParseError::InvalidStream)?;
        if !(-128..=127).contains(&weight) {
            return Err(H264ParseError::InvalidStream);
        }
        let offset = br.read_se().ok_or(H264ParseError::InvalidStream)?;
        if !(-128..=127).contains(&offset) {
            return Err(H264ParseError::InvalidStream);
        }
    }
    Ok(())
}

/// 解析并记录解码后引用帧标记（spec 7.4.3.3）。
///
/// 对应 Chromium `H264Parser::ParseDecRefPicMarking()`。注意 MMCO 操作码
/// **可以同时带多个参数**：op 3 既带 `difference_of_pic_nums_minus1` 又带
/// `long_term_frame_idx` —— 漏读后者会让后续位错位。
fn parse_dec_ref_pic_marking(
    br: &mut H264BitReader<'_>,
    header: &mut H264SliceHeader,
) -> Result<(), H264ParseError> {
    if header.idr_pic_flag {
        header.no_output_of_prior_pics_flag =
            br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        header.long_term_reference_flag = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
        return Ok(());
    }
    let adaptive = br.read_bit().ok_or(H264ParseError::InvalidStream)? != 0;
    header.adaptive_ref_pic_marking_mode_flag = adaptive;
    if !adaptive {
        // 滑动窗口（spec 7.4.3.3 的 adaptive_ref_pic_marking_mode_flag == 0）。
        return Ok(());
    }
    // Chromium 同样给循环次数封顶（`ref_pic_marking` 数组的长度）。
    for _ in 0..32 {
        let op = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
        let mut marking = DecRefPicMarking {
            memory_management_control_operation: op,
            ..DecRefPicMarking::default()
        };
        match op {
            0 => {
                header.dec_ref_pic_marking.push(marking);
                return Ok(());
            }
            1 | 3 => {
                marking.difference_of_pic_nums_minus1 =
                    br.read_ue().ok_or(H264ParseError::InvalidStream)?;
                if marking.difference_of_pic_nums_minus1 > 65535 {
                    return Err(H264ParseError::InvalidStream);
                }
                if op == 3 {
                    marking.long_term_frame_idx =
                        br.read_ue().ok_or(H264ParseError::InvalidStream)?;
                    if marking.long_term_frame_idx > 15 {
                        return Err(H264ParseError::InvalidStream);
                    }
                }
            }
            2 => {
                marking.long_term_pic_num = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
                if marking.long_term_pic_num > 15 {
                    return Err(H264ParseError::InvalidStream);
                }
            }
            4 => {
                marking.max_long_term_frame_idx_plus1 =
                    br.read_ue().ok_or(H264ParseError::InvalidStream)?;
                if marking.max_long_term_frame_idx_plus1 > 16 {
                    return Err(H264ParseError::InvalidStream);
                }
            }
            5 => {}
            6 => {
                marking.long_term_frame_idx = br.read_ue().ok_or(H264ParseError::InvalidStream)?;
                if marking.long_term_frame_idx > 15 {
                    return Err(H264ParseError::InvalidStream);
                }
            }
            _ => return Err(H264ParseError::InvalidStream),
        }
        header.dec_ref_pic_marking.push(marking);
    }
    Err(H264ParseError::UnsupportedStream)
}

#[cfg(test)]
#[path = "../../tests/unit/h264_parser_tests.rs"]
mod tests;
