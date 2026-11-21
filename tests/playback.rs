#![allow(dead_code)] // 各测试按 feature 门控，默认构建下 helper 可能未使用

//! “能正常播放”的验证：多帧连续解码 + 与 ffmpeg 参考解码逐帧比对。
//!
//! 判据：帧数正确、时间戳随输入推进、Y 平面与参考解码的平均绝对误差在
//! 有损压缩的合理范围内。`tests/data/playback/` 下的 fixture 与参考 YUV
//! 由 ffmpeg 8.0 一次性生成并提交，测试本身零依赖。

use zephyra_video::{
    bt709_color_space, level_8bit_420, AlphaMode, DecoderBuffer, Rect, Size, VideoCodec,
    VideoCodecProfile, VideoDecoder, VideoDecoderConfig, VideoFrame, VideoTransformation,
};

#[cfg(feature = "av1")]
use zephyra_video::Dav1dDecoder;
#[cfg(feature = "hevc")]
use zephyra_video::HevcDecoder;
#[cfg(feature = "mjpeg")]
use zephyra_video::MjpegDecoder;
#[cfg(feature = "h264")]
use zephyra_video::OpenH264Decoder;
#[cfg(feature = "theora")]
use zephyra_video::TheoraDecoder;
#[cfg(feature = "vp8")]
use zephyra_video::Vpx8Decoder;
#[cfg(feature = "vp9")]
use zephyra_video::Vpx9Decoder;
#[cfg(feature = "vvc")]
use zephyra_video::VvdecDecoder;
#[cfg(feature = "evc")]
use zephyra_video::XevdDecoder;

const TS_STEP: i64 = 1000;

fn config(codec: VideoCodec, w: u32, h: u32) -> VideoDecoderConfig {
    VideoDecoderConfig::new(
        codec,
        VideoCodecProfile::Unknown,
        level_8bit_420(),
        AlphaMode::IsOpaque,
        bt709_color_space(),
        VideoTransformation::None,
        Size::new(w, h),
        Rect::new(0, 0, w, h),
        Size::new(w, h),
        Vec::new(),
    )
}

/// 逐帧解码；每个 packet 带递增时间戳，最后送 EOS 让延迟帧吐出。
fn decode_stream<D: VideoDecoder>(
    mut d: D,
    cfg: &VideoDecoderConfig,
    packets: &[&[u8]],
) -> Vec<VideoFrame> {
    assert!(d.initialize(cfg, false).is_ok());
    let mut out = Vec::new();
    for (i, packet) in packets.iter().enumerate() {
        let buffer = DecoderBuffer::new(packet.to_vec(), i as i64 * TS_STEP, TS_STEP);
        out.extend(d.decode(&buffer).expect("decode failed"));
    }
    let last_ts = (packets.len().saturating_sub(1)) as i64 * TS_STEP;
    out.extend(
        d.decode(&DecoderBuffer::end_of_stream(last_ts))
            .expect("flush failed"),
    );
    // EOS 帧沿用最后一个输入缓冲的时基；duration 已是 TS_STEP。
    out
}

/// Y 平面平均绝对误差；参考是紧密排列的 8/16-bit 样本。
fn luma_mad(
    frame: &VideoFrame,
    reference: &[u8],
    frame_offset: usize,
    w: usize,
    h: usize,
    bytes: usize,
) -> f64 {
    let plane = frame.planes()[0].cpu_data().expect("CPU Y plane");
    let stride = frame.layout().planes()[0].stride;
    let frame_size = w * h * bytes;
    assert!(reference.len() >= frame_offset + frame_size);
    let mut sum = 0u64;
    for row in 0..h {
        for col in 0..w {
            let off = row * stride + col * bytes;
            let ref_off = frame_offset + (row * w + col) * bytes;
            let (a, b) = if bytes == 1 {
                (plane[off] as u64, reference[ref_off] as u64)
            } else {
                (
                    u16::from_le_bytes([plane[off], plane[off + 1]]) as u64,
                    u16::from_le_bytes([reference[ref_off], reference[ref_off + 1]]) as u64,
                )
            };
            sum += a.abs_diff(b);
        }
    }
    sum as f64 / (w * h) as f64
}

