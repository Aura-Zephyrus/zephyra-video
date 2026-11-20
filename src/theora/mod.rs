//! Theora decoding via vendored libtheora (BSD-3-Clause, Xiph.org).
//!
//! `native/theora/` holds the decoder half of libtheora plus the matching
//! libogg headers (both BSD-3). A [`DecoderBuffer`] must contain **one Ogg
//! packet**: the first packets are Theora headers, consumed by
//! `th_decode_headerin()`, after which `th_decode_alloc()` creates the context.
//! Callers are responsible for Ogg demuxing. Output is 8-bit I420.

use std::cell::RefCell;
use std::ffi::{c_int, c_void};

use crate::buffer::DecoderBuffer;
use crate::config::VideoDecoderConfig;
use crate::decoder::{VideoDecoder, VideoDecoderType};
use crate::ffi_util::{frame_from_planes, LAYOUT_I420};
use crate::frame::VideoFrame;
use crate::status::{DecoderStatus, DecoderStatusCode};
use crate::VideoCodec;

extern "C" {
    fn z_theora_open(out: *mut *mut c_void) -> c_int;
    fn z_theora_close(handle: *mut c_void);
    fn z_theora_decode(
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
}

thread_local! {
    static OUTPUT: RefCell<Vec<VideoFrame>> = const { RefCell::new(Vec::new()) };
}

/// Layout 1 is planar 4:2:0, as reported by the shim.
const THEORA_LAYOUT_I420: c_int = 1;

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
    if layout != THEORA_LAYOUT_I420 {
        return;
    }
    if let Some(frame) = frame_from_planes(w, h, bpc, LAYOUT_I420, y, ys, u, us, v, vs) {
        OUTPUT.with(|sink| sink.borrow_mut().push(frame));
    }
}

/// libtheora software decoder (8-bit I420 output).
pub struct TheoraDecoder {
    handle: *mut c_void,
    initialized: bool,
}

impl Default for TheoraDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl TheoraDecoder {
    pub fn new() -> Self {
        Self {
            handle: std::ptr::null_mut(),
            initialized: false,
        }
    }
}

impl VideoDecoder for TheoraDecoder {
    fn initialize(&mut self, config: &VideoDecoderConfig, _low_delay: bool) -> DecoderStatus {
        if config.codec() != VideoCodec::Theora {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedCodec,
                "libtheora only decodes Theora",
            );
        }
        if !config.is_valid_config() {
            return DecoderStatus::new(DecoderStatusCode::UnsupportedConfig);
        }
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: `handle` is a valid stack out-param; the shim only writes it.
        let ret = unsafe { z_theora_open(&mut handle) };
        if ret < 0 || handle.is_null() {
            return DecoderStatus::with_message(
                DecoderStatusCode::FailedToCreateDecoder,
                format!("libtheora init failed ({ret})"),
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
            // SAFETY: `data` stays alive for this call; the shim copies the
            // packet into libtheora before returning.
            let ret = unsafe {
                z_theora_decode(
                    self.handle,
                    buffer.data().as_ptr(),
                    buffer.data().len() as c_int,
                    on_frame,
                )
            };
            if ret < 0 {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::MalformedBitstream,
                    format!("libtheora packet failed ({ret})"),
                ));
            }
        }
        Ok(OUTPUT.with(|sink| std::mem::take(&mut *sink.borrow_mut())))
    }

    fn reset(&mut self) {
        OUTPUT.with(|sink| sink.borrow_mut().clear());
    }

    fn decoder_type(&self) -> VideoDecoderType {
        VideoDecoderType::Theora
    }
}

impl Drop for TheoraDecoder {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: `handle` is set once after initialize; Drop runs once.
            unsafe { z_theora_close(self.handle) };
            self.initialized = false;
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/theora_tests.rs"]
mod tests;
