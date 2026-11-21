use crate::audio::buffer::AudioBuffer;
use crate::audio::codec::{AudioCodec, AudioDecoderType};
use crate::audio::config::AudioDecoderConfig;
use crate::audio::decoder::AudioDecoder;
use crate::audio::VorbisDecoder;
use crate::status::DecoderStatusCode;

fn config() -> AudioDecoderConfig {
    AudioDecoderConfig::new(
        AudioCodec::Vorbis,
        crate::AudioSampleFormat::Unknown,
        2,
        44100,
        Vec::new(),
    )
}

#[test]
fn create_and_drop_vorbis_decoder() {
    let mut d = VorbisDecoder::new();
    assert!(d.initialize(&config()).is_ok());
    assert_eq!(d.decoder_type(), AudioDecoderType::LibVorbis);
}

#[test]
fn rejects_non_vorbis() {
    let mut d = VorbisDecoder::new();
    assert_eq!(
        d.initialize(&AudioDecoderConfig::new(
            AudioCodec::Opus,
            crate::AudioSampleFormat::Unknown,
            2,
            48000,
            Vec::new()
        ))
        .code(),
        DecoderStatusCode::UnsupportedCodec
    );
}

#[test]
fn garbage_header_returns_error() {
    let mut d = VorbisDecoder::new();
    assert!(d.initialize(&config()).is_ok());
    assert!(d.decode(&AudioBuffer::new(vec![0xFF; 8], 0, 0)).is_err());
}