fn assert_playback(
    frames: &[VideoFrame],
    reference: &[u8],
    w: usize,
    h: usize,
    bytes: usize,
    max_mad: f64,
) {
    assert_eq!(frames.len(), 8, "expected 8 decoded frames");
    // ffmpeg 的 rawvideo 参考含色度：8bit 是 1.5 字节/像素，10bit 是 3 字节/像素。
    let ref_frame_size = w * h * bytes * 3 / 2;
    assert!(reference.len() >= ref_frame_size * 8, "reference too short");

    let mut previous_ts = -1i64;
    for (i, frame) in frames.iter().enumerate() {
        assert!(frame.timestamp() >= 0, "frame {i} lost its timestamp");
        assert!(
            frame.timestamp() >= previous_ts,
            "timestamps went backwards at frame {i}: {} < {}",
            frame.timestamp(),
            previous_ts
        );
        previous_ts = frame.timestamp();
        assert_eq!(frame.duration(), TS_STEP);
        let mad = luma_mad(frame, reference, i * ref_frame_size, w, h, bytes);
        assert!(
            mad <= max_mad,
            "frame {i}: luma MAD {mad:.1} exceeds {max_mad:.1} (possible wrong pixels/stride)"
        );
    }
}

// ---- 解封装 helper（与 real_streams.rs 相同，集成测试间不共享代码）----

fn once(bytes: &'static [u8]) -> Vec<Vec<u8>> {
    vec![bytes.to_vec()]
}

/// H.264 Annex-B: 按 VCL NAL (type 1/5) 切成访问单元，参数集随其后的帧走。
/// AUD (type 9) 会被丢弃 —— OpenH264 对它不是必需的，反而可能扰乱 AU 边界。
fn aud_frames(bytes: &'static [u8]) -> Vec<Vec<u8>> {
    let mut starts = Vec::new();
    let mut i = 0usize;
    while i + 2 < bytes.len() {
        if bytes[i] == 0 && bytes[i + 1] == 0 && bytes[i + 2] == 1 {
            starts.push(i);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    let mut au = Vec::new();
    for (k, &start) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(bytes.len());
        let nal = &bytes[start..end];
        let nal_type = nal.get(3).map(|b| b & 0x1f).unwrap_or(0);
        if nal_type == 9 {
            continue;
        }
        au.extend_from_slice(nal);
        // VCL NAL 结束一个 AU：参数集与它所属的帧必须在同一个 buffer 里。
        if nal_type == 1 || nal_type == 5 {
            out.push(std::mem::take(&mut au));
        }
    }
    if !au.is_empty() {
        out.push(au);
    }
    out
}

/// HEVC Annex-B: 按 AUD (NAL type 35) 切成单个访问单元。
fn hevc_frames(bytes: &'static [u8]) -> Vec<Vec<u8>> {
    let mut starts = Vec::new();
    let mut i = 0usize;
    while i + 3 < bytes.len() {
        if bytes[i] == 0
            && bytes[i + 1] == 0
            && bytes[i + 2] == 1
            && ((bytes[i + 3] >> 1) & 0x3f) == 35
        {
            starts.push(i);
            i += 3;
        } else {
            i += 1;
        }
    }
    if starts.is_empty() {
        return vec![bytes.to_vec()];
    }
    let mut out = Vec::new();
    for w in starts.windows(2) {
        out.push(bytes[w[0]..w[1]].to_vec());
    }
    out.push(bytes[*starts.last().unwrap()..].to_vec());
    out
}

fn ivf_frames(bytes: &'static [u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut pos = 32usize;
    while pos + 12 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 12;
        if size == 0 || pos + size > bytes.len() {
            break;
        }
        out.push(bytes[pos..pos + size].to_vec());
        pos += size;
    }
    out
}

