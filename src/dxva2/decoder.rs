//! Windows DXVA2 硬件解码（D3D11 视频解码器）。
//!
//! 对应 Chromium `media/gpu/windows/d3d11_video_decoder_wrapper.cc`
//! + `d3d_h264_accelerator.cc`。
//!
//! # unsafe 的边界
//!
//! 本模块是整个 crate 里**唯一**碰系统 API 的地方，因此也是唯一允许
//! `unsafe` 的地方（crate 级 `unsafe_code = "deny"`，这里局部开口）。
//! unsafe 只出现在三类地方：
//!
//! 1. 调用 COM 方法（`windows` crate 的绑定全部是 `unsafe fn`）；
//! 2. 把驱动借给我们的缓冲指针变成 Rust 切片；
//! 3. 把 Rust 结构体按字节写进驱动缓冲。
//!
//! 其余全部是安全 Rust。

use std::ffi::c_void;
use std::sync::Arc;

use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1,
    D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_9_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, ID3D11VideoContext,
    ID3D11VideoDecoder, ID3D11VideoDecoderOutputView, ID3D11VideoDevice, D3D11_BIND_DECODER,
    D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_DECODER_PROFILE_H264_VLD_NOFGT, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11_VDOV_DIMENSION_TEXTURE2D, D3D11_VIDEO_DECODER_BUFFER_BITSTREAM,
    D3D11_VIDEO_DECODER_BUFFER_DESC, D3D11_VIDEO_DECODER_BUFFER_INVERSE_QUANTIZATION_MATRIX,
    D3D11_VIDEO_DECODER_BUFFER_PICTURE_PARAMETERS, D3D11_VIDEO_DECODER_BUFFER_SLICE_CONTROL,
    D3D11_VIDEO_DECODER_BUFFER_TYPE, D3D11_VIDEO_DECODER_CONFIG, D3D11_VIDEO_DECODER_DESC,
    D3D11_VIDEO_DECODER_OUTPUT_VIEW_DESC,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};

use crate::buffer::DecoderBuffer;
use crate::codec::VideoCodec;
use crate::config::VideoDecoderConfig;
use crate::decoder::{VideoDecoder, VideoDecoderType};
use crate::dxva2::dpb::{DecodedPicture, DecodedPictureBuffer};
use crate::dxva2::pic_params::{
    build_pic_params, build_qmatrix, DxvaPicParamsH264, DxvaQmatrixH264, DxvaSliceH264Short,
    PicParamsContext, MAX_REF_FRAMES,
};
use crate::frame::{NativeResourceOwner, VideoFrame, VideoPixelFormat};
use crate::geometry::{Rect, Size};
use crate::h264::parser::{H264Pps, H264SliceHeader, H264Sps};
use crate::h264::{H264Nalu, H264Parser};
use crate::status::{DecoderStatus, DecoderStatusCode};

/// `E_PENDING`（0x8000000A）。
///
/// `DecoderBeginFrame` 在驱动还没准备好时返回它 —— 不是错误，
/// 是「稍后再试」。`windows` crate 没有绑这个码，所以自己写一个。
const E_PENDING: windows::core::HRESULT = windows::core::HRESULT(0x8000_000Au32 as i32);

/// `D3DERR_WASSTILLDRAWING`（0x887A0001）。
///
/// `DecoderBeginFrame` 在硬件还忙着上一帧时返回它 —— 这不是错误，
/// 是「稍后再试」。`windows` crate 没有绑这个码，所以自己写一个。
const D3DERR_WASSTILLDRAWING: windows::core::HRESULT =
    windows::core::HRESULT(0x887A_0001u32 as i32);

/// `DecoderBeginFrame` 最多重试多少次。
///
/// Chromium 是**无限**重试（`d3d11_video_decoder_wrapper.cc` 的
/// `WaitForFrameBegins()`：失败就 `YieldCurrentThread()` 再来一次）。
/// 无限重试意味着驱动卡死时整个进程跟着卡死 —— 本 crate 是同步 API，
/// 调用方没有「取消」的办法，所以这里给一个上限：超了当作解码失败，
/// 让调用方决定要不要复位解码器。
const MAX_BEGIN_FRAME_RETRIES: u32 = 4096;

/// Annex-B 起始码（DXVA2 的切片数据必须带它）。
const START_CODE: [u8; 3] = [0x00, 0x00, 0x01];

