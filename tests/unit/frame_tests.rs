//! `frame.rs` 的断言：像素格式、平面布局、帧的两种形态（CPU / 原生）。

use super::super::{
    ColorPlaneLayout, Rect, Size, VideoFrame, VideoFrameLayout, VideoPixelFormat, VideoPlane,
};

#[test]
fn plane_counts_match_chromium() {
    assert_eq!(VideoPixelFormat::Unknown.num_planes(), 0);
    assert_eq!(VideoPixelFormat::I420.num_planes(), 3);
    assert_eq!(VideoPixelFormat::YV12.num_planes(), 3);
    assert_eq!(VideoPixelFormat::NV12.num_planes(), 2);
    assert_eq!(VideoPixelFormat::NV21.num_planes(), 2);
    assert_eq!(VideoPixelFormat::I444.num_planes(), 3);
    assert_eq!(VideoPixelFormat::UYVY.num_planes(), 1);
    assert_eq!(VideoPixelFormat::ARGB.num_planes(), 1);
    assert_eq!(VideoPixelFormat::BGRA.num_planes(), 1);
    assert_eq!(VideoPixelFormat::I420A.num_planes(), 4);
    assert_eq!(VideoPixelFormat::NV12A.num_planes(), 3);
}

#[test]
fn half_plane_formats_are_two_planes() {
    // 半平面也是**两个**平面：Y 一个、交错的 UV 一个。
    assert_eq!(VideoPixelFormat::NV12.num_planes(), 2);
    assert_eq!(VideoPixelFormat::P010LE.num_planes(), 2);
    assert_eq!(VideoPixelFormat::NV16.num_planes(), 2);
}

#[test]
fn is_420_covers_the_four_420_layouts() {
    assert!(VideoPixelFormat::I420.is_420());
    assert!(VideoPixelFormat::YV12.is_420());
    assert!(VideoPixelFormat::NV12.is_420());
    assert!(VideoPixelFormat::NV21.is_420());
    assert!(!VideoPixelFormat::I422.is_420());
    assert!(!VideoPixelFormat::I444.is_420());
}

#[test]
fn bytes_per_pixel_is_the_average_for_planar_formats() {
    assert_eq!(VideoPixelFormat::I420.bytes_per_pixel(), 1.5);
    assert_eq!(VideoPixelFormat::NV12.bytes_per_pixel(), 1.5);
    assert_eq!(VideoPixelFormat::I422.bytes_per_pixel(), 2.0);
    assert_eq!(VideoPixelFormat::I444.bytes_per_pixel(), 3.0);
    assert_eq!(VideoPixelFormat::ARGB.bytes_per_pixel(), 4.0);
    // 没算的格式给 0，而不是猜一个。
    assert_eq!(VideoPixelFormat::MJPEG.bytes_per_pixel(), 0.0);
}

#[test]
fn i420_layout_has_three_contiguous_planes() {
    let layout = VideoFrameLayout::create(VideoPixelFormat::I420, Size::new(64, 32)).unwrap();
    assert_eq!(layout.num_planes(), 3);
    assert_eq!(
        layout.planes()[0],
        ColorPlaneLayout {
            offset: 0,
            stride: 64
        }
    );
    assert_eq!(
        layout.planes()[1],
        ColorPlaneLayout {
            offset: 64 * 32,
            stride: 32
        }
    );
    assert_eq!(
        layout.planes()[2],
        ColorPlaneLayout {
            offset: 64 * 32 + 32 * 16,
            stride: 32
        }
    );
}

#[test]
fn nv12_layout_has_two_planes() {
    let layout = VideoFrameLayout::create(VideoPixelFormat::NV12, Size::new(64, 32)).unwrap();
    assert_eq!(layout.num_planes(), 2);
    assert_eq!(layout.planes()[0].stride, 64);
    // 交错的 UV：宽是亮度的一半，行高也是一半，但一行里有两个字节。
    assert_eq!(layout.planes()[1].offset, 64 * 32);
    assert_eq!(layout.planes()[1].stride, 64);
}

#[test]
fn odd_width_uses_ceil_for_chroma() {
    let layout = VideoFrameLayout::create(VideoPixelFormat::I420, Size::new(65, 33)).unwrap();
    assert_eq!(layout.planes()[1].stride, 33);
    assert_eq!(layout.planes()[2].offset, 65 * 33 + 33 * 17);
}

