#![deny(rustdoc::broken_intra_doc_links)]
//! `zephyra-video`：视频解码 —— 按 Chromium `media/` 的形状组织的独立 crate。
//!
//! # 为什么是**独立 crate**而不是 `zephyra-core` 的一部分
//!
//! 视频解码不是 UI 词汇。GPUI、egui、iced、Qt、Flutter 没有一个把视频解码
//! 放进自己的框架 —— 那会让每个使用者都被迫装上编解码库，而绝大多数 UI
//! 根本不播视频。放进 `zephyra-core` 核心还会破坏它零依赖这条不变量，
//! 而那正是「迁移缝能被验证」的前提。
//!
//! 所以它是独立的 crate，并且**默认 feature 为空**：`cargo build` 与
//! `cargo test` 不编译任何设备代码。默认构建唯一的依赖是 `bytes`（纯 Rust、
//! 零平台 API），用于给 CPU 平面提供零拷贝 `Bytes` 出口。
//!
//! # 与 Chromium 的对应关系
//!
//! 本 crate 的每一个模块都能在 Chromium 里找到出处：
//!
//! | 本 crate | Chromium |
//! |---|---|
//! | [`config::VideoDecoderConfig`] | `media/base/video_decoder_config.h` |
//! | [`buffer::DecoderBuffer`] | `media/base/decoder_buffer.h` |
//! | [`frame::VideoFrame`] / [`frame::VideoFrameLayout`] | `media/base/video_frame.h` / `video_frame_layout.h` |
//! | [`decoder::VideoDecoder`] | `media/base/video_decoder.h` |
//! | [`selector::DecoderSelector`] | `media/filters/decoder_selector.cc` |
//! | [`h264`] | `media/parsers/h264_parser.{h,cc}` + `media/filters/h264_to_annex_b_bitstream_converter.cc` |
//! | `dxva2`（Windows feature） | `media/gpu/windows/d3d11_video_decoder_wrapper.cc` + `d3d_h264_accelerator.cc` |
//!
//! AVS3 解码不在 Chromium 里：`native/uavs3d/` 是 BSD-3 许可的
//! [uavs3d](https://github.com/uavs3/uavs3) C 源码逐字节副本，由
//! [`uavs3d`] 模块包装成同一个 [`VideoDecoder`] 接口。
//!
//! AV1 / VP9 则照抄 Chromium 的选择：AV1 用 **dav1d**、VP9 用 **libvpx**，
//! 源码是 Chromium `DEPS` 里 pin 的同 commit 副本（配置与源码清单也一并抄来）。
//!
//! ⚠️ **许可证**：本 crate 自己的 Rust 代码是 `MIT OR Apache-2.0`，
//! 但 vendored 的 C/C++ 代码各有许可证：`native/uavs3d/` 与 `native/libvpx/`
//! 是 **BSD-3-Clause**，`native/dav1d/` 是 **BSD-2-Clause**，
//! `native/libde265/` 是 **LGPL-3.0-or-later**（注意：LGPL 的分发义务比 BSD 重）。
//! 各目录都保留了原始 `COPYING` / `LICENSE` / `PATENTS` 与源文件的版权头 ——
//! 分发时必须一并带上。
//!
//! # feature 与选择性编译
//!
//! | feature | 依赖 | 内容 |
//! |---|---|---|
//! | 无（默认） | 无 | 纯逻辑：配置 / 缓冲 / 帧 / 选择器 / H.264 位流 / DXVA2 参数 / AVS3 NAL 切分 |
//! | `dxva2` | `windows`（仅 Windows） | DXVA2 硬件解码（`ID3D11VideoDecoder`） |
//! | `uavs3d` | `cc`（构建期） | vendored uavs3d 的 C 解码器 + FFI（全平台，输出 NV12） |
//! | `av1` | `cc`（构建期） | vendored dav1d + FFI（全平台，输出 8bit I420） |
//! | `vp9` | `cc`（构建期） | vendored libvpx + FFI（全平台，输出 8bit I420） |
//! | `vp8` | `cc`（构建期） | 同一份 libvpx，换 VP8 codec iface（输出 8bit I420） |
//! | `hevc` | `cc`（构建期） | vendored libde265（LGPL-3.0+，输出 8bit I420） |
//! | `evc` | `cc`（构建期） | vendored xevd（BSD-3-Clause，输出 8bit I420） |
//! | `h264` | `cc`（构建期） | vendored OpenH264（BSD-2-Clause，输出 8bit I420） |
//! | `vvc` | `cc`（构建期） | vendored vvdec（Clear BSD，输出 8bit I420） |
//! | `theora` | `cc`（构建期） | vendored libtheora + libogg headers（BSD-3，输出 8bit I420） |
//! | `mjpeg` | `cc`（构建期） | vendored libjpeg-turbo（BSD-3/IJG，输出 8bit I420） |
//! | `mp3` / `opus` / `vorbis` / `flac` | `cc`（构建期） | 音频：vendored minimp3 / libopus / libvorbis / libFLAC（PCM 解码器默认可用，无 feature） |
//! | `dynamic` | `libloading`（运行期） | 把 vendored C 库编成共享库并运行时加载；缺库进 `DynamicLoadReport`，不让进程启动失败 |
//! | `video` / `audio` / `all-codecs` | 上方对应 feature 的并集 | 一键开启软件视频 / 音频 / 全部编解码器（均不含 `dxva2`、`dynamic`） |
//!
//! 后端接线只有一个声明处：仓库根的 `codecs.rs`。模块声明、crate 根
//! re-export、`load_all()` 的加载入口和 build.rs 的导出符号表都从它派生 ——
//! 新增复用现有 vendored 库的 codec 只改那一行加 `Cargo.toml` 的 feature。
//!
//! [`dxva2`] 模块**默认就存在** —— 里面的 DPB 与图片参数是纯逻辑，
//! 没有 GPU 的机器也要能单测它们。`dxva2` feature 只多出
//! `dxva2::decoder`（真正碰 D3D11 的那一半）。
//!
//! # 硬件解码为什么是 DXVA2 而不是 Media Foundation
//!
//! Chromium 在 Windows 上的**硬件**解码走的是 **DXVA2**
//! （`ID3D11VideoDevice::CreateVideoDecoder` +
//! `ID3D11VideoContext::DecoderBeginFrame` / `SubmitDecoderBuffers` / `DecoderEndFrame`），
//! **不是** Media Foundation：Chromium 的 `D3D11VideoDecoder` 直接对着
//! D3D11 视频设备建解码器，输出是 D3D11 纹理，**从不**落到系统内存。
//! MF 的 MFT 是**软件**路径（Chromium 的 Windows 硬件路径不经过它）。
//!
//! 所以本 crate 的 Windows 后端也走 DXVA2：形状与 Chromium 一致，
//! 输出同样是 GPU 纹理。