/// D3D11 视频解码器的全部 COM 对象。
struct DecoderState {
    device: ID3D11Device,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    decoder: ID3D11VideoDecoder,
}

/// 驱动借给我们的解码缓冲（`ID3D11VideoContext::GetDecoderBuffer`）。
///
/// ⚠️ 必须在写完之后**归还**（`ReleaseDecoderBuffer`）：驱动在归还之前
/// 不会接受同一类型的下一个缓冲，而 `SubmitDecoderBuffers` 要的正是
/// 「已归还、内容还在」的那份数据。
///
/// 归还放在 `Drop` 里 —— 任何一条错误路径都不会把缓冲留在驱动手里。
struct DecoderBufferGuard {
    video_context: ID3D11VideoContext,
    decoder: ID3D11VideoDecoder,
    buffer_type: D3D11_VIDEO_DECODER_BUFFER_TYPE,
    ptr: *mut c_void,
    size: u32,
}

impl DecoderBufferGuard {
    /// 缓冲内容（可写）。
    fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: `ptr` / `size` 来自驱动的 `GetDecoderBuffer`，在
        // `ReleaseDecoderBuffer` 之前一直有效；本守卫在归还之前活着，
        // 而驱动给的是可写缓冲。
        unsafe { std::slice::from_raw_parts_mut(self.ptr as *mut u8, self.size as usize) }
    }
}

impl Drop for DecoderBufferGuard {
    fn drop(&mut self) {
        // SAFETY: `self.video_context` / `self.decoder` 是活着的 COM 引用，
        // `self.buffer_type` 是当初借这个缓冲时用的类型。
        let _ = unsafe {
            self.video_context
                .ReleaseDecoderBuffer(&self.decoder, self.buffer_type)
        };
    }
}

/// 向驱动借一个解码缓冲。
fn get_decoder_buffer(
    video_context: &ID3D11VideoContext,
    decoder: &ID3D11VideoDecoder,
    buffer_type: D3D11_VIDEO_DECODER_BUFFER_TYPE,
) -> Result<DecoderBufferGuard, DecoderStatus> {
    let mut size: u32 = 0;
    let mut ptr: *mut c_void = std::ptr::null_mut();
    // SAFETY: 出参 `size` / `ptr` 都是合法的栈变量；驱动会把缓冲指针
    // 写进 `ptr`，并保证它在 `ReleaseDecoderBuffer` 之前有效。
    unsafe { video_context.GetDecoderBuffer(decoder, buffer_type, &mut size, &mut ptr) }.map_err(
        |e| {
            DecoderStatus::with_message(
                DecoderStatusCode::FailedToGetDecoderBuffer,
                format!("GetDecoderBuffer failed: {e}"),
            )
        },
    )?;
    Ok(DecoderBufferGuard {
        video_context: video_context.clone(),
        decoder: decoder.clone(),
        buffer_type,
        ptr,
        size,
    })
}

/// DXVA2 H.264 硬件解码器。
///
/// 形状与 Chromium 的 `D3D11VideoDecoder` 一致：同步初始化、同步解码，
/// 输出是 **D3D11 纹理**（永不落到系统内存）。
///
/// # 输出纹理的复用
///
/// 每个图片索引（0..16）对应**一张**输出纹理，索引被重新用到时纹理
/// 会被驱动重新画一遍 —— 与 Chromium 的「一张纹理 + 多个 array slice」
/// 等价。因此消费方若要跨帧持有画面，必须自己复制内容；
/// 帧本身持有纹理的**所有权**（[`NativeResourceOwner`]），所以句柄
/// 不会悬空，但内容可能被覆盖。
pub struct Dxva2H264Decoder {
    state: Option<DecoderState>,
    config: Option<VideoDecoderConfig>,
    parser: H264Parser,
    dpb: DecodedPictureBuffer,
    visible_rect: Rect,
    natural_size: Size,
    /// 每个图片索引对应一张输出纹理（惰性创建）。
    textures: Vec<Option<Arc<ID3D11Texture2D>>>,
    /// 尚未提交的缓冲描述（`SubmitDecoderBuffers` 的参数）。
    pending_buffers: Vec<D3D11_VIDEO_DECODER_BUFFER_DESC>,
}

