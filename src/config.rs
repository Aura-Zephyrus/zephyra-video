//! 解码配置：一条视频流的「是什么、多大、什么颜色、参数集在哪」。
//!
//! 对应 Chromium `media/base/video_decoder_config.h`。字段语义照搬，但去掉了
//! 与本仓无关的两块：`EncryptionScheme`（本仓没有 CDM）与 `HDRMetadata`
//! （本仓没有 HDR 渲染路径）—— 保留它们只会让「这个字段没人读」变成常态。

use crate::codec::{VideoChromaSampling, VideoCodec, VideoCodecLevel, VideoCodecProfile};
use crate::geometry::{Rect, Size};

/// 帧是否带 alpha 通道。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlphaMode {
    HasAlpha,
    IsOpaque,
}

/// 编码方向与画面方向之间的变换（旋转 / 翻转）。
///
/// 对应 Chromium 的 `VideoTransformation`（`media/base/video_transformation.h`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoTransformation {
    None,
    Rotate90,
    Rotate180,
    Rotate270,
    FlipHorizontal,
    FlipVertical,
}

/// 宽高比（容器里解析出来的像素宽高比）。
///
/// 对应 Chromium 的 `VideoAspectRatio`：分子分母各自允许为 0（表示「这一维
/// 不约束」），所以不能用 `f32` —— 0.0 与「未知」分不开。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct VideoAspectRatio {
    pub h_spacing: i32,
    pub v_spacing: i32,
}

impl VideoAspectRatio {
    pub const fn new(h_spacing: i32, v_spacing: i32) -> Self {
        Self {
            h_spacing,
            v_spacing,
        }
    }

    /// 宽高比是否可用于计算显示尺寸。
    pub const fn is_valid(&self) -> bool {
        self.h_spacing > 0 && self.v_spacing > 0
    }

    /// 按显示宽高比把 `visible` 修正成显示尺寸。
    ///
    /// 对应 Chromium `VideoAspectRatio::GetNaturalSize()` 的 **DAR 分支**：
    /// 显示宽高比比像素宽高比「更宽」就加宽、否则加高 —— 它是**放大**，
    /// 不是裁剪，所以结果不会小于传入的尺寸。
    pub fn natural_size(&self, visible: Size) -> Size {
        if !self.is_valid() || visible.is_empty() {
            return visible;
        }
        let mut width = f64::from(visible.width);
        let mut height = f64::from(visible.height);
        let ratio = f64::from(self.h_spacing) / f64::from(self.v_spacing);
        if ratio >= width / height {
            width = height * ratio;
        } else {
            height = width / ratio;
        }
        Size::new(width.round() as u32, height.round() as u32)
    }
}

/// 色彩空间信息。
///
/// 对应 Chromium `media/base/video_color_space.h` 的 `VideoColorSpace`，
/// 只保留与像素格式真正相关的部分：矩阵系数、传输特性、 primaries、是否全范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoColorSpace {
    /// 矩阵系数（BT.601 / BT.709 / BT.2020 …）。见 ITU-T H.273。
    pub matrix_coefficients: u8,
    /// 传输特性（BT.709 / PQ / HLG …）。见 ITU-T H.273。
    pub transfer_characteristics: u8,
    /// 色度 primaries。见 ITU-T H.273。
    pub color_primaries: u8,
    /// 是否全范围（full range 0..255，而非 limited 16..235）。
    pub full_range: bool,
}

impl Default for VideoColorSpace {
    fn default() -> Self {
        // BT.709 limited range —— Web 默认，Chromium 的 `VideoColorSpace` 也用它兜底。
        Self {
            matrix_coefficients: 6,
            transfer_characteristics: 1,
            color_primaries: 1,
            full_range: false,
        }
    }
}

impl VideoColorSpace {
    /// 该色彩空间是否与 BT.709 相同（HD 内容的绝大多数情况）。
    pub const fn is_srgb(&self) -> bool {
        self.matrix_coefficients == 6
            && self.transfer_characteristics == 1
            && self.color_primaries == 1
    }
}

