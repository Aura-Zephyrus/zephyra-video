#![allow(dead_code)] // 各测试按 feature 门控，默认构建下 helper 可能未使用

//! 真实码流解码测试：用 ffmpeg 预生成的极小 fixture 验证各后端真的能出帧。
//!
//! fixture 在 `tests/data/`，由 ffmpeg 8.0 一次性生成并提交，测试本身零依赖。
//! 每个测试用 `#[cfg(feature = ...)]` 门控，避免默认构建引入任何 C 解码器。

use zephyra_video::{
    bt709_color_space, level_8bit_420, AlphaMode, DecoderBuffer, Rect, Size, VideoCodec,
    VideoCodecProfile, VideoDecoder, VideoDecoderConfig, VideoPixelFormat, VideoTransformation,
};

// ---- 各后端（feature 门控）----
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

fn assert_frames(frames: &[zephyra_video::VideoFrame], format: VideoPixelFormat, w: u32, h: u32) {
    assert!(!frames.is_empty(), "decoder returned no frames");
    for frame in frames {
        assert_eq!(frame.format(), format);
        assert!(frame.coded_size().width >= w);
        assert!(frame.coded_size().height >= h);
        assert_eq!(frame.planes().len(), format.num_planes());
        for plane in frame.planes() {
            let data = plane.cpu_data().expect("expected a CPU plane");
            assert!(!data.is_empty());
            assert!(
                data.iter().any(|&b| b != 0),
                "plane is all zeros — suspicious decode"
            );
        }
    }
}

fn decode_all<D: VideoDecoder>(
    mut decoder: D,
    cfg: &VideoDecoderConfig,
    packets: &[&[u8]],
) -> Vec<zephyra_video::VideoFrame> {
    assert!(decoder.initialize(cfg, false).is_ok());
    let mut out = Vec::new();
    for packet in packets {
        let buffer = DecoderBuffer::new(packet.to_vec(), 0, 0);
        out.extend(decoder.decode(&buffer).expect("decode failed"));
    }
    // EOS makes delayed-frame decoders (e.g. VVC) flush their queue.
    out.extend(
        decoder
            .decode(&DecoderBuffer::end_of_stream(0))
            .expect("flush failed"),
    );
    out
}