impl Default for Dxva2H264Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Dxva2H264Decoder {
    /// 新建一个**未初始化**的解码器。
    ///
    /// 必须先调 [`VideoDecoder::initialize`]。
    pub fn new() -> Self {
        Self {
            state: None,
            config: None,
            parser: H264Parser::new(),
            dpb: DecodedPictureBuffer::new(),
            visible_rect: Rect::new(0, 0, 0, 0),
            natural_size: Size::new(0, 0),
            textures: (0..MAX_REF_FRAMES).map(|_| None).collect(),
            pending_buffers: Vec::new(),
        }
    }

    /// D3D11 设备（未初始化时是 `None`）。
    ///
    /// 消费方若要把输出纹理交给自己的渲染器，需要它 ——
    /// 不同设备之间的纹理不能直接共享。
    pub fn device(&self) -> Option<&ID3D11Device> {
        self.state.as_ref().map(|state| &state.device)
    }

    /// 当前的编码尺寸（未初始化时是 0×0）。
    pub fn coded_size(&self) -> Size {
        self.config
            .as_ref()
            .map(|config| config.coded_size())
            .unwrap_or(Size::new(0, 0))
    }
}

impl VideoDecoder for Dxva2H264Decoder {
    fn initialize(&mut self, config: &VideoDecoderConfig, _low_delay: bool) -> DecoderStatus {
        if config.codec() != VideoCodec::H264 {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedCodec,
                "the DXVA2 backend only speaks H.264",
            );
        }
        if config.is_encrypted() {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedEncryptionMode,
                "the DXVA2 backend does not support encrypted streams",
            );
        }
        let coded_size = config.coded_size();
        if coded_size.width == 0 || coded_size.height == 0 {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedConfig,
                "the coded size is empty",
            );
        }
        match create_decoder_state(coded_size) {
            Ok(state) => {
                self.state = Some(state);
                self.config = Some(config.clone());
                self.visible_rect = config.visible_rect();
                self.natural_size = config.natural_size();
                DecoderStatus::new(DecoderStatusCode::Ok)
            }
            Err(status) => status,
        }
    }

    fn decode(&mut self, buffer: &DecoderBuffer) -> Result<Vec<VideoFrame>, DecoderStatus> {
        if self.state.is_none() {
            return Err(DecoderStatus::new(DecoderStatusCode::NotInitialized));
        }
        // DXVA2 是同步的：解码器内部不排队，结束流就是「没有帧」。
        if buffer.is_end_of_stream() || buffer.is_empty() {
            return Ok(Vec::new());
        }

        let slices = self.parser.parse_stream(buffer.data()).map_err(|e| {
            DecoderStatus::with_message(
                DecoderStatusCode::MalformedBitstream,
                format!("H.264 parsing failed: {e:?}"),
            )
        })?;

        // 按**图像**分组：一个访问单元可以有多张切片，边界是
        // `first_mb_in_slice == 0`。
        let mut pictures: Vec<Vec<&crate::h264::H264Slice>> = Vec::new();
        for slice in &slices {
            if slice.header.first_mb_in_slice == 0 || pictures.is_empty() {
                pictures.push(Vec::new());
            }
            pictures
                .last_mut()
                .expect("just pushed a picture")
                .push(slice);
        }

        let mut frames = Vec::new();
        for picture in pictures {
            if let Some(frame) =
                self.decode_picture(&picture, buffer.timestamp(), buffer.duration())?
            {
                frames.push(frame);
            }
        }
        Ok(frames)
    }

    fn reset(&mut self) {
        self.parser.reset();
        self.dpb.reset();
        self.pending_buffers.clear();
    }

    /// DXVA2 只吃 Annex-B；容器里通常是 AVCC。
    fn needs_bitstream_conversion(&self) -> bool {
        true
    }

    fn can_read_without_stalling(&self) -> bool {
        true
    }

    /// 与 Chromium 的 `D3D11VideoDecoder::GetMaxDecodeRequests()` 相同。
    fn max_decode_requests(&self) -> usize {
        4
    }

    /// 输出帧持有 D3D11 纹理（外部资源）。
    fn frames_hold_external_resources(&self) -> bool {
        true
    }

    fn decoder_type(&self) -> VideoDecoderType {
        VideoDecoderType::D3D11
    }

    fn is_platform_decoder(&self) -> bool {
        true
    }
}

