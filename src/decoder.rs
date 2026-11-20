//! 解码器接口。
//!
//! 对应 Chromium `media/base/video_decoder.h` 的 `VideoDecoder`。
//!
//! # 与 Chromium 的形状差异
//!
//! Chromium 用**回调**（`InitCB` / `DecodeCB` / `OutputCB`）表达异步解码。
//! 本 crate 用**同步拉取**：`decode()` 进去一个缓冲，出来若干帧。
//!
//! 理由不是「Rust 不喜欢回调」，而是**本仓没有异步运行时可依赖**：
//! `zephyra-video` 是零依赖的独立 crate，引入 tokio 之类会把它变成
//! 「不是所有项目都需要」的包的反面例子。而同步拉取把「解码器输出帧的
//! 时刻由解码器决定」这件事表达成了「这一次调用返回几帧」—— 语义等价，
//! 且调用方能自己决定要不要把它搬到别的线程上。
//!
//! 代价是**硬件解码的流水线深度由调用方控制**：`max_decode_requests()`
//! 告诉你最多能同时喂几个缓冲。

use crate::buffer::DecoderBuffer;
use crate::config::VideoDecoderConfig;
use crate::frame::VideoFrame;
use crate::status::DecoderStatus;

/// 解码器的种类（用于统计与日志）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoDecoderType {
    Unknown,
    /// 纯软件解码。
    Software,
    /// Windows DXVA2（D3D11 `ID3D11VideoDecoder`）。
    D3D11,
    /// D3D12 视频解码。
    D3D12,
    /// FFmpeg（libavcodec）。
    Ffmpeg,
    /// libvpx（VP8/VP9）。
    Vpx,
    /// libdav1d（AV1）。
    Dav1d,
    /// vendored uavs3d（AVS3 软件解码）。
    Uavs3d,
    /// vendored libde265（HEVC 软件解码）。
    Hevc,
    /// vendored xevd（EVC 软件解码）。
    Xevd,
    /// vendored OpenH264（H.264 软件解码）。
    OpenH264,
    /// vendored vvdec（VVC 软件解码）。
    Vvdec,
    /// vendored libtheora（Theora 软件解码）。
    Theora,
    /// vendored libjpeg-turbo（MJPEG 软件解码）。
    Mjpeg,
}

impl VideoDecoderType {
    pub const fn as_str(self) -> &'static str {
        match self {
            VideoDecoderType::Unknown => "unknown",
            VideoDecoderType::Software => "software",
            VideoDecoderType::D3D11 => "d3d11",
            VideoDecoderType::D3D12 => "d3d12",
            VideoDecoderType::Ffmpeg => "ffmpeg",
            VideoDecoderType::Vpx => "vpx",
            VideoDecoderType::Dav1d => "dav1d",
            VideoDecoderType::Uavs3d => "uavs3d",
            VideoDecoderType::Hevc => "hevc",
            VideoDecoderType::Xevd => "xevd",
            VideoDecoderType::OpenH264 => "openh264",
            VideoDecoderType::Vvdec => "vvdec",
            VideoDecoderType::Theora => "theora",
            VideoDecoderType::Mjpeg => "mjpeg",
        }
    }
}

/// 视频解码器。
pub trait VideoDecoder {
    /// 用给定配置初始化解码器。
    ///
    /// 返回的状态码为 [`crate::status::DecoderStatusCode::Ok`] 表示成功；
    /// 任何其它码都表示失败（并带上说明）。**注意**：Chromium 的
    /// `Initialize()` 是异步的（可能先返回再回调），本 crate 刻意做成同步 ——
    /// 硬件解码器的设备与 profile 协商本来就是同步的
    /// （`CreateVideoDecoder` 会立刻失败或成功），把它拆成异步没有收益。
    fn initialize(&mut self, config: &VideoDecoderConfig, low_delay: bool) -> DecoderStatus;

    /// 解码一个缓冲。
    ///
    /// 返回这一调用产生的帧（硬件解码常常是 0 帧或 1 帧，软件解码在
    /// 低延迟模式下也可能是 0 帧）。流结束缓冲（[`DecoderBuffer::is_end_of_stream`]）
    /// 会把解码器里剩下的帧全部吐出来。
    fn decode(&mut self, buffer: &DecoderBuffer) -> Result<Vec<VideoFrame>, DecoderStatus>;

    /// 复位解码器状态，丢弃所有内部缓冲的帧。
    fn reset(&mut self);

    /// 该解码器是否需要**位流转换**（AVCC ↔ Annex-B）。
    ///
    /// 对应 Chromium `VideoDecoder::NeedsBitstreamConversion()`，基类默认
    /// 是 `false`。DXVA2 后端返回 `true`：它只吃 Annex-B，而容器里通常是 AVCC。
    fn needs_bitstream_conversion(&self) -> bool {
        false
    }

    /// 解码器当前能否在不等待的情况下继续接收缓冲。
    ///
    /// 对应 Chromium `VideoDecoder::CanReadWithoutStalling()`，基类默认 `true`。
    fn can_read_without_stalling(&self) -> bool {
        true
    }

    /// 最多能**并行**解码几个缓冲。
    ///
    /// 对应 Chromium `VideoDecoder::GetMaxDecodeRequests()`，基类默认 1。
    /// DXVA2 后端返回 4（Chromium 的 `D3D11VideoDecoder::GetMaxDecodeRequests()`
    /// 同样是 4）。
    fn max_decode_requests(&self) -> usize {
        1
    }

    /// 输出的帧是否持有**外部资源**（GPU 纹理）。
    ///
    /// 对应 Chromium `VideoDecoder::FramesHoldExternalResources()`，基类默认 `false`。
    fn frames_hold_external_resources(&self) -> bool {
        false
    }

    /// 解码器种类（统计用）。
    fn decoder_type(&self) -> VideoDecoderType;

    /// 该解码器是不是**平台（硬件）**解码器。
    ///
    /// 对应 Chromium `Decoder::IsPlatformDecoder()`。默认 `false`（软件解码）。
    /// 这个答案决定 [`crate::selector::select_decoder_priority`] 给不给它
    /// 去优先级 —— 分界是可见区域高度 360 像素。
    fn is_platform_decoder(&self) -> bool {
        false
    }
}

#[cfg(test)]
#[path = "../tests/unit/decoder_tests.rs"]
mod tests;
