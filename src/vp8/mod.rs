//! VP8 decoding via the same vendored libvpx as VP9 (BSD-3-Clause).
//!
//! VP8 and VP9 share one libvpx build; only the codec interface differs
//! (`vpx_codec_vp8_dx()` vs `vpx_codec_vp9_dx()`). The C shim flattens
//! `vpx_image_t` into scalar callback arguments, so Rust mirrors no structs.

use std::cell::RefCell;
use std::ffi::{c_int, c_void};

use crate::buffer::DecoderBuffer;
use crate::config::VideoDecoderConfig;
use crate::decoder::{VideoDecoder, VideoDecoderType};
use crate::ffi_util::{frame_from_planes, reopen_decoder, set_decode_timing, LAYOUT_I420};
use crate::frame::VideoFrame;
use crate::status::{DecoderStatus, DecoderStatusCode};
use crate::VideoCodec;

crate::codec_api!("z_vpx";
    fn z_vpx8_open(out: *mut *mut c_void) -> c_int;
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

);

thread_local! {
    static OUTPUT: RefCell<Vec<VideoFrame>> = const { RefCell::new(Vec::new()) };
}

const IMG_FMT_I420: c_int = 0x100 | 2;

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
    if fmt != IMG_FMT_I420 {
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

/// libvpx VP8 software decoder.
pub struct Vpx8Decoder {
    handle: *mut c_void,
    initialized: bool,
}

impl Default for Vpx8Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Vpx8Decoder {
    pub fn new() -> Self {
        Self {
            handle: std::ptr::null_mut(),
            initialized: false,
        }
    }
}

impl VideoDecoder for Vpx8Decoder {
    fn initialize(&mut self, config: &VideoDecoderConfig, _low_delay: bool) -> DecoderStatus {
        if config.codec() != VideoCodec::VP8 {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedCodec,
                "libvpx VP8 decoder only decodes VP8",
            );
        }
        if !config.is_valid_config() {
            return DecoderStatus::new(DecoderStatusCode::UnsupportedConfig);
        }
        if let Some(status) = crate::check_lib!("z_vpx") {
            return status;
        }
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: `handle` is a valid stack out-param.
        let ret = unsafe { z_vpx8_open(&mut handle) };
        if ret < 0 || handle.is_null() {
            return DecoderStatus::with_message(
                DecoderStatusCode::FailedToCreateDecoder,
                format!("vpx_codec_dec_init(VP8) failed ({ret})"),
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
                    format!("vpx_codec_decode(VP8) failed ({ret})"),
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
            |h| unsafe { z_vpx_close(h) },
            |out| unsafe { z_vpx8_open(out) },
        );
    }

    fn decoder_type(&self) -> VideoDecoderType {
        VideoDecoderType::Vpx
    }
}

impl Drop for Vpx8Decoder {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: `handle` is set once after initialize; Drop runs once.
            unsafe { z_vpx_close(self.handle) };
            self.initialized = false;
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/vp8_tests.rs"]
mod tests;
