//! 共享 FFI 帧构造：vendored 解码器的 C 平面指针 → [`VideoFrame`]。
//!
//! 各解码器的 shim 把输出摊平成 `(w, h, bpc, layout, 三平面指针与字节 stride)`。
//! 这里统一负责按位深决定每采样字节数、按 layout 选像素格式、并把 C 平面拷成
//! 紧密排列的 Rust `Vec<u8>`。8bit 出 I420，10/12bit 出 YUV420P10/P12。
//!
//! 只有这一小块用 `unsafe`（C 指针）；crate 的其它部分仍是零 unsafe。

use std::cell::Cell;
use std::ffi::{c_int, c_void};

use crate::frame::{VideoFrame, VideoPixelFormat};
use crate::geometry::{Rect, Size};

/// 归一化色度布局：4:2:0 三平面（Y/U/V）。
pub const LAYOUT_I420: c_int = 1;

thread_local! {
    /// 当前正在解码的缓冲的时基。C 回调拿不到 Rust 的 `DecoderBuffer`，
    /// 所以在调用 shim 前把它放进线程局部，`on_frame` 造帧时取用。
    static CURRENT_TIMESTAMP: Cell<i64> = const { Cell::new(0) };
    static CURRENT_DURATION: Cell<i64> = const { Cell::new(0) };
}

/// 在调用 C 解码器前设置当前缓冲的时基。
pub fn set_decode_timing(timestamp: i64, duration: i64) {
    CURRENT_TIMESTAMP.with(|c| c.set(timestamp));
    CURRENT_DURATION.with(|c| c.set(duration));
}

/// 从三个 C 平面构造一帧紧密排列的 CPU 帧。
///
/// `bpc` 是位深（8/10/12），`layout` 取 [`LAYOUT_I420`]，各 `*_stride` 是
/// C 报告的行距（**字节**）。任一步不合法就返回 `None`，绝不为坏数据造帧。
#[allow(clippy::too_many_arguments)]
pub fn frame_from_planes(
    w: c_int,
    h: c_int,
    bpc: c_int,
    layout: c_int,
    y: *const u8,
    y_stride: isize,
    u: *const u8,
    u_stride: isize,
    v: *const u8,
    v_stride: isize,
) -> Option<VideoFrame> {
    let format = pixel_format(layout, bpc)?;
    let bytes = bytes_per_sample(format);
    let width = usize::try_from(w).ok()?;
    let height = usize::try_from(h).ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);

    let plane_y = copy_plane(y, y_stride, width, height, bytes)?;
    let plane_u = copy_plane(u, u_stride, chroma_width, chroma_height, bytes)?;
    let plane_v = copy_plane(v, v_stride, chroma_width, chroma_height, bytes)?;

    let size = Size::new(u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    let visible = Rect::new(0, 0, size.width, size.height);
    VideoFrame::from_cpu(
        format,
        size,
        visible,
        size,
        vec![plane_y, plane_u, plane_v],
        CURRENT_TIMESTAMP.with(|c| c.get()),
        CURRENT_DURATION.with(|c| c.get()),
    )
}

fn pixel_format(layout: c_int, bpc: c_int) -> Option<VideoPixelFormat> {
    match (layout, bpc) {
        (LAYOUT_I420, 8) => Some(VideoPixelFormat::I420),
        (LAYOUT_I420, 10) => Some(VideoPixelFormat::YUV420P10),
        (LAYOUT_I420, 12) => Some(VideoPixelFormat::YUV420P12),
        _ => None,
    }
}

const fn bytes_per_sample(format: VideoPixelFormat) -> usize {
    match format {
        VideoPixelFormat::I420 => 1,
        VideoPixelFormat::YUV420P10 | VideoPixelFormat::YUV420P12 => 2,
        _ => 1,
    }
}

/// 把 C 平面拷成紧密排列的字节；`stride` 是 C 的行距（字节）。
fn copy_plane(
    src: *const u8,
    stride: isize,
    width: usize,
    height: usize,
    bytes_per_sample: usize,
) -> Option<Vec<u8>> {
    if src.is_null() || stride <= 0 {
        return None;
    }
    let stride = usize::try_from(stride).ok()?;
    let row_bytes = width.checked_mul(bytes_per_sample)?;
    if stride < row_bytes {
        return None;
    }
    let mut out = vec![0u8; row_bytes.checked_mul(height)?];
    for row in 0..height {
        // 恶意码流可能报告超大 stride；乘法溢出会让 add() 落到任意地址。
        let offset = row.checked_mul(stride)?;
        // SAFETY: 上游保证该平面有 `height` 行、每行 `stride` 字节。
        let from = unsafe { src.add(offset) };
        let to = &mut out[row * row_bytes..(row + 1) * row_bytes];
        // SAFETY: 源是 C 所有、目标是新建的 Vec，二者不重叠。
        unsafe { std::ptr::copy_nonoverlapping(from, to.as_mut_ptr(), row_bytes) };
    }
    Some(out)
}

/// 真正复位一个 C 解码器：关闭旧上下文再开一个新的。
///
/// 各软件解码器的 `reset()` 只清 Rust 输出是不够的 —— C 侧的参考帧 / DPB
/// 仍残留。这里统一做 close + reopen；若 reopen 失败，把 `initialized`
/// 置为 `false`，后续 `decode()` 会返回 `NotInitialized` 而不是用坏句柄。
pub fn reopen_decoder<O, C>(handle: &mut *mut c_void, initialized: &mut bool, close: C, open: O)
where
    O: Fn(*mut *mut c_void) -> c_int,
    C: Fn(*mut c_void),
{
    if !*initialized || handle.is_null() {
        return;
    }
    close(*handle);
    *handle = std::ptr::null_mut();
    let mut fresh: *mut c_void = std::ptr::null_mut();
    if open(&mut fresh) == 0 && !fresh.is_null() {
        *handle = fresh;
    } else {
        *initialized = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_bit_depths_to_planar_formats() {
        assert_eq!(pixel_format(LAYOUT_I420, 8), Some(VideoPixelFormat::I420));
        assert_eq!(
            pixel_format(LAYOUT_I420, 10),
            Some(VideoPixelFormat::YUV420P10)
        );
        assert_eq!(
            pixel_format(LAYOUT_I420, 12),
            Some(VideoPixelFormat::YUV420P12)
        );
        assert_eq!(pixel_format(LAYOUT_I420, 16), None);
        assert_eq!(pixel_format(0, 8), None);
    }

    #[test]
    fn samples_per_pixel_follow_bit_depth() {
        assert_eq!(bytes_per_sample(VideoPixelFormat::I420), 1);
        assert_eq!(bytes_per_sample(VideoPixelFormat::YUV420P10), 2);
        assert_eq!(bytes_per_sample(VideoPixelFormat::YUV420P12), 2);
    }
}