/// 一条视频流的解码配置。
///
/// 字段与语义对应 Chromium `VideoDecoderConfig`；`extra_data` 的约定也照搬：
/// **H.264 且 `extra_data` 非空 ⇒ 码流是 AVCC（长度前缀）格式；为空 ⇒
/// Annex-B（起始码）格式**。这条约定来自 Chromium 的 `FFmpegVideoDecoder`
/// 注释，也是本仓 `h264` 模块区分两种位流的唯一依据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoDecoderConfig {
    codec: VideoCodec,
    profile: VideoCodecProfile,
    level: VideoCodecLevel,
    alpha_mode: AlphaMode,
    color_space: VideoColorSpace,
    transformation: VideoTransformation,
    coded_size: Size,
    visible_rect: Rect,
    natural_size: Size,
    aspect_ratio: VideoAspectRatio,
    extra_data: Vec<u8>,
    encrypted: bool,
}

impl VideoDecoderConfig {
    /// 新建一份配置。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        codec: VideoCodec,
        profile: VideoCodecProfile,
        level: VideoCodecLevel,
        alpha_mode: AlphaMode,
        color_space: VideoColorSpace,
        transformation: VideoTransformation,
        coded_size: Size,
        visible_rect: Rect,
        natural_size: Size,
        extra_data: Vec<u8>,
    ) -> Self {
        Self {
            codec,
            profile,
            level,
            alpha_mode,
            color_space,
            transformation,
            coded_size,
            visible_rect,
            natural_size,
            aspect_ratio: VideoAspectRatio::default(),
            extra_data,
            encrypted: false,
        }
    }

    pub fn codec(&self) -> VideoCodec {
        self.codec
    }
    pub fn profile(&self) -> VideoCodecProfile {
        self.profile
    }
    pub fn set_profile(&mut self, profile: VideoCodecProfile) {
        self.profile = profile;
    }
    pub fn level(&self) -> VideoCodecLevel {
        self.level
    }
    pub fn set_level(&mut self, level: VideoCodecLevel) {
        self.level = level;
    }
    pub fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }
    pub fn color_space(&self) -> VideoColorSpace {
        self.color_space
    }
    pub fn set_color_space(&mut self, color_space: VideoColorSpace) {
        self.color_space = color_space;
    }
    pub fn transformation(&self) -> VideoTransformation {
        self.transformation
    }
    pub fn set_transformation(&mut self, transformation: VideoTransformation) {
        self.transformation = transformation;
    }
    /// 解码后**立刻**得到的帧尺寸（不一定所有像素都有效）。
    pub fn coded_size(&self) -> Size {
        self.coded_size
    }
    pub fn set_coded_size(&mut self, size: Size) {
        self.coded_size = size;
    }
    /// `coded_size` 内真正有图像数据的区域（clean aperture）。
    pub fn visible_rect(&self) -> Rect {
        self.visible_rect
    }
    pub fn set_visible_rect(&mut self, rect: Rect) {
        self.visible_rect = rect;
    }
    /// 显示尺寸（已考虑宽高比）。
    pub fn natural_size(&self) -> Size {
        self.natural_size
    }
    pub fn set_natural_size(&mut self, size: Size) {
        self.natural_size = size;
    }
    pub fn aspect_ratio(&self) -> VideoAspectRatio {
        self.aspect_ratio
    }
    pub fn set_aspect_ratio(&mut self, aspect_ratio: VideoAspectRatio) {
        self.aspect_ratio = aspect_ratio;
    }
    /// 解码器初始化数据（H.264 则是 avcC 记录）。
    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }
    pub fn set_extra_data(&mut self, extra_data: Vec<u8>) {
        self.extra_data = extra_data;
    }
    /// 该流是否可能是加密的（逐个 buffer 判定）。
    pub fn is_encrypted(&self) -> bool {
        self.encrypted
    }
    pub fn set_is_encrypted(&mut self, encrypted: bool) {
        self.encrypted = encrypted;
    }

    /// 配置是否自洽。
    ///
    /// 对应 Chromium `VideoDecoderConfig::IsValidConfig()`：编解码器已知、
    /// coded / natural 尺寸非零，且 visible_rect **完全落在** coded 区域内。
    /// 最后一条是 Chromium 用 `gfx::Rect(coded_size_).Contains(visible_rect_)`
    /// 表达的 —— 越界的 visible_rect 会让解码器解出错误的裁剪。
    pub fn is_valid_config(&self) -> bool {
        self.codec != VideoCodec::Unknown
            && !self.coded_size.is_empty()
            && !self.natural_size.is_empty()
            && Rect::new(0, 0, self.coded_size.width, self.coded_size.height)
                .contains(&self.visible_rect)
    }

    /// 两份配置是否完全一致（`extra_data` 按内容比，不按指针）。
    ///
    /// 对应 Chromium `VideoDecoderConfig::Matches()`。
    pub fn matches(&self, other: &VideoDecoderConfig) -> bool {
        self.codec == other.codec
            && self.profile == other.profile
            && self.alpha_mode == other.alpha_mode
            && self.transformation == other.transformation
            && self.coded_size == other.coded_size
            && self.visible_rect == other.visible_rect
            && self.natural_size == other.natural_size
            && self.aspect_ratio == other.aspect_ratio
            && self.extra_data == other.extra_data
            && self.encrypted == other.encrypted
            && self.color_space == other.color_space
            && self.level == other.level
    }

    /// 该配置的位流是 AVCC（长度前缀）还是 Annex-B（起始码）。
    ///
    /// 规则与 Chromium 一致：**有 `extra_data` ⇒ AVCC**。空 `extra_data` 的
    /// H.264 流按 Annex-B 处理（参数集得在流里）。
    pub fn bitstream_format(&self) -> BitstreamFormat {
        if self.codec.is_h264_family() {
            if self.extra_data.is_empty() {
                BitstreamFormat::AnnexB
            } else {
                BitstreamFormat::Avcc
            }
        } else {
            BitstreamFormat::Unknown
        }
    }

    /// 按 `aspect_ratio` 修正后的显示尺寸。
    ///
    /// 宽高比有效时按 **visible_rect** 放大（Chromium 的 DAR 语义）；
    /// 否则退回配置里给的 `natural_size`。
    pub fn display_size(&self) -> Size {
        if self.aspect_ratio.is_valid() {
            self.aspect_ratio.natural_size(self.visible_rect.size())
        } else {
            self.natural_size
        }
    }

    /// 人类可读描述（对应 Chromium `AsHumanReadableString()`）。
    pub fn as_human_readable_string(&self) -> String {
        format!(
            "codec={} profile={} level={} coded={}x{} visible={},{},{}x{} natural={}x{} extra_data={}B",
            self.codec.as_str(),
            self.profile.as_str(),
            self.level.level,
            self.coded_size.width,
            self.coded_size.height,
            self.visible_rect.x,
            self.visible_rect.y,
            self.visible_rect.width,
            self.visible_rect.height,
            self.natural_size.width,
            self.natural_size.height,
            self.extra_data.len(),
        )
    }
}

/// 位流的两种封装格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BitstreamFormat {
    /// 未知（该编解码器不区分）。
    Unknown,
    /// AVCC：NAL 用长度前缀（`extra_data` 里带 avcC）。
    Avcc,
    /// Annex-B：NAL 用 00 00 01 / 00 00 00 01 起始码。
    AnnexB,
}

/// 便捷构造：常用色彩空间。
pub fn bt709_color_space() -> VideoColorSpace {
    VideoColorSpace::default()
}

/// 便捷构造：BT.601 limited range（标清内容）。
pub fn bt601_color_space() -> VideoColorSpace {
    VideoColorSpace {
        matrix_coefficients: 6,
        transfer_characteristics: 1,
        color_primaries: 1,
        full_range: false,
    }
}

/// 便捷构造：色度采样对应的位深（8 bit）。
pub fn level_8bit_420() -> VideoCodecLevel {
    VideoCodecLevel::new(0, 8, VideoChromaSampling::K420)
}

#[cfg(test)]
#[path = "../tests/unit/config_tests.rs"]
mod tests;