fn once(bytes: &'static [u8]) -> Vec<&'static [u8]> {
    vec![bytes]
}

/// 解码 → reset → 再解码同一段流：验证 C 侧状态真的被复位了。
fn assert_reset_roundtrip<D: VideoDecoder>(
    mut decoder: D,
    cfg: &VideoDecoderConfig,
    packets: &[&[u8]],
    format: VideoPixelFormat,
    w: u32,
    h: u32,
) {
    assert!(decoder.initialize(cfg, false).is_ok());
    for packet in packets {
        let buffer = DecoderBuffer::new(packet.to_vec(), 0, 0);
        let frames = decoder.decode(&buffer).expect("first decode failed");
        if !frames.is_empty() {
            assert_frames(&frames, format, w, h);
        }
    }
    decoder.reset();
    let mut second = Vec::new();
    for packet in packets {
        let buffer = DecoderBuffer::new(packet.to_vec(), 0, 0);
        second.extend(decoder.decode(&buffer).expect("second decode failed"));
    }
    second.extend(
        decoder
            .decode(&DecoderBuffer::end_of_stream(0))
            .expect("second flush failed"),
    );
    assert_frames(&second, format, w, h);
}

/// IVF: 32-byte file header, then 12-byte frame headers (u32 size + u64 pts).
fn ivf_frames(bytes: &'static [u8]) -> Vec<&'static [u8]> {
    let mut out = Vec::new();
    let mut pos = 32usize;
    while pos + 12 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 12;
        if size == 0 || pos + size > bytes.len() {
            break;
        }
        out.push(&bytes[pos..pos + size]);
        pos += size;
    }
    out
}

/// MJPEG: concatenated JPEG frames, each from FFD8 to the following FFD9.
fn jpeg_frames(bytes: &'static [u8]) -> Vec<&'static [u8]> {
    let mut out = Vec::new();
    let mut start = None;
    for i in 0..bytes.len().saturating_sub(1) {
        if bytes[i] == 0xFF && bytes[i + 1] == 0xD8 && start.is_none() {
            start = Some(i);
        } else if bytes[i] == 0xFF && bytes[i + 1] == 0xD9 {
            if let Some(s) = start.take() {
                out.push(&bytes[s..i + 2]);
            }
        }
    }
    out
}

/// ffmpeg's raw EVC muxer writes 4-byte big-endian length prefixes; xevd
/// expects Annex-B start codes. Convert one to the other.
fn evc_annexb(bytes: &'static [u8]) -> Vec<u8> {
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
    out
}

/// Ogg: reassemble packets from pages using the lacing-value table.
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

#[cfg(feature = "h264")]
#[test]
fn decodes_real_h264() {
    let bytes = include_bytes!("data/h264_8.h264");
    let frames = decode_all(
        OpenH264Decoder::new(),
        &config(VideoCodec::H264, 64, 48),
        &once(bytes),
    );
    assert_frames(&frames, VideoPixelFormat::I420, 64, 48);
}

#[cfg(feature = "hevc")]
#[test]
fn decodes_real_hevc_8bit() {
    let bytes = include_bytes!("data/hevc_8.hevc");
    let frames = decode_all(
        HevcDecoder::new(),
        &config(VideoCodec::HEVC, 64, 48),
        &once(bytes),
    );
    assert_frames(&frames, VideoPixelFormat::I420, 64, 48);
}

#[cfg(feature = "hevc")]
#[test]
fn decodes_real_hevc_10bit() {
    let bytes = include_bytes!("data/hevc_10.hevc");
    let frames = decode_all(
        HevcDecoder::new(),
        &config(VideoCodec::HEVC, 64, 48),
        &once(bytes),
    );
    assert_frames(&frames, VideoPixelFormat::YUV420P10, 64, 48);
}

#[cfg(feature = "av1")]
#[test]
fn decodes_real_av1_8bit() {
    let bytes = include_bytes!("data/av1_8.obu");
    let frames = decode_all(
        Dav1dDecoder::new(),
        &config(VideoCodec::AV1, 64, 48),
        &once(bytes),
    );
    assert_frames(&frames, VideoPixelFormat::I420, 64, 48);
}

#[cfg(feature = "av1")]
#[test]
fn decodes_real_av1_10bit() {
    let bytes = include_bytes!("data/av1_10.obu");
    let frames = decode_all(
        Dav1dDecoder::new(),
        &config(VideoCodec::AV1, 64, 48),
        &once(bytes),
    );
    assert_frames(&frames, VideoPixelFormat::YUV420P10, 64, 48);
}

#[cfg(feature = "vp8")]
#[test]
fn decodes_real_vp8() {
    let bytes = include_bytes!("data/vp8_8.ivf");
    let packets = ivf_frames(bytes);
    assert!(!packets.is_empty());
    let frames = decode_all(
        Vpx8Decoder::new(),
        &config(VideoCodec::VP8, 64, 48),
        &packets,
    );
    assert_frames(&frames, VideoPixelFormat::I420, 64, 48);
}

#[cfg(feature = "vp9")]
#[test]
fn decodes_real_vp9_8bit() {
    let bytes = include_bytes!("data/vp9_8.ivf");
    let packets = ivf_frames(bytes);
    assert!(!packets.is_empty());
    let frames = decode_all(
        Vpx9Decoder::new(),
        &config(VideoCodec::VP9, 64, 48),
        &packets,
    );
    assert_frames(&frames, VideoPixelFormat::I420, 64, 48);
}

#[cfg(feature = "vp9")]
#[test]
fn decodes_real_vp9_10bit() {
    let bytes = include_bytes!("data/vp9_10.ivf");
    let packets = ivf_frames(bytes);
    assert!(!packets.is_empty());
    let frames = decode_all(
        Vpx9Decoder::new(),
        &config(VideoCodec::VP9, 64, 48),
        &packets,
    );
    assert_frames(&frames, VideoPixelFormat::YUV420P10, 64, 48);
}

#[cfg(feature = "theora")]
#[test]
fn decodes_real_theora() {
    let bytes = include_bytes!("data/theora_8.ogg");
    let packets = ogg_packets(bytes);
    assert!(
        packets.len() >= 4,
        "expected 3 headers + 1 frame, got {}",
        packets.len()
    );
    let refs: Vec<&[u8]> = packets.iter().map(Vec::as_slice).collect();
    let frames = decode_all(
        TheoraDecoder::new(),
        &config(VideoCodec::Theora, 64, 48),
        &refs,
    );
    assert_frames(&frames, VideoPixelFormat::I420, 64, 48);
}

#[cfg(feature = "mjpeg")]
#[test]
fn decodes_real_mjpeg() {
    let bytes = include_bytes!("data/mjpeg_8.mjpg");
    let packets = jpeg_frames(bytes);
    assert!(!packets.is_empty());
    let frames = decode_all(
        MjpegDecoder::new(),
        &config(VideoCodec::Mjpeg, 64, 48),
        &packets,
    );
    assert_frames(&frames, VideoPixelFormat::I420, 64, 48);
}

#[cfg(feature = "vvc")]
#[test]
fn decodes_real_vvc_10bit() {
    let bytes = include_bytes!("data/vvc_10.vvc");
    let frames = decode_all(
        VvdecDecoder::new(),
        &config(VideoCodec::Vvc, 64, 48),
        &once(bytes),
    );
    assert_frames(&frames, VideoPixelFormat::YUV420P10, 64, 48);
}

#[cfg(feature = "evc")]
#[test]
fn decodes_real_evc_10bit() {
    let annexb = evc_annexb(include_bytes!("data/evc_10.evc"));
    let frames = decode_all(
        XevdDecoder::new(),
        &config(VideoCodec::Evc, 128, 64),
        &[&annexb],
    );
    assert_frames(&frames, VideoPixelFormat::YUV420P10, 128, 64);
}

// ---- reset() 回归：每种后端复位后必须还能解出同样的帧 ----

#[cfg(feature = "h264")]
#[test]
fn h264_reset_then_decode_again() {
    assert_reset_roundtrip(
        OpenH264Decoder::new(),
        &config(VideoCodec::H264, 64, 48),
        &once(include_bytes!("data/h264_8.h264")),
        VideoPixelFormat::I420,
        64,
        48,
    );
}

#[cfg(feature = "hevc")]
#[test]
fn hevc_reset_then_decode_again() {
    assert_reset_roundtrip(
        HevcDecoder::new(),
        &config(VideoCodec::HEVC, 64, 48),
        &once(include_bytes!("data/hevc_8.hevc")),
        VideoPixelFormat::I420,
        64,
        48,
    );
}

#[cfg(feature = "av1")]
#[test]
fn av1_reset_then_decode_again() {
    assert_reset_roundtrip(
        Dav1dDecoder::new(),
        &config(VideoCodec::AV1, 64, 48),
        &once(include_bytes!("data/av1_8.obu")),
        VideoPixelFormat::I420,
        64,
        48,
    );
}

#[cfg(feature = "vp8")]
#[test]
fn vp8_reset_then_decode_again() {
    let bytes = include_bytes!("data/vp8_8.ivf");
    assert_reset_roundtrip(
        Vpx8Decoder::new(),
        &config(VideoCodec::VP8, 64, 48),
        &ivf_frames(bytes),
        VideoPixelFormat::I420,
        64,
        48,
    );
}

#[cfg(feature = "vp9")]
#[test]
fn vp9_reset_then_decode_again() {
    let bytes = include_bytes!("data/vp9_8.ivf");
    assert_reset_roundtrip(
        Vpx9Decoder::new(),
        &config(VideoCodec::VP9, 64, 48),
        &ivf_frames(bytes),
        VideoPixelFormat::I420,
        64,
        48,
    );
}

#[cfg(feature = "theora")]
#[test]
fn theora_reset_then_decode_again() {
    let bytes = include_bytes!("data/theora_8.ogg");
    let packets = ogg_packets(bytes);
    let refs: Vec<&[u8]> = packets.iter().map(Vec::as_slice).collect();
    assert_reset_roundtrip(
        TheoraDecoder::new(),
        &config(VideoCodec::Theora, 64, 48),
        &refs,
        VideoPixelFormat::I420,
        64,
        48,
    );
}

#[cfg(feature = "mjpeg")]
#[test]
fn mjpeg_reset_then_decode_again() {
    let bytes = include_bytes!("data/mjpeg_8.mjpg");
    assert_reset_roundtrip(
        MjpegDecoder::new(),
        &config(VideoCodec::Mjpeg, 64, 48),
        &jpeg_frames(bytes),
        VideoPixelFormat::I420,
        64,
        48,
    );
}

#[cfg(feature = "vvc")]
#[test]
fn vvc_reset_then_decode_again() {
    assert_reset_roundtrip(
        VvdecDecoder::new(),
        &config(VideoCodec::Vvc, 64, 48),
        &once(include_bytes!("data/vvc_10.vvc")),
        VideoPixelFormat::YUV420P10,
        64,
        48,
    );
}

#[cfg(feature = "evc")]
#[test]
fn evc_reset_then_decode_again() {
    let annexb = evc_annexb(include_bytes!("data/evc_10.evc"));
    assert_reset_roundtrip(
        XevdDecoder::new(),
        &config(VideoCodec::Evc, 128, 64),
        &[&annexb],
        VideoPixelFormat::YUV420P10,
        128,
        64,
    );
}
