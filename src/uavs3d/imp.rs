//! uavs3d 的 FFI 与解码器实现（仅在 `uavs3d` feature 下编译）。
//!
//! unsafe 只出现在这里：它调用 vendored C 的导出函数、读 C 回调给的指针。
//! 生成的 `VideoFrame` 是**系统内存**里的 NV12 拷贝 —— C 解码器的图像缓冲
//! 归它自己管，回调结束就可能被复用，所以必须逐行拷出来。

use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void};

use crate::buffer::DecoderBuffer;
use crate::config::VideoDecoderConfig;
use crate::decoder::{VideoDecoder, VideoDecoderType};
use crate::frame::{VideoFrame, VideoPixelFormat};
use crate::geometry::{Rect, Size};
use crate::status::{DecoderStatus, DecoderStatusCode};

use super::{map_uavs3d_error, split_annex_b_nals};

/// `uavs3d_cfg_t`（`native/uavs3d/source/decoder/uavs3d.h`）的镜像。
#[repr(C)]
struct Uavs3dCfg {
    input_file: *const c_char,
    output_file: *const c_char,
    frm_threads: c_int,
    dec_frames: c_int,
    log_level: c_int,
    check_md5: c_int,
}

/// `uavs3d_io_frm_t` 的镜像（只列 FFI 需要的字段，布局与 C 一致）。
///
/// 字段顺序、类型照抄头文件；`seqhdr` 用不透明指针（Rust 侧不读它）。
#[repr(C)]
struct Uavs3dIoFrm {
    priv_: *mut c_void,
    got_pic: c_int,
    num_plane: c_int,
    bit_depth: c_int,
    width: [c_int; 3],
    height: [c_int; 3],
    stride: [c_int; 3],
    buffer: [*mut c_void; 3],
    ptr: i64,
    pts: i64,
    dtr: i64,
    dts: i64,
    type_: c_int,
    refpic_num: [c_int; 2],
    refpic: [[i64; 16]; 2],
    pkt_pos: i64,
    pkt_size: c_int,
    bs: *mut u8,
    bs_len: c_int,
    nal_type: c_int,
    seqhdr: *mut c_void,
}

extern "C" {
    fn uavs3d_create(
        cfg: *mut Uavs3dCfg,
        callback: Option<extern "C" fn(*mut Uavs3dIoFrm)>,
        err: *mut c_int,
    ) -> *mut c_void;
    fn uavs3d_delete(handle: *mut c_void);
    fn uavs3d_reset(handle: *mut c_void);
    fn uavs3d_decode(handle: *mut c_void, frame: *mut Uavs3dIoFrm) -> c_int;
    fn uavs3d_flush(handle: *mut c_void, frame: *mut Uavs3dIoFrm) -> c_int;
}

thread_local! {
    /// 解码回调把输出帧放进这里，`decode` / `flush` 返回后立刻取走。
    ///
    /// C 侧 `uavs3d_output_frame()` 是在调用方的线程里同步调用回调的
    /// （`frm_threads = 1`，见 `Uavs3dDecoder::initialize`），所以线程局部
    /// 足够；不需要全局锁，也不会串到别的解码器上。
    static OUTPUT: RefCell<Vec<VideoFrame>> = const { RefCell::new(Vec::new()) };
}

extern "C" fn on_frame(frame: *mut Uavs3dIoFrm) {
    // SAFETY: `frame` 由 C 在本回调期间持有，字段都是它填好的值。
    // 图像缓冲归 C 管、回调返回后可能被复用，所以这里逐行拷成 Rust 自己的。
    let Some(width) = usize::try_from(unsafe { (*frame).width[0] }).ok() else {
        return;
    };
    let Some(height) = usize::try_from(unsafe { (*frame).height[0] }).ok() else {
        return;
    };
    if width == 0 || height == 0 || unsafe { (*frame).num_plane } != 2 {
        return;
    }
    // 本构建固定 8bit（build.rs 的 `COMPILE_10BIT=0`）。
    if unsafe { (*frame).bit_depth } != 8 {
        return;
    }

    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);
    let size = Size::new(width as u32, height as u32);
    let visible = Rect::new(0, 0, size.width, size.height);
    let pts = unsafe { (*frame).pts };

    let y = copy_plane(
        unsafe { (*frame).buffer[0] },
        unsafe { (*frame).stride[0] },
        width,
        height,
    );
    let uv = copy_plane(
        unsafe { (*frame).buffer[1] },
        unsafe { (*frame).stride[1] },
        chroma_width * 2,
        chroma_height,
    );
    let Some(y) = y else { return };
    let Some(uv) = uv else { return };

    if let Some(frame) = VideoFrame::from_cpu(
        VideoPixelFormat::NV12,
        size,
        visible,
        size,
        vec![y, uv],
        pts,
        0,
    ) {
        OUTPUT.with(|sink| sink.borrow_mut().push(frame));
    }
}

