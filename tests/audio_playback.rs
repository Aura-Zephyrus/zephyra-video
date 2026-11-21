#![allow(dead_code)]

//! 真实音频码流回放测试：每个后端解码 ffmpeg 生成的 fixture，验证出帧、
//! 声道/采样率/位深、时基与内容非空。

use zephyra_video::{
    AudioBuffer, AudioCodec, AudioDecoder, AudioDecoderConfig, AudioFrame, AudioSampleFormat,
};

fn config(codec: AudioCodec) -> AudioDecoderConfig {
    AudioDecoderConfig::new(codec, AudioSampleFormat::Unknown, 2, 48000, Vec::new())
}

fn assert_audio_frames(frames: &[AudioFrame], expected_format: AudioSampleFormat) {
    assert!(!frames.is_empty(), "decoder returned no audio frames");
    let mut previous_ts = -1i64;
    for frame in frames {
        assert_eq!(frame.channels(), 2);
        assert_eq!(frame.sample_rate(), 48000);
        assert_eq!(frame.format(), expected_format);
        assert!(frame.frames() > 0);
        assert!(frame.timestamp() >= previous_ts);
        previous_ts = frame.timestamp();
        assert!(!frame.planes()[0].is_empty());
        // 编码器延迟可能让首帧为静音，只要求整体有非静音帧。
        assert!(
            frames
                .iter()
                .any(|f| f.planes().iter().any(|p| p.iter().any(|&b| b != 0))),
            "all decoded audio is silent"
        );
    }
}

/// Ogg: 从 page/lacing 重排 packet（Opus/Vorbis 都封装在 Ogg 里）。
fn ogg_packets(bytes: &[u8]) -> Vec<Vec<u8>> {
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

#[test]
fn pcm_passthrough() {
    use zephyra_video::PcmDecoder;
    let mut d = PcmDecoder::new();
    assert!(d
        .initialize(&AudioDecoderConfig::pcm(2, 48000, AudioSampleFormat::S16))
        .is_ok());
    let bytes = include_bytes!("data/audio/test.pcm");
    let mut frames = Vec::new();
    for (i, chunk) in bytes.chunks(3840).enumerate() {
        frames.extend(
            d.decode(&AudioBuffer::new(chunk.to_vec(), i as i64 * 20000, 20000))
                .unwrap(),
        );
    }
    assert_audio_frames(&frames, AudioSampleFormat::PlanarS16);
}

#[cfg(feature = "mp3")]
#[test]
fn mp3_playback() {
    use zephyra_video::Mp3Decoder;
    let mut d = Mp3Decoder::new();
    assert!(d.initialize(&config(AudioCodec::Mp3)).is_ok());
    let bytes = include_bytes!("data/audio/test.mp3");
    let frames = d
        .decode(&AudioBuffer::new(bytes.to_vec(), 0, 200000))
        .unwrap();
    assert_audio_frames(&frames, AudioSampleFormat::PlanarS16);
}

#[cfg(feature = "opus")]
#[test]
fn opus_playback() {
    use zephyra_video::OpusDecoder;
    let mut d = OpusDecoder::new();
    assert!(d.initialize(&config(AudioCodec::Opus)).is_ok());
    let packets = ogg_packets(include_bytes!("data/audio/test.opus"));
    assert!(packets.len() > 3);
    let mut frames = Vec::new();
    // 前两个 Ogg packet 是 OpusHead / OpusTags。
    for (i, packet) in packets.iter().enumerate().skip(2) {
        frames.extend(
            d.decode(&AudioBuffer::new(packet.clone(), i as i64 * 20000, 20000))
                .unwrap(),
        );
    }
    assert_audio_frames(&frames, AudioSampleFormat::PlanarS16);
}

#[cfg(feature = "vorbis")]
#[test]
fn vorbis_playback() {
    use zephyra_video::VorbisDecoder;
    let mut d = VorbisDecoder::new();
    assert!(d.initialize(&config(AudioCodec::Vorbis)).is_ok());
    let all = ogg_packets(include_bytes!("data/audio/test.ogg"));
    // ffmpeg 会在 Ogg 里先写一条 Skeleton 流；只取 Vorbis 的 identification 起。
    let start = all
        .iter()
        .position(|p| p.starts_with(b"\x01vorbis"))
        .unwrap_or(0);
    let packets = &all[start..];
    assert!(packets.len() > 3);
    let mut frames = Vec::new();
    for (i, packet) in packets.iter().enumerate() {
        frames.extend(
            d.decode(&AudioBuffer::new(packet.clone(), i as i64 * 20000, 20000))
                .unwrap(),
        );
    }
    assert_audio_frames(&frames, AudioSampleFormat::PlanarF32);
}

#[cfg(feature = "flac")]
#[test]
fn flac_playback() {
    use zephyra_video::FlacDecoder;
    let mut d = FlacDecoder::new();
    assert!(d.initialize(&config(AudioCodec::Flac)).is_ok());
    let bytes = include_bytes!("data/audio/test.flac");
    let frames = d
        .decode(&AudioBuffer::new(bytes.to_vec(), 0, 200000))
        .unwrap();
    assert_audio_frames(&frames, AudioSampleFormat::PlanarS32);
}
