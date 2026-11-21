//! AV1 decoding via vendored dav1d (BSD-2-Clause, same as Chromium).
//!
//! `native/dav1d/` mirrors Chromium's third_party/dav1d at the same DEPS pin:
//! `libdav1d/` is upstream source, `config/` is Chromium's generated config,
//! and `dav1d_generated.gni` is Chromium's exact C source list.
//!
//! Build flags: HAVE_ASM=0 (no nasm/yasm here, pure-C fallback), CONFIG_LOG=0.
//! `native/dav1d_shim.c` flattens Dav1dPicture into scalar callback args so
//! Rust never has to mirror that large, version-dependent struct.

use std::cell::RefCell;
use std::ffi::{c_int, c_void};

use crate::buffer::DecoderBuffer;
use crate::config::VideoDecoderConfig;
use crate::decoder::{VideoDecoder, VideoDecoderType};
use crate::ffi_util::{frame_from_planes, reopen_decoder, set_decode_timing, LAYOUT_I420};
use crate::frame::VideoFrame;
use crate::status::{DecoderStatus, DecoderStatusCode};
use crate::VideoCodec;

crate::codec_api!("z_dav1d";
    fn z_dav1d_open(out: *mut *mut c_void, threads: c_int) -> c_int;
    fn z_dav1d_close(handle: *mut c_void);
    fn z_dav1d_decode(
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

/// 单线程打开，供 `reset()` 的通用 reopen helper 使用。
unsafe extern "C" fn open_single(out: *mut *mut c_void) -> c_int {
    z_dav1d_open(out, 1)
}

/// DAV1D_PIXEL_LAYOUT_I420 from dav1d/headers.h.
const DAV1D_LAYOUT_I420: c_int = 1;

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
    if layout != DAV1D_LAYOUT_I420 {
        return;
    }
    if let Some(frame) = frame_from_planes(w, h, bpc, LAYOUT_I420, y, ys, u, us, v, vs) {
        OUTPUT.with(|sink| sink.borrow_mut().push(frame));
    }
}

/// dav1d software decoder.
pub struct Dav1dDecoder {
    handle: *mut c_void,
    initialized: bool,
}

impl Default for Dav1dDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Dav1dDecoder {
    pub fn new() -> Self {
        Self {
            handle: std::ptr::null_mut(),
            initialized: false,
        }
    }
}

impl VideoDecoder for Dav1dDecoder {
    fn initialize(&mut self, config: &VideoDecoderConfig, _low_delay: bool) -> DecoderStatus {
        if config.codec() != VideoCodec::AV1 {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedCodec,
                "dav1d only decodes AV1",
            );
        }
        if !config.is_valid_config() {
            return DecoderStatus::new(DecoderStatusCode::UnsupportedConfig);
        }
        if let Some(status) = crate::check_lib!("z_dav1d") {
            return status;
        }
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: `handle` is a valid stack out-param; the shim only writes it.
        let ret = unsafe { z_dav1d_open(&mut handle, 1) };
        if ret < 0 || handle.is_null() {
            return DecoderStatus::with_message(
                DecoderStatusCode::FailedToCreateDecoder,
                format!("dav1d_open failed ({ret})"),
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
                z_dav1d_decode(
                    self.handle,
                    buffer.data().as_ptr(),
                    buffer.data().len() as c_int,
                    on_frame,
                )
            };
            if ret < 0 {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::MalformedBitstream,
                    format!("dav1d_decode failed ({ret})"),
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
            |h| unsafe { z_dav1d_close(h) },
            |out| unsafe { open_single(out) },
        );
    }

    fn decoder_type(&self) -> VideoDecoderType {
        VideoDecoderType::Dav1d
    }
}

impl Drop for Dav1dDecoder {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: `handle` is set once after initialize; Drop runs once.
            unsafe { z_dav1d_close(self.handle) };
            self.initialized = false;
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/av1_tests.rs"]
mod tests;
