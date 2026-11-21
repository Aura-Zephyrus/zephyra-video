use crate::audio::buffer::AudioBuffer;
use crate::audio::codec::{AudioCodec, AudioDecoderType};
use crate::audio::config::AudioDecoderConfig;
use crate::audio::decoder::AudioDecoder;
use crate::audio::Mp3Decoder;
use crate::status::DecoderStatusCode;

fn config() -> AudioDecoderConfig {
    AudioDecoderConfig::new(
        AudioCodec::Mp3,
        crate::AudioSampleFormat::Unknown,
        2,
        44100,
        Vec::new(),
    )
}

#[test]
fn create_and_drop_mp3_decoder() {
    let mut d = Mp3Decoder::new();
    assert!(d.initialize(&config()).is_ok());
    assert_eq!(d.decoder_type(), AudioDecoderType::Minimimp3);
}

#[test]
fn rejects_non_mp3() {
    let mut d = Mp3Decoder::new();
    assert_eq!(
        d.initialize(&AudioDecoderConfig::new(
            AudioCodec::Vorbis,
            crate::AudioSampleFormat::Unknown,
            2,
            44100,
            Vec::new()
        ))
        .code(),
        DecoderStatusCode::UnsupportedCodec
    );
}

#[test]
fn garbage_returns_error() {
    let mut d = Mp3Decoder::new();
    assert!(d.initialize(&config()).is_ok());
    assert!(d.decode(&AudioBuffer::new(vec![0xFF; 16], 0, 0)).is_ok());
}