impl Dxva2H264Decoder {
    /// 解码一张图像（一个访问单元）。
    fn decode_picture(
        &mut self,
        slices: &[&crate::h264::H264Slice],
        timestamp: i64,
        duration: i64,
    ) -> Result<Option<VideoFrame>, DecoderStatus> {
        let first = slices
            .first()
            .ok_or_else(|| DecoderStatus::new(DecoderStatusCode::MalformedBitstream))?;
        let header: &H264SliceHeader = &first.header;

        // 参数集要从解析器里取；先克隆出来 —— 后面要 `&mut self`。
        let pps: H264Pps = self
            .parser
            .get_pps(header.pic_parameter_set_id)
            .cloned()
            .ok_or_else(|| {
                DecoderStatus::with_message(
                    DecoderStatusCode::MalformedBitstream,
                    "the slice references a PPS that was never seen",
                )
            })?;
        let sps: H264Sps = self
            .parser
            .get_sps(pps.seq_parameter_set_id)
            .cloned()
            .ok_or_else(|| {
                DecoderStatus::with_message(
                    DecoderStatusCode::MalformedBitstream,
                    "the PPS references an SPS that was never seen",
                )
            })?;

        let order_cnt = self
            .dpb
            .compute_picture_order_cnt(&sps, header)
            .map_err(|e| {
                DecoderStatus::with_message(
                    DecoderStatusCode::MalformedBitstream,
                    format!("POC computation failed: {e:?}"),
                )
            })?;

        // 图片索引必须在解码**之前**定下来 ——
        // `DXVA_PicParams_H264.CurrPic.Index7Bits` 是驱动认识当前图像的
        // 唯一凭据。
        let picture_index = self.dpb.allocate_picture_index();
        if picture_index == u8::MAX {
            return Err(DecoderStatus::with_message(
                DecoderStatusCode::FailedToGetVideoFrame,
                "the decoded picture buffer is full",
            ));
        }

        let current = DecodedPicture {
            picture_index,
            frame_num: header.frame_num as u16,
            long_term_frame_idx: None,
            top_field_order_cnt: order_cnt.top_field_order_cnt,
            bottom_field_order_cnt: order_cnt.bottom_field_order_cnt,
            is_reference: header.nal_ref_idc != 0,
            non_existing: false,
        };

        let dpb_snapshot: Vec<DecodedPicture> = self.dpb.pictures().to_vec();
        let params = build_pic_params(&PicParamsContext {
            sps: &sps,
            pps: &pps,
            slice: header,
            current: &current,
            dpb: &dpb_snapshot,
        })
        .map_err(|e| {
            DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedConfig,
                format!("DXVA2 picture parameters cannot be built: {e:?}"),
            )
        })?;
        let matrix = build_qmatrix(&sps, &pps);

        let texture = self.output_texture(picture_index)?;
        let output_view = self.output_view(&texture)?;

        self.begin_frame(&output_view)?;
        self.submit_picture_parameters(&params)?;
        self.submit_inverse_quantization_matrix(&matrix)?;

        let nalus: Vec<&H264Nalu> = slices.iter().map(|slice| &slice.nalu).collect();
        self.submit_slices(&nalus)?;
        self.end_frame()?;

        // 解码成功 ⇒ 这张图进 DPB。
        self.dpb
            .commit_picture(&sps, header, picture_index, &order_cnt);

