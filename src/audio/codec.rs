//! 音频编码词汇表：编解码器、采样格式、解码器种类。
//!
//! 判别值与 Chromium `media/base/audio_codecs.h` /
//! `media/base/audio_sample_types.h` 对齐（会进日志与 UMA 直方图）。

/// 音频编解码器。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum AudioCodec {
    Unknown = 0,
    /// MPEG-1/2 Layer III。
    Mp3 = 2,
    /// 未压缩 PCM（S16LE / F32LE）。
    Pcm = 3,
    Vorbis = 4,
    Flac = 5,
    Opus = 12,
}

impl AudioCodec {
    pub const fn as_str(self) -> &'static str {
        match self {
            AudioCodec::Unknown => "unknown",
            AudioCodec::Mp3 => "MP3",
            AudioCodec::Pcm => "PCM",
            AudioCodec::Vorbis => "Vorbis",
            AudioCodec::Flac => "FLAC",
            AudioCodec::Opus => "Opus",
        }
    }
}

/// 采样格式（对应 Chromium `AudioSampleFormat`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum AudioSampleFormat {
    Unknown = 0,
    U8 = 1,
    S16 = 2,
    S32 = 3,
    F32 = 4,
    PlanarU8 = 5,
    PlanarS16 = 6,
    PlanarF32 = 7,
    /// 本 crate 扩展：32-bit 整数 planar（Chromium 的 kSampleFormatPlanarS32）。
    PlanarS32 = 12,
}

impl AudioSampleFormat {
    /// 该格式是否为 planar（每声道独立平面）。
    pub const fn is_planar(self) -> bool {
        matches!(
            self,
            AudioSampleFormat::PlanarU8
                | AudioSampleFormat::PlanarS16
                | AudioSampleFormat::PlanarF32
                | AudioSampleFormat::PlanarS32
        )
    }

    /// 每个采样的字节数。
    pub const fn bytes_per_sample(self) -> u8 {
        match self {
            AudioSampleFormat::U8 | AudioSampleFormat::PlanarU8 => 1,
            AudioSampleFormat::S16 | AudioSampleFormat::PlanarS16 => 2,
            AudioSampleFormat::S32
            | AudioSampleFormat::F32
            | AudioSampleFormat::PlanarF32
            | AudioSampleFormat::PlanarS32 => 4,
            AudioSampleFormat::Unknown => 0,
        }
    }
}

/// 音频解码器种类（用于统计与日志）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioDecoderType {
    /// 纯 Rust PCM 直通。
    Pcm,
    /// vendored minimp3（CC0）。
    Minimimp3,
    /// vendored libopus（BSD-3）。
    LibOpus,
    /// vendored libvorbis（BSD-3）。
    LibVorbis,
    /// vendored libFLAC（BSD-3）。
    LibFlac,
}

impl AudioDecoderType {
    pub const fn as_str(self) -> &'static str {
        match self {
            AudioDecoderType::Pcm => "pcm",
            AudioDecoderType::Minimimp3 => "minimp3",
            AudioDecoderType::LibOpus => "libopus",
            AudioDecoderType::LibVorbis => "libvorbis",
            AudioDecoderType::LibFlac => "libflac",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_values_stay_chromium_aligned() {
        assert_eq!(AudioCodec::Mp3 as i32, 2);
        assert_eq!(AudioCodec::Pcm as i32, 3);
        assert_eq!(AudioCodec::Vorbis as i32, 4);
        assert_eq!(AudioCodec::Flac as i32, 5);
        assert_eq!(AudioCodec::Opus as i32, 12);
    }

    #[test]
    fn sample_format_bytes_and_planarity() {
        assert_eq!(AudioSampleFormat::S16.bytes_per_sample(), 2);
        assert_eq!(AudioSampleFormat::PlanarF32.bytes_per_sample(), 4);
        assert!(AudioSampleFormat::PlanarS16.is_planar());
        assert!(!AudioSampleFormat::S16.is_planar());
    }
}
