//! 未压缩 PCM 解码器：纯 Rust，无 C 依赖，默认构建即可用。

use crate::audio::buffer::AudioBuffer;
use crate::audio::codec::{AudioDecoderType, AudioSampleFormat};
use crate::audio::config::AudioDecoderConfig;
use crate::audio::decoder::AudioDecoder;
use crate::audio::frame::AudioFrame;
use crate::status::{DecoderStatus, DecoderStatusCode};

/// PCM 直通解码器。按配置把交错字节拆成每声道一个平面。
pub struct PcmDecoder {
    config: Option<AudioDecoderConfig>,
}

impl Default for PcmDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl PcmDecoder {
    pub fn new() -> Self {
        Self { config: None }
    }
}

impl AudioDecoder for PcmDecoder {
    fn initialize(&mut self, config: &AudioDecoderConfig) -> DecoderStatus {
        if !config.is_valid_config() {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedConfig,
                "invalid PCM audio config",
            );
        }
        self.config = Some(config.clone());
        DecoderStatus::default()
    }

    fn decode(&mut self, buffer: &AudioBuffer) -> Result<Vec<AudioFrame>, DecoderStatus> {
        let Some(config) = &self.config else {
            return Err(DecoderStatus::new(DecoderStatusCode::NotInitialized));
        };
        if buffer.is_end_of_stream() || buffer.data().is_empty() {
            return Ok(Vec::new());
        }
        let bps = config.sample_format().bytes_per_sample() as usize;
        let channels = config.channels() as usize;
        if bps == 0 || channels == 0 {
            return Err(DecoderStatus::new(DecoderStatusCode::UnsupportedConfig));
        }
        let frame_bytes = bps
            .checked_mul(channels)
            .ok_or_else(|| DecoderStatus::new(DecoderStatusCode::UnsupportedConfig))?;
        if !buffer.data().len().is_multiple_of(frame_bytes) {
            return Err(DecoderStatus::with_message(
                DecoderStatusCode::MalformedBitstream,
                "PCM data is not a whole number of sample frames",
            ));
        }
        let frames = buffer.data().len() / frame_bytes;

        let planar = match config.sample_format() {
            AudioSampleFormat::S16 => AudioSampleFormat::PlanarS16,
            AudioSampleFormat::F32 => AudioSampleFormat::PlanarF32,
            AudioSampleFormat::U8 => AudioSampleFormat::PlanarU8,
            AudioSampleFormat::S32 => AudioSampleFormat::S32,
            other => {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::UnsupportedConfig,
                    format!("unsupported PCM sample format: {other:?}"),
                ))
            }
        };

        let mut planes = vec![Vec::with_capacity(frames * bps); channels];
        for chunk in buffer.data().chunks_exact(frame_bytes) {
            for (ch, plane) in planes.iter_mut().enumerate() {
                plane.extend_from_slice(&chunk[ch * bps..(ch + 1) * bps]);
            }
        }
        let frame = AudioFrame::from_planar(
            planar,
            config.channels(),
            config.sample_rate(),
            planes,
            frames,
            buffer.timestamp(),
            buffer.duration(),
        )
        .ok_or_else(|| DecoderStatus::new(DecoderStatusCode::Failed))?;
        Ok(vec![frame])
    }

    fn reset(&mut self) {
        // PCM 无内部状态。
    }

    fn decoder_type(&self) -> AudioDecoderType {
        AudioDecoderType::Pcm
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::buffer::AudioBuffer;

    #[test]
    fn deinterleaves_s16() {
        let mut d = PcmDecoder::new();
        assert!(d
            .initialize(&AudioDecoderConfig::pcm(2, 48000, AudioSampleFormat::S16))
            .is_ok());
        // L=0x0102, R=0x0304, L=0x0506, R=0x0708
        let data = vec![0x02, 0x01, 0x04, 0x03, 0x06, 0x05, 0x08, 0x07];
        let frames = d.decode(&AudioBuffer::new(data, 10, 20)).unwrap();
        assert_eq!(frames.len(), 1);
        let f = &frames[0];
        assert_eq!(f.format(), AudioSampleFormat::PlanarS16);
        assert_eq!(f.frames(), 2);
        assert_eq!(f.s16_sample(0, 0), Some(0x0102));
        assert_eq!(f.s16_sample(1, 1), Some(0x0708));
    }
}