        let handle = texture.as_raw() as u64;
        let owner = NativeResourceOwner::new(texture);
        let coded_size = self.coded_size();
        VideoFrame::from_native_owned(
            VideoPixelFormat::NV12,
            coded_size,
            self.visible_rect,
            self.natural_size,
            // NV12 是一个纹理、两个平面（Y 一个、交错的 UV 一个）。
            vec![handle, handle],
            owner,
            timestamp,
            duration,
        )
        .map(Some)
        .ok_or_else(|| {
            DecoderStatus::with_message(
                DecoderStatusCode::FailedToGetVideoFrame,
                "the output frame does not match the NV12 layout",
            )
        })
    }

    /// 取（必要时创建）某个图片索引的输出纹理。
    fn output_texture(&mut self, picture_index: u8) -> Result<Arc<ID3D11Texture2D>, DecoderStatus> {
        let slot = picture_index as usize;
        if let Some(Some(texture)) = self.textures.get(slot) {
            return Ok(texture.clone());
        }
        let size = self.coded_size();
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size.width,
            Height: size.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            // 驱动要往里画，消费方要拿它当纹理采样。
            BindFlags: (D3D11_BIND_DECODER.0 as u32) | (D3D11_BIND_SHADER_RESOURCE.0 as u32),
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| DecoderStatus::new(DecoderStatusCode::NotInitialized))?;
        let mut texture: Option<ID3D11Texture2D> = None;
        // SAFETY: `desc` 是合法的栈变量；`texture` 是合法的出参。
        unsafe {
            state
                .device
                .CreateTexture2D(&desc, None, Some(&mut texture))
        }
        .map_err(|e| {
            DecoderStatus::with_message(
                DecoderStatusCode::FailedToGetVideoFrame,
                format!("CreateTexture2D failed: {e}"),
            )
        })?;
        let texture = texture.ok_or_else(|| {
            DecoderStatus::with_message(
                DecoderStatusCode::FailedToGetVideoFrame,
                "CreateTexture2D returned no texture",
            )
        })?;
        let texture = Arc::new(texture);
        if let Some(entry) = self.textures.get_mut(slot) {
            *entry = Some(texture.clone());
        }
        Ok(texture)
    }

    /// 建一个输出视图（DXVA2 的「往哪画」）。
    fn output_view(
        &self,
        texture: &ID3D11Texture2D,
    ) -> Result<ID3D11VideoDecoderOutputView, DecoderStatus> {
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| DecoderStatus::new(DecoderStatusCode::NotInitialized))?;
        let desc = D3D11_VIDEO_DECODER_OUTPUT_VIEW_DESC {
            DecodeProfile: D3D11_DECODER_PROFILE_H264_VLD_NOFGT,
            ViewDimension: D3D11_VDOV_DIMENSION_TEXTURE2D,
            Anonymous:
                windows::Win32::Graphics::Direct3D11::D3D11_VIDEO_DECODER_OUTPUT_VIEW_DESC_0 {
                    Texture2D: windows::Win32::Graphics::Direct3D11::D3D11_TEX2D_VDOV {
                        ArraySlice: 0,
                    },
                },
        };
        let mut view: Option<ID3D11VideoDecoderOutputView> = None;
        // SAFETY: `desc` 是合法的栈变量；`view` 是合法的出参；
        // `texture` 是活着的 COM 引用。
        unsafe {
            state
                .video_device
                .CreateVideoDecoderOutputView(texture, &desc, Some(&mut view))
        }
        .map_err(|e| {
            DecoderStatus::with_message(
                DecoderStatusCode::FailedToGetVideoFrame,
                format!("CreateVideoDecoderOutputView failed: {e}"),
            )
        })?;
        view.ok_or_else(|| {
            DecoderStatus::with_message(
                DecoderStatusCode::FailedToGetVideoFrame,
                "CreateVideoDecoderOutputView returned no view",
            )
        })
    }

    /// `DecoderBeginFrame` —— 硬件忙的时候要重试。
    fn begin_frame(&self, output_view: &ID3D11VideoDecoderOutputView) -> Result<(), DecoderStatus> {
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| DecoderStatus::new(DecoderStatusCode::NotInitialized))?;
        for _ in 0..MAX_BEGIN_FRAME_RETRIES {
            // SAFETY: `output_view` 是活着的 COM 引用；没有内容密钥。
            match unsafe {
                state
                    .video_context
                    .DecoderBeginFrame(&state.decoder, output_view, 0, None)
            } {
                Ok(()) => return Ok(()),
                Err(e) if e.code() == E_PENDING || e.code() == D3DERR_WASSTILLDRAWING => {
                    std::thread::yield_now();
                }
                Err(e) => {
                    return Err(DecoderStatus::with_message(
                        DecoderStatusCode::PlatformDecodeFailure,
                        format!("DecoderBeginFrame failed: {e}"),
                    ))
                }
            }
        }
        Err(DecoderStatus::with_message(
            DecoderStatusCode::PlatformDecodeFailure,
            "DecoderBeginFrame kept reporting the decoder as busy",
        ))
    }

    /// `DecoderEndFrame` —— 这一帧解码完成。
    fn end_frame(&self) -> Result<(), DecoderStatus> {
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| DecoderStatus::new(DecoderStatusCode::NotInitialized))?;
        // SAFETY: `state.decoder` 是活着的 COM 引用。
        unsafe { state.video_context.DecoderEndFrame(&state.decoder) }.map_err(|e| {
            DecoderStatus::with_message(
                DecoderStatusCode::PlatformDecodeFailure,
                format!("DecoderEndFrame failed: {e}"),
            )
        })
    }

    /// 提交 `DXVA_PicParams_H264`。
    fn submit_picture_parameters(
        &mut self,
        params: &DxvaPicParamsH264,
    ) -> Result<(), DecoderStatus> {
        let (video_context, decoder) = self.context_and_decoder()?;
        let mut guard = get_decoder_buffer(
            &video_context,
            &decoder,
            D3D11_VIDEO_DECODER_BUFFER_PICTURE_PARAMETERS,
        )?;
        let needed = std::mem::size_of::<DxvaPicParamsH264>();
        {
            let bytes = guard.as_mut_slice();
            if bytes.len() < needed {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::FailedToGetDecoderBuffer,
                    "the picture parameter buffer is smaller than DXVA_PicParams_H264",
                ));
            }
            // SAFETY: `bytes` 至少有 `needed` 字节；`params` 的布局与
            // `DXVA_PicParams_H264` 一致（偏移由单测钉住）。
            unsafe {
                std::ptr::copy_nonoverlapping(
                    params as *const DxvaPicParamsH264 as *const u8,
                    bytes.as_mut_ptr(),
                    needed,
                )
            };
        }
        drop(guard);
        self.pending_buffers.push(D3D11_VIDEO_DECODER_BUFFER_DESC {
            BufferType: D3D11_VIDEO_DECODER_BUFFER_PICTURE_PARAMETERS,
            DataOffset: 0,
            DataSize: needed as u32,
            ..Default::default()
        });
        Ok(())
    }

    /// 提交 `DXVA_Qmatrix_H264`（反量化矩阵）。
    fn submit_inverse_quantization_matrix(
        &mut self,
        matrix: &DxvaQmatrixH264,
    ) -> Result<(), DecoderStatus> {
        let (video_context, decoder) = self.context_and_decoder()?;
        let mut guard = get_decoder_buffer(
            &video_context,
            &decoder,
            D3D11_VIDEO_DECODER_BUFFER_INVERSE_QUANTIZATION_MATRIX,
        )?;
        let needed = std::mem::size_of::<DxvaQmatrixH264>();
        {
            let bytes = guard.as_mut_slice();
            if bytes.len() < needed {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::FailedToGetDecoderBuffer,
                    "the inverse quantization matrix buffer is too small",
                ));
            }
            // SAFETY: 同 `submit_picture_parameters`。
            unsafe {
                std::ptr::copy_nonoverlapping(
                    matrix as *const DxvaQmatrixH264 as *const u8,
                    bytes.as_mut_ptr(),
                    needed,
                )
            };
        }
        drop(guard);
        self.pending_buffers.push(D3D11_VIDEO_DECODER_BUFFER_DESC {
            BufferType: D3D11_VIDEO_DECODER_BUFFER_INVERSE_QUANTIZATION_MATRIX,
            DataOffset: 0,
            DataSize: needed as u32,
            ..Default::default()
        });
        Ok(())
    }

    /// 提交切片数据（位流 + 切片控制）。
    ///
    /// ⚠️ 提交顺序与 Chromium 一致：切片控制在前、位流在后 ——
    /// DXVA2 的 H.264 规范钉死了这个顺序（图片参数 → 反量化矩阵 →
    /// 切片控制 → 位流）。
    fn submit_slices(&mut self, nalus: &[&H264Nalu]) -> Result<(), DecoderStatus> {
        let (video_context, decoder) = self.context_and_decoder()?;
        let mut entries: Vec<DxvaSliceH264Short> = Vec::new();
        let mut guard = get_decoder_buffer(
            &video_context,
            &decoder,
            D3D11_VIDEO_DECODER_BUFFER_BITSTREAM,
        )?;
        let capacity = guard.size as usize;
        let mut written = 0usize;

        for nalu in nalus {
            let need = START_CODE.len() + nalu.data.len();
            if need > capacity {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::Failed,
                    "one slice is larger than the driver's bitstream buffer",
                ));
            }
            if written + need > capacity {
                // 这一块满了 ⇒ 先把它提交掉，再开新的一块
                // （Chromium 的 `SubmitAndGetBitstreamBuffer()` 同款）。
                self.commit_chunk(guard, written, &entries, &video_context, &decoder)?;
                entries.clear();
                self.submit_pending_buffers()?;
                guard = get_decoder_buffer(
                    &video_context,
                    &decoder,
                    D3D11_VIDEO_DECODER_BUFFER_BITSTREAM,
                )?;
                written = 0;
            }
            let offset = written as u32;
            {
                let bytes = guard.as_mut_slice();
                bytes[written..written + START_CODE.len()].copy_from_slice(&START_CODE);
                bytes[written + START_CODE.len()..written + need].copy_from_slice(&nalu.data);
            }
            written += need;
            entries.push(DxvaSliceH264Short::new(offset, need as u32, 0));
        }

        self.commit_chunk(guard, written, &entries, &video_context, &decoder)?;
        self.submit_pending_buffers()
    }

    /// 归还一块位流缓冲并登记两个缓冲描述（切片控制 + 位流）。
    fn commit_chunk(
        &mut self,
        guard: DecoderBufferGuard,
        written: usize,
        entries: &[DxvaSliceH264Short],
        video_context: &ID3D11VideoContext,
        decoder: &ID3D11VideoDecoder,
    ) -> Result<(), DecoderStatus> {
        if !entries.is_empty() {
            let mut control = get_decoder_buffer(
                video_context,
                decoder,
                D3D11_VIDEO_DECODER_BUFFER_SLICE_CONTROL,
            )?;
            let needed = entries.len() * DxvaSliceH264Short::SIZE;
            {
                let bytes = control.as_mut_slice();
                if bytes.len() < needed {
                    return Err(DecoderStatus::with_message(
                        DecoderStatusCode::FailedToGetDecoderBuffer,
                        "the slice control buffer is too small",
                    ));
                }
                for (i, entry) in entries.iter().enumerate() {
                    let start = i * DxvaSliceH264Short::SIZE;
                    bytes[start..start + DxvaSliceH264Short::SIZE]
                        .copy_from_slice(entry.as_bytes());
                }
            }
            drop(control);
            self.pending_buffers.push(D3D11_VIDEO_DECODER_BUFFER_DESC {
                BufferType: D3D11_VIDEO_DECODER_BUFFER_SLICE_CONTROL,
                DataOffset: 0,
                DataSize: needed as u32,
                ..Default::default()
            });
        }

        // 归还位流缓冲（内容保留到 `SubmitDecoderBuffers`）。
        drop(guard);
        self.pending_buffers.push(D3D11_VIDEO_DECODER_BUFFER_DESC {
            BufferType: D3D11_VIDEO_DECODER_BUFFER_BITSTREAM,
            DataOffset: 0,
            DataSize: written as u32,
            ..Default::default()
        });
        Ok(())
    }

    /// `SubmitDecoderBuffers` —— 把攒好的缓冲一次性交给驱动。
    fn submit_pending_buffers(&mut self) -> Result<(), DecoderStatus> {
        let descs = std::mem::take(&mut self.pending_buffers);
        if descs.is_empty() {
            return Ok(());
        }
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| DecoderStatus::new(DecoderStatusCode::NotInitialized))?;
        // SAFETY: `descs` 是合法的切片；`state.decoder` 是活着的 COM 引用。
        unsafe {
            state
                .video_context
                .SubmitDecoderBuffers(&state.decoder, &descs)
        }
        .map_err(|e| {
            DecoderStatus::with_message(
                DecoderStatusCode::PlatformDecodeFailure,
                format!("SubmitDecoderBuffers failed: {e}"),
            )
        })
    }

    /// 取一份视频上下文与解码器的克隆（COM 引用计数 +1）。
    fn context_and_decoder(
        &self,
    ) -> Result<(ID3D11VideoContext, ID3D11VideoDecoder), DecoderStatus> {
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| DecoderStatus::new(DecoderStatusCode::NotInitialized))?;
        Ok((state.video_context.clone(), state.decoder.clone()))
    }
}

