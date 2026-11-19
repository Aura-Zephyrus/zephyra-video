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
use crate::frame::{VideoFrame, VideoPixelFormat};
use crate::geometry::{Rect, Size};
use crate::status::{DecoderStatus, DecoderStatusCode};
use crate::VideoCodec;

extern "C" {
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
}

thread_local! {
    static OUTPUT: RefCell<Vec<VideoFrame>> = const { RefCell::new(Vec::new()) };
}

/// DAV1D_PIXEL_LAYOUT_I420 from dav1d/headers.h; only 8-bit 4:2:0 is handled.
const PIXEL_LAYOUT_I420: c_int = 1;

extern "C" fn on_frame(
    w: c_int,
    h: c_int,
    bpc: c_int,
    layout: c_int,
    y: *const u8,
    ys: isize,
    u: *const u8,
    _us: isize,
    v: *const u8,
    vs: isize,
) {
    if w <= 0 || h <= 0 || bpc != 8 || layout != PIXEL_LAYOUT_I420 {
        return;
    }
    let width = w as usize;
    let height = h as usize;
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);
    let size = Size::new(width as u32, height as u32);
    let visible = Rect::new(0, 0, size.width, size.height);

    // dav1d shares one chroma stride for U and V; the shim passes it to both.
    let Some(plane_y) = copy_plane(y, ys, width, height) else {
        return;
    };
    let Some(plane_u) = copy_plane(u, vs, chroma_width, chroma_height) else {
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

/// Copy a C plane into tightly-packed bytes; `stride` is the C row pitch.
fn copy_plane(src: *const u8, stride: isize, row_bytes: usize, rows: usize) -> Option<Vec<u8>> {
    if src.is_null() || stride <= 0 {
        return None;
    }
    let stride = usize::try_from(stride).ok()?;
    if stride < row_bytes {
        return None;
    }
    let mut out = vec![0u8; row_bytes.checked_mul(rows)?];
    for row in 0..rows {
        // SAFETY: dav1d guarantees each plane has `rows` rows of `stride` bytes.
        let from = unsafe { src.add(row * stride) };
        let to = &mut out[row * row_bytes..(row + 1) * row_bytes];
        // SAFETY: source is C-owned, destination is a fresh Vec; no overlap.
        unsafe { std::ptr::copy_nonoverlapping(from, to.as_mut_ptr(), row_bytes) };
    }
    Some(out)
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