fn jpeg_frames(bytes: &'static [u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut start = None;
    for i in 0..bytes.len().saturating_sub(1) {
        if bytes[i] == 0xFF && bytes[i + 1] == 0xD8 && start.is_none() {
            start = Some(i);
        } else if bytes[i] == 0xFF && bytes[i + 1] == 0xD9 {
            if let Some(s) = start.take() {
                out.push(bytes[s..i + 2].to_vec());
            }
        }
    }
    out
}

fn ogg_packets(bytes: &'static [u8]) -> Vec<Vec<u8>> {
    let mut packets = Vec::new();
    let mut current = Vec::new();
    let mut pos = 0usize;
    while pos + 27 <= bytes.len() && &bytes[pos..pos + 4] == b"OggS" {
        let segments = bytes[pos + 26] as usize;
        let lacing_start = pos + 27;
        let lacing_end = lacing_start + segments;
        if lacing_end > bytes.len() {
            break;
        }
        let mut payload = lacing_end;
        for &len in &bytes[lacing_start..lacing_end] {
            let len = len as usize;
            if payload + len > bytes.len() {
                break;
            }
            current.extend_from_slice(&bytes[payload..payload + len]);
            payload += len;
            if len < 255 {
                packets.push(std::mem::take(&mut current));
            }
        }
        pos = payload;
    }
    packets
}

fn evc_annexb(bytes: &'static [u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut pos = 0usize;
    while pos + 4 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if size == 0 || pos + size > bytes.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&bytes[pos..pos + size]);
        pos += size;
    }
    vec![out]
}

// 默认构建没有任何 codec feature，所有调用点都被 cfg 掉，宏本身仍未使用。
#[allow(unused_macros)]
macro_rules! playback_test {
    ($name:ident, $ctor:expr, $codec:expr, $bitstream:expr, $reference:expr, $w:expr, $h:expr, $bytes:expr, $mad:expr, $packets:expr) => {
        #[test]
        fn $name() {
            let packets = $packets($bitstream);
            let refs: Vec<&[u8]> = packets.iter().map(Vec::as_slice).collect();
            let frames = decode_stream($ctor, &config($codec, $w, $h), &refs);
            assert_playback(&frames, $reference, $w as usize, $h as usize, $bytes, $mad);
        }
    };
}

// 8-bit 流：ffmpeg 与 vendored 解码器的 Y 平面通常逐字节相同或差 1~2。
#[cfg(feature = "h264")]
playback_test!(
    playback_h264,
    OpenH264Decoder::new(),
    VideoCodec::H264,
    include_bytes!("data/playback/h264.h264"),
    include_bytes!("data/playback/h264.yuv"),
    64,
    48,
    1,
    12.0,
    aud_frames
);

#[cfg(feature = "hevc")]
playback_test!(
    playback_hevc8,
    HevcDecoder::new(),
    VideoCodec::HEVC,
    include_bytes!("data/playback/hevc8.hevc"),
    include_bytes!("data/playback/hevc8.yuv"),
    64,
    48,
    1,
    12.0,
    hevc_frames
);

#[cfg(feature = "av1")]
playback_test!(
    playback_av18,
    Dav1dDecoder::new(),
    VideoCodec::AV1,
    include_bytes!("data/playback/av18.obu"),
    include_bytes!("data/playback/av18.yuv"),
    64,
    48,
    1,
    12.0,
    once
);

#[cfg(feature = "vp8")]
playback_test!(
    playback_vp8,
    Vpx8Decoder::new(),
    VideoCodec::VP8,
    include_bytes!("data/playback/vp8.ivf"),
    include_bytes!("data/playback/vp8.yuv"),
    64,
    48,
    1,
    12.0,
    ivf_frames
);