#[test]
fn unknown_format_or_empty_size_has_no_layout() {
    assert!(VideoFrameLayout::create(VideoPixelFormat::Unknown, Size::new(64, 32)).is_none());
    assert!(VideoFrameLayout::create(VideoPixelFormat::I420, Size::new(0, 32)).is_none());
    assert!(VideoFrameLayout::create(VideoPixelFormat::I420, Size::new(64, 0)).is_none());
}

#[test]
fn unsupported_formats_have_no_layout_rather_than_a_wrong_one() {
    // 「猜一个错的 stride」比「没有布局」糟得多。
    assert!(VideoFrameLayout::create(VideoPixelFormat::UYVY, Size::new(64, 32)).is_none());
    assert!(VideoFrameLayout::create(VideoPixelFormat::I422, Size::new(64, 32)).is_none());
}

#[test]
fn required_buffer_size_covers_the_last_plane() {
    let layout = VideoFrameLayout::create(VideoPixelFormat::I420, Size::new(64, 32)).unwrap();
    // 64*32 + 32*16 + 32*16
    assert_eq!(layout.required_buffer_size(), 64 * 32 + 32 * 16 * 2);

    let nv12 = VideoFrameLayout::create(VideoPixelFormat::NV12, Size::new(64, 32)).unwrap();
    // 64*32 + 64*16
    assert_eq!(nv12.required_buffer_size(), 64 * 32 + 64 * 16);
}

#[test]
fn cpu_frame_accepts_matching_planes() {
    let size = Size::new(4, 4);
    let y = vec![0u8; 16];
    let u = vec![0u8; 4];
    let v = vec![0u8; 4];
    let frame = VideoFrame::from_cpu(
        VideoPixelFormat::I420,
        size,
        Rect::new(0, 0, 4, 4),
        size,
        vec![y, u, v],
        100,
        33,
    )
    .unwrap();
    assert!(frame.is_cpu());
    assert_eq!(frame.num_planes(), 3);
    assert_eq!(frame.timestamp(), 100);
    assert_eq!(frame.duration(), 33);
    assert_eq!(frame.format(), VideoPixelFormat::I420);
    assert_eq!(frame.coded_size(), size);
    assert!(frame.planes().iter().all(VideoPlane::is_cpu));
}

#[test]
fn cpu_frame_rejects_wrong_plane_count() {
    let size = Size::new(4, 4);
    assert!(VideoFrame::from_cpu(
        VideoPixelFormat::I420,
        size,
        Rect::new(0, 0, 4, 4),
        size,
        vec![vec![0; 16], vec![0; 4]],
        0,
        0
    )
    .is_none());
}

#[test]
fn cpu_frame_rejects_short_plane_data() {
    let size = Size::new(4, 4);
    assert!(VideoFrame::from_cpu(
        VideoPixelFormat::I420,
        size,
        Rect::new(0, 0, 4, 4),
        size,
        vec![vec![0; 15], vec![0; 4], vec![0; 4]],
        0,
        0
    )
    .is_none());
}

#[test]
fn native_frame_carries_opaque_handles() {
    let size = Size::new(4, 4);
    let frame = VideoFrame::from_native(
        VideoPixelFormat::NV12,
        size,
        Rect::new(0, 0, 4, 4),
        size,
        vec![0xAAAA, 0xBBBB],
        7,
        0,
    )
    .unwrap();
    assert!(!frame.is_cpu());
    assert_eq!(frame.num_planes(), 2);
    match frame.planes()[0] {
        VideoPlane::Native { handle } => assert_eq!(handle, 0xAAAA),
        VideoPlane::Cpu { .. } => panic!("expected a native plane"),
    }
    assert!(frame.planes()[0].cpu_data().is_none());
}

#[test]
fn native_frame_rejects_wrong_handle_count() {
    let size = Size::new(4, 4);
    assert!(VideoFrame::from_native(
        VideoPixelFormat::NV12,
        size,
        Rect::new(0, 0, 4, 4),
        size,
        vec![1],
        0,
        0
    )
    .is_none());
}

#[test]
fn layout_is_reachable_from_the_frame() {
    let size = Size::new(8, 8);
    let frame = VideoFrame::from_cpu(
        VideoPixelFormat::NV12,
        size,
        Rect::new(0, 0, 8, 8),
        size,
        vec![vec![0; 64], vec![0; 32]],
        0,
        0,
    )
    .unwrap();
    assert_eq!(frame.layout().format(), VideoPixelFormat::NV12);
    assert_eq!(frame.layout().num_planes(), 2);
}