/// 从 C 的平面拷出紧排的字节：每行只读 `row_bytes`，跳过 `src_stride` 的填充。
fn copy_plane(src: *mut c_void, src_stride: i32, row_bytes: usize, rows: usize) -> Option<Vec<u8>> {
    if src.is_null() || src_stride <= 0 {
        return None;
    }
    let stride = usize::try_from(src_stride).ok()?;
    if stride < row_bytes {
        return None;
    }
    let mut out = vec![0u8; row_bytes.checked_mul(rows)?];
    for row in 0..rows {
        // SAFETY: C 保证这一行有 `stride` 字节可读（它就是按这个 stride 分配的）。
        let from = unsafe { src.cast::<u8>().add(row * stride) };
        let to = &mut out[row * row_bytes..(row + 1) * row_bytes];
        // SAFETY: `from..from+row_bytes` 与 `to` 不重叠（一个是 C 缓冲，一个是 Rust 新分配）。
        unsafe { std::ptr::copy_nonoverlapping(from, to.as_mut_ptr(), row_bytes) };
    }
    Some(out)
}

/// uavs3d 软件解码器（AVS3-P2 baseline，8bit 4:2:0 → NV12）。
pub struct Uavs3dDecoder {
    handle: *mut c_void,
    initialized: bool,
}

impl Default for Uavs3dDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Uavs3dDecoder {
    pub fn new() -> Self {
        Self {
            handle: std::ptr::null_mut(),
            initialized: false,
        }
    }

    /// 造一个空的输入帧（字段全 0，调用前再填 `bs` / `bs_len`）。
    fn empty_frame() -> Uavs3dIoFrm {
        Uavs3dIoFrm {
            priv_: std::ptr::null_mut(),
            got_pic: 0,
            num_plane: 0,
            bit_depth: 0,
            width: [0; 3],
            height: [0; 3],
            stride: [0; 3],
            buffer: [std::ptr::null_mut(); 3],
            ptr: 0,
            pts: 0,
            dtr: 0,
            dts: 0,
            type_: 0,
            refpic_num: [0; 2],
            refpic: [[0; 16]; 2],
            pkt_pos: 0,
            pkt_size: 0,
            bs: std::ptr::null_mut(),
            bs_len: 0,
            nal_type: 0,
            seqhdr: std::ptr::null_mut(),
        }
    }

    /// 调一次 C 解码器并取走这一调用产生的帧。
    fn call(
        &mut self,
        nal: &[u8],
        timestamp: i64,
        flush: bool,
    ) -> Result<Vec<VideoFrame>, DecoderStatus> {
        OUTPUT.with(|sink| sink.borrow_mut().clear());

        let mut frame = Self::empty_frame();
        frame.pts = timestamp;
        frame.bs = nal.as_ptr() as *mut u8;
        frame.bs_len = nal.len() as c_int;

        // SAFETY: `handle` 在 `initialize` 成功后有效，且 `&mut self` 保证
        // 同一时刻只有一个调用；`frame` 是合法的 C 结构体镜像。
        let ret = if flush {
            unsafe { uavs3d_flush(self.handle, &mut frame) }
        } else {
            unsafe { uavs3d_decode(self.handle, &mut frame) }
        };
        if let Some(status) = map_uavs3d_error(ret) {
            return Err(status);
        }

        Ok(OUTPUT.with(|sink| std::mem::take(&mut *sink.borrow_mut())))
    }
}

impl VideoDecoder for Uavs3dDecoder {
    fn initialize(&mut self, config: &VideoDecoderConfig, _low_delay: bool) -> DecoderStatus {
        if config.codec() != crate::codec::VideoCodec::Av3 {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedCodec,
                "uavs3d only decodes AVS3",
            );
        }
        if !config.is_valid_config() {
            return DecoderStatus::new(DecoderStatusCode::UnsupportedConfig);
        }
        let mut cfg = Uavs3dCfg {
            input_file: std::ptr::null(),
            output_file: std::ptr::null(),
            // 单线程：回调就在 `decode()` 的调用线程上同步发生，
            // 这样线程局部的帧池不会漏掉任何输出。
            frm_threads: 1,
            dec_frames: 0,
            log_level: 0,
            check_md5: 0,
        };
        let mut err = 0;
        // SAFETY: `cfg` 与 `err` 都是栈上的合法指针；回调是无捕获函数指针。
        let handle = unsafe { uavs3d_create(&mut cfg, Some(on_frame), &mut err) };
        if handle.is_null() {
            return DecoderStatus::with_message(
                DecoderStatusCode::FailedToCreateDecoder,
                format!("uavs3d_create failed ({err})"),
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
        if buffer.is_end_of_stream() {
            return self.call(&[], buffer.timestamp(), true);
        }

        let mut frames = Vec::new();
        for nal in split_annex_b_nals(buffer.data()) {
            frames.extend(self.call(nal, buffer.timestamp(), false)?);
        }
        Ok(frames)
    }

    fn reset(&mut self) {
        if self.initialized {
            // SAFETY: `handle` 有效；reset 清掉 C 侧状态，不动对象本身。
            unsafe { uavs3d_reset(self.handle) };
        }
        OUTPUT.with(|sink| sink.borrow_mut().clear());
    }

    fn needs_bitstream_conversion(&self) -> bool {
        false
    }

    fn decoder_type(&self) -> VideoDecoderType {
        VideoDecoderType::Uavs3d
    }
}

impl Drop for Uavs3dDecoder {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: `handle` 只在 `initialize` 成功后赋值一次，这里也不会重复析构
            // （`Drop` 只跑一次），之后对象即消失。
            unsafe { uavs3d_delete(self.handle) };
            self.initialized = false;
        }
    }
}
