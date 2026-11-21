//! H.264 software decoding via vendored OpenH264 (BSD-2-Clause, Cisco).
//!
//! Only the decoder half of OpenH264 is compiled, with `X86_ASM` undefined so
//! the generic C++ fallbacks are used (no nasm/yasm). `native/openh264_shim.c`
//! hides `ISVCDecoderVtbl` / `SBufferInfo` from Rust and flattens each decoded
//! picture into scalar callback arguments.

use std::cell::RefCell;
use std::ffi::{c_int, c_void};

use crate::buffer::DecoderBuffer;
use crate::config::VideoDecoderConfig;
use crate::decoder::{VideoDecoder, VideoDecoderType};
use crate::ffi_util::{frame_from_planes, reopen_decoder, set_decode_timing, LAYOUT_I420};
use crate::frame::VideoFrame;
use crate::status::{DecoderStatus, DecoderStatusCode};
use crate::VideoCodec;

crate::codec_api!("z_h264";
    fn z_h264_open(out: *mut *mut c_void) -> c_int;
    fn z_h264_close(handle: *mut c_void);
    fn z_h264_flush(
        handle: *mut c_void,
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
    fn z_h264_decode(
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

/// OpenH264 outputs `videoFormatI420`, mapped by the shim to layout 1 (8-bit only).
const OPENH264_LAYOUT_I420: c_int = 1;

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
    if layout != OPENH264_LAYOUT_I420 {
        return;
    }
    if let Some(frame) = frame_from_planes(w, h, bpc, LAYOUT_I420, y, ys, u, us, v, vs) {
        OUTPUT.with(|sink| sink.borrow_mut().push(frame));
    }
}

/// OpenH264 software decoder (H.264, 8-bit I420 output).
pub struct OpenH264Decoder {
    handle: *mut c_void,
    initialized: bool,
}

impl Default for OpenH264Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenH264Decoder {
    pub fn new() -> Self {
        Self {
            handle: std::ptr::null_mut(),
            initialized: false,
        }
    }
}

impl VideoDecoder for OpenH264Decoder {
    fn initialize(&mut self, config: &VideoDecoderConfig, _low_delay: bool) -> DecoderStatus {
        if config.codec() != VideoCodec::H264 {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedCodec,
                "OpenH264 only decodes H.264",
            );
        }
        if !config.is_valid_config() {
            return DecoderStatus::new(DecoderStatusCode::UnsupportedConfig);
        }
        if let Some(status) = crate::check_lib!("z_h264") {
            return status;
        }
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: `handle` is a valid stack out-param; the shim only writes it.
        let ret = unsafe { z_h264_open(&mut handle) };
        if ret < 0 || handle.is_null() {
            return DecoderStatus::with_message(
                DecoderStatusCode::FailedToCreateDecoder,
                format!("WelsCreateDecoder/Initialize failed ({ret})"),
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
            // SAFETY: `data` stays alive for this call; the shim consumes it
            // before returning and keeps no pointer.
            let ret = unsafe {
                z_h264_decode(
                    self.handle,
                    buffer.data().as_ptr(),
                    buffer.data().len() as c_int,
                    on_frame,
                )
            };
            if ret < 0 {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::MalformedBitstream,
                    format!("OpenH264 DecodeFrameNoDelay failed ({ret})"),
                ));
            }
        }
        if buffer.is_end_of_stream() {
            // SAFETY: `on_frame` only writes to the thread-local sink.
            let ret = unsafe { z_h264_flush(self.handle, on_frame) };
            if ret < 0 {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::Failed,
                    format!("OpenH264 EOS flush failed ({ret})"),
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
            |h| unsafe { z_h264_close(h) },
            |out| unsafe { z_h264_open(out) },
        );
    }

    /// OpenH264 eats Annex-B; containers normally carry AVCC (length-prefixed).
    fn needs_bitstream_conversion(&self) -> bool {
        true
    }

    fn decoder_type(&self) -> VideoDecoderType {
        VideoDecoderType::OpenH264
    }
}

impl Drop for OpenH264Decoder {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: `handle` is set once after initialize; Drop runs once.
            unsafe { z_h264_close(self.handle) };
            self.initialized = false;
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/h264_decoder_tests.rs"]
mod tests;
