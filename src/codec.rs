//! 编码格式词汇：编解码器、profile、level、色度采样。
//!
//! 判别值与 Chromium `media/base/video_codecs.h` **逐个对齐** —— 那些值会被
//! 写进日志与 UMA 直方图，改名可以、改值不行。

/// 视频编解码器（对应 Chromium `VideoCodec`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum VideoCodec {
    Unknown = 0,
    H264 = 1,
    VC1 = 2,
    MPEG2 = 3,
    MPEG4 = 4,
    Theora = 5,
    VP8 = 6,
    VP9 = 7,
    HEVC = 8,
    DolbyVision = 9,
    AV1 = 10,
    /// AVS3（UAVS3D）。Chromium 的枚举里没有它 —— 这是本 crate 的扩展，
    /// 取第一个空闲判别值，避免动任何已有值（那些会进日志与直方图）。
    Av3 = 11,
    /// VVC / H.266。本 crate 的扩展，沿用空闲判别值。
    Vvc = 13,
    /// EVC（MPEG-5 Part 1）。本 crate 的扩展，沿用空闲判别值。
    Evc = 14,
    /// MJPEG。Chromium 的 VideoCodec 没有它（走 FFmpeg），本 crate 的扩展。
    Mjpeg = 15,
}

impl VideoCodec {
    /// 该编解码器是否走 H.264 那一套位流约定（AVCC / Annex-B、参数集）。
    pub const fn is_h264_family(self) -> bool {
        matches!(self, VideoCodec::H264)
    }

    /// 人类可读名（用于日志与错误信息）。
    pub const fn as_str(self) -> &'static str {
        match self {
            VideoCodec::Unknown => "unknown",
            VideoCodec::H264 => "H264",
            VideoCodec::VC1 => "VC1",
            VideoCodec::MPEG2 => "MPEG2",
            VideoCodec::MPEG4 => "MPEG4",
            VideoCodec::Theora => "Theora",
            VideoCodec::VP8 => "VP8",
            VideoCodec::VP9 => "VP9",
            VideoCodec::HEVC => "HEVC",
            VideoCodec::DolbyVision => "DolbyVision",
            VideoCodec::AV1 => "AV1",
            VideoCodec::Av3 => "AVS3",
            VideoCodec::Vvc => "VVC",
            VideoCodec::Evc => "EVC",
            VideoCodec::Mjpeg => "MJPEG",
        }
    }
}

/// 编解码器的 profile。判别值与 Chromium `VideoCodecProfile` 对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum VideoCodecProfile {
    Unknown = -1,
    /// H.264 Baseline。
    H264Baseline = 0,
    H264Main = 1,
    H264Extended = 2,
    H264High = 3,
    H264High10 = 4,
    H264High422 = 5,
    H264High444Predictive = 6,
    H264ScalableBaseline = 7,
    H264ScalableHigh = 8,
    H264StereoHigh = 9,
    H264MultiviewHigh = 10,
    /// VP8 没有 profile 之分。
    VP8Any = 11,
    VP9Profile0 = 12,
    VP9Profile1 = 13,
    VP9Profile2 = 14,
    VP9Profile3 = 15,
    HEVCMain = 16,
    HEVCMain10 = 17,
    HEVCMainStillPicture = 18,
    DolbyVisionProfile0 = 19,
    DolbyVisionProfile5 = 21,
    DolbyVisionProfile7 = 22,
    TheoraAny = 23,
    AV1ProfileMain = 24,
    AV1ProfileHigh = 25,
    AV1ProfilePro = 26,
    DolbyVisionProfile8 = 27,
    DolbyVisionProfile9 = 28,
    HEVCRext = 29,
    HEVCHighThroughput = 30,
    HEVCMultiviewMain = 31,
    HEVCScalableMain = 32,
    HEVC3DMain = 33,
    HEVCScreenExtended = 34,
    HEVCScalableRext = 35,
    HEVCHighThroughputScreenExtended = 36,
    DolbyVisionProfile10 = 37,
    DolbyVisionProfile20 = 38,
    /// VVC Main10 profile（本 crate 只解 8bit，profile 值仍按标准保留）。本 crate 的扩展。
    VvcProfileMain10 = 40,
    /// EVC Baseline profile（xevd 的 `src_base` 路径）。本 crate 的扩展。
    EvcProfileMain = 41,
    /// MJPEG 没有 profile 之分。本 crate 的扩展。
    MjpegAny = 42,
}

