//! AVS3（UAVS3D）解码的 Rust 侧：Annex-B 切分、状态码映射与解码器封装。
//!
//! # 与 vendored C 源码的关系
//!
//! `native/uavs3d/` 是 [`uavs3d`](https://github.com/uavs3/uavs3) 的**逐字节副本**
//! （BSD-3 许可，见 `native/uavs3d/COPYING`）。本模块只做两件事：
//!
//! 1. 把 Annex-B 码流切成 NAL —— uavs3d 的 API **一次只吃一个 NAL**，
//!    这一点与 `test/utest.c::find_next_start_code()` 对得上；
//! 2. 用 `#[repr(C)]` 镜像它的两个公开结构，经 FFI 调用
//!    `uavs3d_create` / `uavs3d_decode` / `uavs3d_flush` / `uavs3d_delete`。
//!
//! 切分与状态映射是**纯逻辑**，默认构建就能单测；真正碰 C 的那一半在
//! `imp` 里，只有 `uavs3d` feature 打开才编译。

#[cfg(any(feature = "uavs3d", test))]
use crate::status::{DecoderStatus, DecoderStatusCode};

#[cfg(feature = "uavs3d")]
#[allow(unsafe_code)]
mod imp;
#[cfg(feature = "uavs3d")]
pub use imp::Uavs3dDecoder;

/// AVS3 NAL 类型字节（起始码 `00 00 01` 之后的第一个字节）。
///
/// 与 `native/uavs3d/source/decoder/uavs3d.c::uavs3d_decode()` 的分支一致：
/// 序列头 `0xB0`、序列尾 `0xB1`、图像头 `0xB3` / `0xB6`、切片 `0x00..=0x8E`。
pub(crate) fn is_nal_type(byte: u8) -> bool {
    byte <= 0x8E || matches!(byte, 0xB0 | 0xB1 | 0xB3 | 0xB6)
}

/// 把 Annex-B 码流切成若干 NAL，每个 NAL **保留三字节起始码**
/// （`00 00 01`），因为 uavs3d 的 `uavs3d_decode()` 直接读 `bs[3]` 当 NAL 类型。
///
/// 对应 `native/uavs3d/test/utest.c::find_next_start_code()`：它在缓冲区开头
/// 自己写 `00 00 01`，所以四字节起始码前面的那个 0 会被自然丢掉 —— 本函数
/// 从一个已有的字节切片里切，效果相同。
///
/// 切分只认合法 NAL 类型字节（`[`is_nal_type`]`），所以 RBSP 里偶然出现的
/// `00 00 01 xx` 不会被误当成边界（xx 不是 NAL 类型）。
pub fn split_annex_b_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0usize;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 && is_nal_type(data[i + 3]) {
            starts.push(i);
            i += 4;
        } else {
            i += 1;
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(index, &start)| {
            let end = starts.get(index + 1).copied().unwrap_or(data.len());
            &data[start..end]
        })
        .collect()
}

/// uavs3d 的返回值 → 本 crate 的解码状态。
///
/// `0` / `1`（延迟）/ `2`（序列结束）都是成功，返回 `None`；负数是错误。
/// 数字与 `native/uavs3d/source/decoder/uavs3d.h` 的 `ERR_*` 宏一一对应
/// （那个头文件只给宏不给名字表，所以这里照抄符号名以便复核）。
#[cfg(any(feature = "uavs3d", test))]
pub(crate) fn map_uavs3d_error(ret: i32) -> Option<DecoderStatus> {
    if ret >= 0 {
        return None;
    }
    let (code, name) = match ret {
        -1 => (DecoderStatusCode::OutOfMemory, "ERR_OUT_OF_MEMORY"),
        -2 => (DecoderStatusCode::Failed, "ERR_UNEXPECTED"),
        -3 => (DecoderStatusCode::UnsupportedConfig, "ERR_BAD_BITDEPTH"),
        -4 => (DecoderStatusCode::MalformedBitstream, "ERR_UNKNOWN_NAL"),
        -5 => (DecoderStatusCode::MalformedBitstream, "ERR_LOSS_REF_FRAME"),
        -6 => (
            DecoderStatusCode::MalformedBitstream,
            "ERR_PIC_HEADER_UNREADY",
        ),
        -7 => (
            DecoderStatusCode::MalformedBitstream,
            "ERR_SEQ_HEADER_UNREADY",
        ),
        -8 => (
            DecoderStatusCode::UnsupportedConfig,
            "ERR_SEQ_MAININFO_CHANGED",
        ),
        -9 => (DecoderStatusCode::OutOfMemory, "ERR_PIC_BUFFER_FULL"),
        -100 => (DecoderStatusCode::Failed, "ERR_UNKNOWN"),
        _ => (DecoderStatusCode::Failed, "unknown uavs3d error"),
    };
    Some(DecoderStatus::with_message(
        code,
        format!("uavs3d: {name} ({ret})"),
    ))
}

#[cfg(test)]
#[path = "../../tests/unit/uavs3d_tests.rs"]
mod tests;
