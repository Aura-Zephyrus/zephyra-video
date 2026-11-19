//! DXVA2：Windows 上的 H.264 硬件解码。
//!
//! # 与 Chromium 的对应关系
//!
//! | 本 crate | Chromium |
//! |---|---|
//! | [`dpb`] | `media/gpu/h264_dpb.{h,cc}`（`H264DPB`） |
//! | [`pic_params`] | `media/gpu/windows/d3d_h264_accelerator.cc` |
//! | `decoder`（Windows + `dxva2` feature） | `media/gpu/windows/d3d11_video_decoder_wrapper.cc` |
//!
//! # 为什么分两半
//!
//! [`dpb`] 与 [`pic_params`] 是**纯逻辑**：拼参考帧列表、拼
//! `DXVA_PicParams_H264` 的字节。它们没有 unsafe，在**任何**平台上都能
//! 单测 —— 字节布局错了这件事，必须在没有 GPU 的机器上就能被抓住，
//! 而不是等到某台机器的驱动拒绝解码时才暴露。
//!
//! `decoder` 才是碰 D3D11 的那一半：建设备、协商 profile、提交缓冲、
//! 产出纹理。它只在 Windows 且开了 `dxva2` feature 时才编译。
//!
//! # 解码流程（与 Chromium 同款）
//!
//! ```text
//! Annex-B 码流
//!   → H264Parser（切访问单元、解析切片头）
//!   → DecodedPictureBuffer（算 POC、维护 DPB）
//!   → DXVA_PicParams_H264（喂给驱动）
//!   → ID3D11VideoContext::DecoderBeginFrame / SubmitDecoderBuffers / DecoderEndFrame
//!   → ID3D11Texture2D（NV12，永不落到系统内存）
//! ```

pub mod dpb;
pub mod pic_params;

#[cfg(all(feature = "dxva2", windows))]
#[allow(unsafe_code)]
pub mod decoder;

#[cfg(all(feature = "dxva2", windows))]
pub use decoder::Dxva2H264Decoder;