impl VideoCodecProfile {
    /// 该 profile 属于哪个编解码器。用于「配置的 profile 与 codec 不匹配」这类校验。
    pub const fn codec(self) -> VideoCodec {
        match self {
            VideoCodecProfile::Unknown => VideoCodec::Unknown,
            VideoCodecProfile::H264Baseline
            | VideoCodecProfile::H264Main
            | VideoCodecProfile::H264Extended
            | VideoCodecProfile::H264High
            | VideoCodecProfile::H264High10
            | VideoCodecProfile::H264High422
            | VideoCodecProfile::H264High444Predictive
            | VideoCodecProfile::H264ScalableBaseline
            | VideoCodecProfile::H264ScalableHigh
            | VideoCodecProfile::H264StereoHigh
            | VideoCodecProfile::H264MultiviewHigh => VideoCodec::H264,
            VideoCodecProfile::VP8Any => VideoCodec::VP8,
            VideoCodecProfile::VP9Profile0
            | VideoCodecProfile::VP9Profile1
            | VideoCodecProfile::VP9Profile2
            | VideoCodecProfile::VP9Profile3 => VideoCodec::VP9,
            VideoCodecProfile::HEVCMain
            | VideoCodecProfile::HEVCMain10
            | VideoCodecProfile::HEVCMainStillPicture
            | VideoCodecProfile::HEVCRext
            | VideoCodecProfile::HEVCHighThroughput
            | VideoCodecProfile::HEVCMultiviewMain
            | VideoCodecProfile::HEVCScalableMain
            | VideoCodecProfile::HEVC3DMain
            | VideoCodecProfile::HEVCScreenExtended
            | VideoCodecProfile::HEVCScalableRext
            | VideoCodecProfile::HEVCHighThroughputScreenExtended => VideoCodec::HEVC,
            VideoCodecProfile::AV1ProfileMain
            | VideoCodecProfile::AV1ProfileHigh
            | VideoCodecProfile::AV1ProfilePro => VideoCodec::AV1,
            VideoCodecProfile::TheoraAny => VideoCodec::Theora,
            VideoCodecProfile::DolbyVisionProfile0
            | VideoCodecProfile::DolbyVisionProfile5
            | VideoCodecProfile::DolbyVisionProfile7
            | VideoCodecProfile::DolbyVisionProfile8
            | VideoCodecProfile::DolbyVisionProfile9
            | VideoCodecProfile::DolbyVisionProfile10
            | VideoCodecProfile::DolbyVisionProfile20 => VideoCodec::DolbyVision,
            VideoCodecProfile::VvcProfileMain10 => VideoCodec::Vvc,
            VideoCodecProfile::EvcProfileMain => VideoCodec::Evc,
            VideoCodecProfile::MjpegAny => VideoCodec::Mjpeg,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            VideoCodecProfile::Unknown => "unknown",
            VideoCodecProfile::H264Baseline => "H264Baseline",
            VideoCodecProfile::H264Main => "H264Main",
            VideoCodecProfile::H264Extended => "H264Extended",
            VideoCodecProfile::H264High => "H264High",
            VideoCodecProfile::H264High10 => "H264High10",
            VideoCodecProfile::H264High422 => "H264High422",
            VideoCodecProfile::H264High444Predictive => "H264High444Predictive",
            VideoCodecProfile::H264ScalableBaseline => "H264ScalableBaseline",
            VideoCodecProfile::H264ScalableHigh => "H264ScalableHigh",
            VideoCodecProfile::H264StereoHigh => "H264StereoHigh",
            VideoCodecProfile::H264MultiviewHigh => "H264MultiviewHigh",
            VideoCodecProfile::VP8Any => "VP8Any",
            VideoCodecProfile::VP9Profile0 => "VP9Profile0",
            VideoCodecProfile::VP9Profile1 => "VP9Profile1",
            VideoCodecProfile::VP9Profile2 => "VP9Profile2",
            VideoCodecProfile::VP9Profile3 => "VP9Profile3",
            VideoCodecProfile::HEVCMain => "HEVCMain",
            VideoCodecProfile::HEVCMain10 => "HEVCMain10",
            VideoCodecProfile::HEVCMainStillPicture => "HEVCMainStillPicture",
            VideoCodecProfile::HEVCRext => "HEVCRext",
            VideoCodecProfile::HEVCHighThroughput => "HEVCHighThroughput",
            VideoCodecProfile::HEVCMultiviewMain => "HEVCMultiviewMain",
            VideoCodecProfile::HEVCScalableMain => "HEVCScalableMain",
            VideoCodecProfile::HEVC3DMain => "HEVC3DMain",
            VideoCodecProfile::HEVCScreenExtended => "HEVCScreenExtended",
            VideoCodecProfile::HEVCScalableRext => "HEVCScalableRext",
            VideoCodecProfile::HEVCHighThroughputScreenExtended => {
                "HEVCHighThroughputScreenExtended"
            }
            VideoCodecProfile::AV1ProfileMain => "AV1ProfileMain",
            VideoCodecProfile::AV1ProfileHigh => "AV1ProfileHigh",
            VideoCodecProfile::AV1ProfilePro => "AV1ProfilePro",
            VideoCodecProfile::TheoraAny => "TheoraAny",
            VideoCodecProfile::DolbyVisionProfile0 => "DolbyVisionProfile0",
            VideoCodecProfile::DolbyVisionProfile5 => "DolbyVisionProfile5",
            VideoCodecProfile::DolbyVisionProfile7 => "DolbyVisionProfile7",
            VideoCodecProfile::DolbyVisionProfile8 => "DolbyVisionProfile8",
            VideoCodecProfile::DolbyVisionProfile9 => "DolbyVisionProfile9",
            VideoCodecProfile::DolbyVisionProfile10 => "DolbyVisionProfile10",
            VideoCodecProfile::DolbyVisionProfile20 => "DolbyVisionProfile20",
            VideoCodecProfile::VvcProfileMain10 => "VvcProfileMain10",
            VideoCodecProfile::EvcProfileMain => "EvcProfileMain",
            VideoCodecProfile::MjpegAny => "MjpegAny",
        }
    }
}

