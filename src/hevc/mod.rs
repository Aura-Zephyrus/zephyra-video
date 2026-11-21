//! HEVC / H.265 decoding via vendored libde265 (LGPL-3.0+).
//!
//! `native/libde265/` is a verbatim copy of libde265 1.0.15 with only the
//! generated `de265-version.h` filled in. The library is C++ but exposes a
//! C API; `native/hevc_shim.cpp` hides de265_image behind scalar callbacks,
//! so Rust mirrors no structs. Assembly/SIMD is disabled (pure C++ fallback).

use std::cell::RefCell;
use std::ffi::{c_int, c_void};

use crate::buffer::DecoderBuffer;
use crate::config::VideoDecoderConfig;
use crate::decoder::{VideoDecoder, VideoDecoderType};
use crate::ffi_util::{frame_from_planes, set_decode_timing, LAYOUT_I420};
use crate::frame::VideoFrame;
use crate::status::{DecoderStatus, DecoderStatusCode};
use crate::VideoCodec;

crate::codec_api!("z_hevc";
    fn z_hevc_open() -> *mut c_void;
    fn z_hevc_close(handle: *mut c_void);
    fn z_hevc_reset(handle: *mut c_void);
    fn z_hevc_flush(
        handle: *mut c_void,
        callback: extern "C" fn(
            c_int,
            c_int,
            c_int,
            c_int,
            *const u8,
            c_int,
            *const u8,
            c_int,
            *const u8,
            c_int,
        ),
    ) -> c_int;
    fn z_hevc_decode(
        handle: *mut c_void,
        data: *const u8,
        len: c_int,
        callback: extern "C" fn(
            c_int,
            c_int,
            c_int,
            c_int,
            *const u8,
            c_int,
            *const u8,
            c_int,
            *const u8,
            c_int,
        ),
    ) -> c_int;

);

thread_local! {
    static OUTPUT: RefCell<Vec<VideoFrame>> = const { RefCell::new(Vec::new()) };
}

/// de265_chroma_420 (see de265.h).
const DE265_CHROMA_420: c_int = 1;

extern "C" fn on_frame(
    w: c_int,
    h: c_int,
    bpc: c_int,
    chroma: c_int,
    y: *const u8,
    ys: c_int,
    u: *const u8,
    us: c_int,
    v: *const u8,
    vs: c_int,
) {
    if chroma != DE265_CHROMA_420 {
        return;
    }
    if let Some(frame) = frame_from_planes(
        w,
        h,
        bpc,
        LAYOUT_I420,
        y,
        ys as isize,
        u,
        us as isize,
        v,
        vs as isize,
    ) {
        OUTPUT.with(|sink| sink.borrow_mut().push(frame));
    }
}

/// libde265 HEVC software decoder.
pub struct HevcDecoder {
    handle: *mut c_void,
    initialized: bool,
}

impl Default for HevcDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl HevcDecoder {
    pub fn new() -> Self {
        Self {
            handle: std::ptr::null_mut(),
            initialized: false,
        }
    }
}

impl VideoDecoder for HevcDecoder {
    fn initialize(&mut self, config: &VideoDecoderConfig, _low_delay: bool) -> DecoderStatus {
        if config.codec() != VideoCodec::HEVC {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedCodec,
                "libde265 only decodes HEVC",
            );
        }
        if !config.is_valid_config() {
            return DecoderStatus::new(DecoderStatusCode::UnsupportedConfig);
        }
        if let Some(status) = crate::check_lib!("z_hevc") {
            return status;
        }
        // SAFETY: z_hevc_open returns an opaque handle or NULL.
        let handle = unsafe { z_hevc_open() };
        if handle.is_null() {
            return DecoderStatus::with_message(
                DecoderStatusCode::FailedToCreateDecoder,
                "de265_new_decoder failed",
            );
        }
        self.handle = handle;
        self.initialized = true;
        DecoderStatus::default()
    }

    fn decode(&mut self, buffer: &DecoderBuffer) -> Result<Vec<VideoFrame>, DecoderStatus> {
        if !self.initialized {
            return Err(DecoderStatus::new(DecoderStatusCode::NotInitialized));
        }
        OUTPUT.with(|sink| sink.borrow_mut().clear());
        if !buffer.is_end_of_stream() && !buffer.data().is_empty() {
            // 延迟帧在 EOS 时才吐出：保留最后一次数据帧的时基。
            set_decode_timing(buffer.timestamp(), buffer.duration());
            // SAFETY: `data` stays alive for this call; the shim consumes it.
            let ret = unsafe {
                z_hevc_decode(
                    self.handle,
                    buffer.data().as_ptr(),
                    buffer.data().len() as c_int,
                    on_frame,
                )
            };
            if ret < 0 {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::MalformedBitstream,
                    format!("de265_decode failed ({ret})"),
                ));
            }
        }
        if buffer.is_end_of_stream() {
            // SAFETY: `on_frame` only writes to the thread-local sink.
            let ret = unsafe { z_hevc_flush(self.handle, on_frame) };
            if ret < 0 {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::Failed,
                    format!("de265 EOS flush failed ({ret})"),
                ));
            }
        }
        Ok(OUTPUT.with(|sink| std::mem::take(&mut *sink.borrow_mut())))
    }

    fn reset(&mut self) {
        if self.initialized {
            // SAFETY: `handle` is valid.
            unsafe { z_hevc_reset(self.handle) };
        }
        OUTPUT.with(|sink| sink.borrow_mut().clear());
    }

    fn decoder_type(&self) -> VideoDecoderType {
        VideoDecoderType::Hevc
    }
}

impl Drop for HevcDecoder {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: `handle` is set once after initialize; Drop runs once.
            unsafe { z_hevc_close(self.handle) };
            self.initialized = false;
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/hevc_tests.rs"]
mod tests;
