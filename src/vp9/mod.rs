//! VP9 decoding via vendored libvpx (BSD-3-Clause, same as Chromium).
//!
//! `native/libvpx/` mirrors Chromium's third_party/libvpx at the same DEPS pin;
//! `config/` is Chromium's generated config and `libvpx_srcs.gni` is its exact
//! source list. Assembly is disabled (HAVE_X86_ASM=0, no yasm/nasm here).
//! `native/libvpx_shim.c` flattens vpx_image_t into scalar callback args.

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
    fn z_vpx9_open(out: *mut *mut c_void) -> c_int;
    fn z_vpx_close(handle: *mut c_void);
    fn z_vpx_decode(
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
}

thread_local! {
    static OUTPUT: RefCell<Vec<VideoFrame>> = const { RefCell::new(Vec::new()) };
}

/// VPX_IMG_FMT_I420 == VPX_IMG_FMT_PLANAR | 2 == 0x100 | 2.
const IMG_FMT_I420: c_int = 0x100 | 2;
/// VPX_IMG_FMT_HIGHBITDEPTH: 10/12-bit images set this bit on top of I420.
const IMG_FMT_HIGHBITDEPTH: c_int = 0x800;

extern "C" fn on_frame(
    w: c_int,
    h: c_int,
    bit_depth: c_int,
    fmt: c_int,
    y: *const u8,
    ys: c_int,
    u: *const u8,
    us: c_int,
    v: *const u8,
    vs: c_int,
) {
    if fmt & !IMG_FMT_HIGHBITDEPTH != IMG_FMT_I420 {
        return;
    }
    if let Some(frame) = frame_from_planes(
        w,
        h,
        bit_depth,
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

/// libvpx VP9 software decoder.
pub struct Vpx9Decoder {
    handle: *mut c_void,
    initialized: bool,
}

impl Default for Vpx9Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Vpx9Decoder {
    pub fn new() -> Self {
        Self {
            handle: std::ptr::null_mut(),
            initialized: false,
        }
    }
}

impl VideoDecoder for Vpx9Decoder {
    fn initialize(&mut self, config: &VideoDecoderConfig, _low_delay: bool) -> DecoderStatus {
        if config.codec() != VideoCodec::VP9 {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedCodec,
                "libvpx VP9 decoder only decodes VP9",
            );
        }
        if !config.is_valid_config() {
            return DecoderStatus::new(DecoderStatusCode::UnsupportedConfig);
        }
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: `handle` is a valid stack out-param.
        let ret = unsafe { z_vpx9_open(&mut handle) };
        if ret < 0 || handle.is_null() {
            return DecoderStatus::with_message(
                DecoderStatusCode::FailedToCreateDecoder,
                format!("vpx_codec_dec_init failed ({ret})"),
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
            // SAFETY: `data` stays alive for this call; the shim consumes it.
            let ret = unsafe {
                z_vpx_decode(
                    self.handle,
                    buffer.data().as_ptr(),
                    buffer.data().len() as c_int,
                    on_frame,
                )
            };
            if ret < 0 {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::MalformedBitstream,
                    format!("vpx_codec_decode failed ({ret})"),
                ));
            }
        }
        Ok(OUTPUT.with(|sink| std::mem::take(&mut *sink.borrow_mut())))
    }

    fn reset(&mut self) {
        OUTPUT.with(|sink| sink.borrow_mut().clear());
    }

    fn decoder_type(&self) -> VideoDecoderType {
        VideoDecoderType::Vpx
    }
}

impl Drop for Vpx9Decoder {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: `handle` is set once after initialize; Drop runs once.
            unsafe { z_vpx_close(self.handle) };
            self.initialized = false;
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/vp9_tests.rs"]
mod tests;