/// 色度采样（对应 Chromium `VideoChromaSampling`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum VideoChromaSampling {
    Unknown = 0,
    /// 4:2:0 —— 色度宽高各是亮度的一半。
    K420 = 1,
    /// 4:2:2 —— 色度高与亮度相同、宽为一半。
    K422 = 2,
    /// 4:4:4 —— 无下采样。
    K444 = 3,
    /// 4:0:0 —— 单色，无色度平面。
    K400 = 4,
}

/// 编解码器 level。Chromium 用结构体表达（level 码 + 位深 + 色度采样）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoCodecLevel {
    /// level 码本身（H.264 的 `level_idc` 等）。
    pub level: u32,
    /// 每分量位数（8 / 10 / 12）。
    pub bit_depth: u8,
    pub chroma_sampling: VideoChromaSampling,
}

impl VideoCodecLevel {
    pub const fn new(level: u32, bit_depth: u8, chroma_sampling: VideoChromaSampling) -> Self {
        Self {
            level,
            bit_depth,
            chroma_sampling,
        }
    }
}

impl Default for VideoCodecLevel {
    fn default() -> Self {
        Self {
            level: 0,
            bit_depth: 8,
            chroma_sampling: VideoChromaSampling::K420,
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/codec_tests.rs"]
mod tests;