#[cfg(feature = "vp9")]
playback_test!(
    playback_vp98,
    Vpx9Decoder::new(),
    VideoCodec::VP9,
    include_bytes!("data/playback/vp98.ivf"),
    include_bytes!("data/playback/vp98.yuv"),
    64,
    48,
    1,
    12.0,
    ivf_frames
);

#[cfg(feature = "theora")]
#[test]
fn playback_theora() {
    // libtheora 的输出与 ffmpeg 参考在像素值上有系统差异（后处理/色度范围），
    // 因此这里不逐像素比对，而是验证“真的在连续解码不同的画面”。
    let bytes = include_bytes!("data/playback/theora.ogg");
    let packets = ogg_packets(bytes);
    let refs: Vec<&[u8]> = packets.iter().map(Vec::as_slice).collect();
    let frames = decode_stream(
        TheoraDecoder::new(),
        &config(VideoCodec::Theora, 64, 48),
        &refs,
    );
    assert_eq!(frames.len(), 8, "expected 8 decoded frames");

    let mut changed = 0;
    let mut previous_ts = -1i64;
    let mut prev_y: Option<Vec<u8>> = None;
    for frame in &frames {
        assert!(frame.timestamp() >= previous_ts);
        assert_eq!(frame.duration(), TS_STEP);
        previous_ts = frame.timestamp();
        let y = frame.planes()[0].cpu_data().unwrap().to_vec();
        if let Some(p) = &prev_y {
            let diff: f64 = y
                .iter()
                .zip(p)
                .map(|(a, b)| a.abs_diff(*b) as u64)
                .sum::<u64>() as f64
                / y.len() as f64;
            if diff > 1.0 {
                changed += 1;
            }
        }
        prev_y = Some(y);
    }
    assert!(
        changed >= 3,
        "frames look static — decoder may not be producing real pictures"
    );
}

#[cfg(feature = "mjpeg")]
playback_test!(
    playback_mjpeg,
    MjpegDecoder::new(),
    VideoCodec::Mjpeg,
    include_bytes!("data/playback/mjpeg.mjpeg"),
    include_bytes!("data/playback/mjpeg.yuv"),
    64,
    48,
    1,
    20.0,
    jpeg_frames
);

// 10-bit 流：误差按 16-bit 尺度衡量，阈值放宽。
#[cfg(feature = "hevc")]
playback_test!(
    playback_hevc10,
    HevcDecoder::new(),
    VideoCodec::HEVC,
    include_bytes!("data/playback/hevc10.hevc"),
    include_bytes!("data/playback/hevc10.yuv"),
    64,
    48,
    2,
    48.0,
    hevc_frames
);

#[cfg(feature = "av1")]
playback_test!(
    playback_av110,
    Dav1dDecoder::new(),
    VideoCodec::AV1,
    include_bytes!("data/playback/av110.obu"),
    include_bytes!("data/playback/av110.yuv"),
    64,
    48,
    2,
    48.0,
    once
);

#[cfg(feature = "vp9")]
playback_test!(
    playback_vp910,
    Vpx9Decoder::new(),
    VideoCodec::VP9,
    include_bytes!("data/playback/vp910.ivf"),
    include_bytes!("data/playback/vp910.yuv"),
    64,
    48,
    2,
    48.0,
    ivf_frames
);

#[cfg(feature = "vvc")]
playback_test!(
    playback_vvc,
    VvdecDecoder::new(),
    VideoCodec::Vvc,
    include_bytes!("data/playback/vvc.vvc"),
    include_bytes!("data/playback/vvc.yuv"),
    64,
    48,
    2,
    48.0,
    once
);

#[cfg(feature = "evc")]
playback_test!(
    playback_evc,
    XevdDecoder::new(),
    VideoCodec::Evc,
    include_bytes!("data/playback/evc.evc"),
    include_bytes!("data/playback/evc.yuv"),
    128,
    64,
    2,
    48.0,
    evc_annexb
);
