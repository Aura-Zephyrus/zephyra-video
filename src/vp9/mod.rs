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
use crate::frame::{VideoFrame, VideoPixelFormat};
use crate::geometry::{Rect, Size};
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
    if w <= 0 || h <= 0 || bit_depth != 8 || fmt != IMG_FMT_I420 {
        return;
    }
    let width = w as usize;
    let height = h as usize;
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);
    let size = Size::new(width as u32, height as u32);
    let visible = Rect::new(0, 0, size.width, size.height);

    let Some(plane_y) = copy_plane(y, ys, width, height) else {
        return;
    };
    let Some(plane_u) = copy_plane(u, us, chroma_width, chroma_height) else {
        return;
    };
    let Some(plane_v) = copy_plane(v, vs, chroma_width, chroma_height) else {
        return;
    };

    if let Some(frame) = VideoFrame::from_cpu(
        VideoPixelFormat::I420,
        size,
        visible,
        size,
        vec![plane_y, plane_u, plane_v],
        0,
        0,
    ) {
        OUTPUT.with(|sink| sink.borrow_mut().push(frame));
    }
}

fn copy_plane(src: *const u8, stride: c_int, row_bytes: usize, rows: usize) -> Option<Vec<u8>> {
    if src.is_null() || stride <= 0 {
        return None;
    }
    let stride = usize::try_from(stride).ok()?;
    if stride < row_bytes {
        return None;
    }
    let mut out = vec![0u8; row_bytes.checked_mul(rows)?];
    for row in 0..rows {
        // SAFETY: libvpx guarantees each plane has `rows` rows of `stride` bytes.
        let from = unsafe { src.add(row * stride) };
        let to = &mut out[row * row_bytes..(row + 1) * row_bytes];
        // SAFETY: source is C-owned, destination is a fresh Vec; no overlap.
        unsafe { std::ptr::copy_nonoverlapping(from, to.as_mut_ptr(), row_bytes) };
    }
    Some(out)
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