// 解码器后端的模块声明与 re-export 由 codecs.rs 的单一声明表生成。
// 必须排在 audio / dynamic_loader 之前：那两个模块会用这里定义的宏。
include!("codec_registry.rs");

pub mod audio;
pub mod buffer;
pub mod codec;
pub mod config;
pub mod decoder;
pub mod dxva2;
#[cfg(feature = "dynamic")]
pub mod dynamic_loader;
#[allow(unsafe_code)]
pub mod ffi_util;
pub mod frame;
pub mod geometry;
// h264 / uavs3d 的纯逻辑部分默认构建就在；解码器类型仍由声明表门控。
pub mod h264;
mod macros;
pub mod selector;
pub mod status;
pub mod uavs3d;

pub use audio::{
    AudioBuffer, AudioCodec, AudioDecoder, AudioDecoderConfig, AudioDecoderType, AudioFrame,
    AudioSampleFormat, PcmDecoder,
};
pub use buffer::DecoderBuffer;
pub use codec::{VideoChromaSampling, VideoCodec, VideoCodecLevel, VideoCodecProfile};
pub use config::{
    bt601_color_space, bt709_color_space, level_8bit_420, AlphaMode, BitstreamFormat,
    VideoAspectRatio, VideoColorSpace, VideoDecoderConfig, VideoTransformation,
};
pub use decoder::{VideoDecoder, VideoDecoderType};
#[cfg(feature = "dynamic")]
pub use dynamic_loader::{
    current_report, library_status, load_all, DynamicLoadReport, LoadFailure,
};
pub use frame::{
    ColorPlaneLayout, NativeResourceOwner, VideoFrame, VideoFrameLayout, VideoPixelFormat,
    VideoPlane,
};
pub use geometry::{Rect, Size};
pub use selector::{select_decoder_priority, DecoderPriority, DecoderSelector};
pub use status::{ok, DecoderStatus, DecoderStatusCode};
