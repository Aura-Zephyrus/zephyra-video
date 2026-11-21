//! 音频解码配置：对应 Chromium `media/base/audio_decoder_config.h`。

use crate::audio::codec::{AudioCodec, AudioSampleFormat};

/// 一条音频流的解码配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDecoderConfig {
    codec: AudioCodec,
    sample_format: AudioSampleFormat,
    channels: u16,
    sample_rate: i32,
    extra_data: Vec<u8>,
    encrypted: bool,
}

impl AudioDecoderConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        codec: AudioCodec,
        sample_format: AudioSampleFormat,
        channels: u16,
        sample_rate: i32,
        extra_data: Vec<u8>,
    ) -> Self {
        Self {
            codec,
            sample_format,
            channels,
            sample_rate,
            extra_data,
            encrypted: false,
        }
    }

    /// PCM 流的便捷构造：无额外数据。
    pub fn pcm(channels: u16, sample_rate: i32, format: AudioSampleFormat) -> Self {
        Self::new(AudioCodec::Pcm, format, channels, sample_rate, Vec::new())
    }

    pub fn codec(&self) -> AudioCodec {
        self.codec
    }
    pub fn sample_format(&self) -> AudioSampleFormat {
        self.sample_format
    }
    pub fn channels(&self) -> u16 {
        self.channels
    }
    pub fn sample_rate(&self) -> i32 {
        self.sample_rate
    }
    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }
    pub fn is_encrypted(&self) -> bool {
        self.encrypted
    }
    pub fn set_is_encrypted(&mut self, encrypted: bool) {
        self.encrypted = encrypted;
    }

    /// 配置是否自洽。
    ///
    /// - codec 已知
    /// - 声道数 > 0（除 Unknown 外）
    /// - 采样率 > 0
    /// - PCM 必须给出具体的采样格式；压缩格式允许 Unknown（由解码器决定）
    pub fn is_valid_config(&self) -> bool {
        if self.codec == AudioCodec::Unknown || self.channels == 0 || self.sample_rate <= 0 {
            return false;
        }
        if self.codec == AudioCodec::Pcm {
            return matches!(
                self.sample_format,
                AudioSampleFormat::S16
                    | AudioSampleFormat::F32
                    | AudioSampleFormat::S32
                    | AudioSampleFormat::U8
            );
        }
        true
    }

    pub fn as_human_readable_string(&self) -> String {
        format!(
            "codec={} format={:?} channels={} sample_rate={} extra_data={}B",
            self.codec.as_str(),
            self.sample_format,
            self.channels,
            self.sample_rate,
            self.extra_data.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_config_validation() {
        assert!(AudioDecoderConfig::pcm(2, 48000, AudioSampleFormat::S16).is_valid_config());
        assert!(!AudioDecoderConfig::pcm(0, 48000, AudioSampleFormat::S16).is_valid_config());
        assert!(!AudioDecoderConfig::pcm(2, 0, AudioSampleFormat::S16).is_valid_config());
        assert!(!AudioDecoderConfig::pcm(2, 48000, AudioSampleFormat::Unknown).is_valid_config());
    }

    #[test]
    fn compressed_config_allows_unknown_sample_format() {
        let c = AudioDecoderConfig::new(
            AudioCodec::Opus,
            AudioSampleFormat::Unknown,
            2,
            48000,
            Vec::new(),
        );
        assert!(c.is_valid_config());
    }
}
