//! MJPEG decoding via vendored libjpeg-turbo 2.1.5.1 (BSD-3 / IJG license).
//!
//! Each [`DecoderBuffer`] must contain **one complete JPEG frame** (SOI…EOU).
//! The shim asks libjpeg for full-resolution YCbCr and box-filters chroma down
//! to 4:2:0, so the Rust side receives ordinary 8-bit I420 planes. No SIMD is
//! compiled — libjpeg's C fallback is used.

use std::cell::RefCell;
use std::ffi::{c_int, c_void};

use crate::buffer::DecoderBuffer;
use crate::config::VideoDecoderConfig;
use crate::decoder::{VideoDecoder, VideoDecoderType};
use crate::ffi_util::{frame_from_planes, reopen_decoder, set_decode_timing, LAYOUT_I420};
use crate::frame::VideoFrame;
use crate::status::{DecoderStatus, DecoderStatusCode};
use crate::VideoCodec;

crate::codec_api!("z_mjpeg";
    fn z_mjpeg_open(out: *mut *mut c_void) -> c_int;
    fn z_mjpeg_close(handle: *mut c_void);
    fn z_mjpeg_decode(
        handle: *mut c_void,
        data: *const u8,
        len: c_int,
        callback: extern "C" fn(
            c_int,
            c_int,
            c_int,
            c_int,
            *const u8,
            isize,
            *const u8,
            isize,
            *const u8,
            isize,
        ),
    ) -> c_int;

);

thread_local! {
    static OUTPUT: RefCell<Vec<VideoFrame>> = const { RefCell::new(Vec::new()) };
}

/// Layout 1 is planar 4:2:0, as reported by the shim.
const MJPEG_LAYOUT_I420: c_int = 1;

extern "C" fn on_frame(
    w: c_int,
    h: c_int,
    bpc: c_int,
    layout: c_int,
    y: *const u8,
    ys: isize,
    u: *const u8,
    us: isize,
    v: *const u8,
    vs: isize,
) {
    if layout != MJPEG_LAYOUT_I420 {
        return;
    }
    if let Some(frame) = frame_from_planes(w, h, bpc, LAYOUT_I420, y, ys, u, us, v, vs) {
        OUTPUT.with(|sink| sink.borrow_mut().push(frame));
    }
}

/// libjpeg-turbo software decoder (MJPEG → 8-bit I420).
pub struct MjpegDecoder {
    handle: *mut c_void,
    initialized: bool,
}

impl Default for MjpegDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl MjpegDecoder {
    pub fn new() -> Self {
        Self {
            handle: std::ptr::null_mut(),
            initialized: false,
        }
    }
}

impl VideoDecoder for MjpegDecoder {
    fn initialize(&mut self, config: &VideoDecoderConfig, _low_delay: bool) -> DecoderStatus {
        if config.codec() != VideoCodec::Mjpeg {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedCodec,
                "libjpeg-turbo only decodes MJPEG",
            );
        }
        if !config.is_valid_config() {
            return DecoderStatus::new(DecoderStatusCode::UnsupportedConfig);
        }
        if let Some(status) = crate::check_lib!("z_mjpeg") {
            return status;
        }
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: `handle` is a valid stack out-param; the shim only writes it.
        let ret = unsafe { z_mjpeg_open(&mut handle) };
        if ret < 0 || handle.is_null() {
            return DecoderStatus::with_message(
                DecoderStatusCode::FailedToCreateDecoder,
                format!("jpeg_create_decompress failed ({ret})"),
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
            // SAFETY: `data` stays alive for this call; the shim reads it
            // entirely inside jpeg_mem_src before returning.
            let ret = unsafe {
                z_mjpeg_decode(
                    self.handle,
                    buffer.data().as_ptr(),
                    buffer.data().len() as c_int,
                    on_frame,
                )
            };
            if ret < 0 {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::MalformedBitstream,
                    format!("MJPEG decompression failed ({ret})"),
                ));
            }
        }
        Ok(OUTPUT.with(|sink| std::mem::take(&mut *sink.borrow_mut())))
    }

    fn reset(&mut self) {
        OUTPUT.with(|sink| sink.borrow_mut().clear());
        reopen_decoder(
            &mut self.handle,
            &mut self.initialized,
            |h| unsafe { z_mjpeg_close(h) },
            |out| unsafe { z_mjpeg_open(out) },
        );
    }

    fn decoder_type(&self) -> VideoDecoderType {
        VideoDecoderType::Mjpeg
    }
}

impl Drop for MjpegDecoder {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: `handle` is set once after initialize; Drop runs once.
            unsafe { z_mjpeg_close(self.handle) };
            self.initialized = false;
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/mjpeg_tests.rs"]
mod tests;