/// 建设备、协商 profile、创建解码器。
fn create_decoder_state(coded_size: Size) -> Result<DecoderState, DecoderStatus> {
    let feature_levels = [
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
        D3D_FEATURE_LEVEL_9_1,
    ];
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let mut achieved = D3D_FEATURE_LEVEL_9_1;
    // SAFETY: 出参都是合法的栈变量；`feature_levels` 的生命周期覆盖调用。
    unsafe {
        D3D11CreateDevice(
            None::<&windows::Win32::Graphics::Dxgi::IDXGIAdapter>,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            Some(&feature_levels),
            windows::Win32::Graphics::Direct3D11::D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut achieved),
            Some(&mut context),
        )
    }
    .map_err(|e| {
        DecoderStatus::with_message(
            DecoderStatusCode::FailedToCreateDecoder,
            format!("D3D11CreateDevice failed: {e}"),
        )
    })?;
    let device = device.ok_or_else(|| {
        DecoderStatus::with_message(
            DecoderStatusCode::FailedToCreateDecoder,
            "D3D11CreateDevice returned no device",
        )
    })?;
    let context = context.ok_or_else(|| {
        DecoderStatus::with_message(
            DecoderStatusCode::FailedToCreateDecoder,
            "D3D11CreateDevice returned no immediate context",
        )
    })?;

    // 视频设备与视频上下文**不在** `ID3D11Device` / `ID3D11DeviceContext`
    // 的接口继承里，只能运行时 `QueryInterface`（Chromium 同款）。
    let video_device = device.cast::<ID3D11VideoDevice>().map_err(|e| {
        DecoderStatus::with_message(
            DecoderStatusCode::FailedToCreateDecoder,
            format!("the device does not expose ID3D11VideoDevice: {e}"),
        )
    })?;
    let video_context = context.cast::<ID3D11VideoContext>().map_err(|e| {
        DecoderStatus::with_message(
            DecoderStatusCode::FailedToCreateDecoder,
            format!("the context does not expose ID3D11VideoContext: {e}"),
        )
    })?;

    let desc = D3D11_VIDEO_DECODER_DESC {
        Guid: D3D11_DECODER_PROFILE_H264_VLD_NOFGT,
        SampleWidth: coded_size.width,
        SampleHeight: coded_size.height,
        OutputFormat: DXGI_FORMAT_NV12,
    };

    // SAFETY: `desc` 与 profile 都是合法的栈变量。
    let supported = unsafe {
        video_device
            .CheckVideoDecoderFormat(&D3D11_DECODER_PROFILE_H264_VLD_NOFGT, DXGI_FORMAT_NV12)
    }
    .map_err(|e| {
        DecoderStatus::with_message(
            DecoderStatusCode::UnsupportedProfile,
            format!("CheckVideoDecoderFormat failed: {e}"),
        )
    })?;
    if !supported.as_bool() {
        return Err(DecoderStatus::with_message(
            DecoderStatusCode::UnsupportedProfile,
            "the device does not support H.264 VLD with NV12 output",
        ));
    }

    // SAFETY: `desc` 是合法的栈变量。
    let count = unsafe { video_device.GetVideoDecoderConfigCount(&desc) }.map_err(|e| {
        DecoderStatus::with_message(
            DecoderStatusCode::UnsupportedConfig,
            format!("GetVideoDecoderConfigCount failed: {e}"),
        )
    })?;
    let mut chosen: Option<D3D11_VIDEO_DECODER_CONFIG> = None;
    for index in 0..count {
        let mut config = D3D11_VIDEO_DECODER_CONFIG::default();
        // SAFETY: `desc` 与 `config` 都是合法的栈变量。
        unsafe { video_device.GetVideoDecoderConfig(&desc, index, &mut config) }.map_err(|e| {
            DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedConfig,
                format!("GetVideoDecoderConfig failed: {e}"),
            )
        })?;
        // `ConfigBitstreamRaw == 2` 表示解码器吃 `DXVA_Slice_H264_Short`。
        if config.ConfigBitstreamRaw == 2 {
            chosen = Some(config);
            break;
        }
    }
    let config = chosen.ok_or_else(|| {
        DecoderStatus::with_message(
            DecoderStatusCode::UnsupportedConfig,
            "no decoder configuration with ConfigBitstreamRaw == 2",
        )
    })?;

    // SAFETY: `desc` 与 `config` 都是合法的栈变量。
    let decoder = unsafe { video_device.CreateVideoDecoder(&desc, &config) }.map_err(|e| {
        DecoderStatus::with_message(
            DecoderStatusCode::FailedToCreateDecoder,
            format!("CreateVideoDecoder failed: {e}"),
        )
    })?;

    Ok(DecoderState {
        device,
        video_device,
        video_context,
        decoder,
    })
}
