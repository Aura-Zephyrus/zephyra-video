//! 音频解码：与视频侧对称的词汇表 + 各后端解码器。
//!
//! 默认构建只有纯 Rust 的 [`PcmDecoder`]，零依赖、零 unsafe；MP3 / Opus /
//! Vorbis / FLAC 各自独立 feature，按需才编译 vendored C 源码。

pub mod buffer;
pub mod codec;
pub mod config;
pub mod decoder;
pub mod frame;
pub mod pcm;

#[cfg(feature = "flac")]
#[allow(unsafe_code)]
pub mod flac;
#[cfg(feature = "mp3")]
#[allow(unsafe_code)]
pub mod mp3;
#[cfg(feature = "opus")]
#[allow(unsafe_code)]
pub mod opus;
#[cfg(feature = "vorbis")]
#[allow(unsafe_code)]
pub mod vorbis;

pub use buffer::AudioBuffer;
pub use codec::{AudioCodec, AudioDecoderType, AudioSampleFormat};
pub use config::AudioDecoderConfig;
pub use decoder::AudioDecoder;
#[cfg(feature = "flac")]
pub use flac::FlacDecoder;
pub use frame::AudioFrame;
#[cfg(feature = "mp3")]
pub use mp3::Mp3Decoder;
#[cfg(feature = "opus")]
pub use opus::OpusDecoder;
pub use pcm::PcmDecoder;
#[cfg(feature = "vorbis")]
pub use vorbis::VorbisDecoder;
