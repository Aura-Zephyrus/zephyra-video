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

// 音频后端的模块声明与 re-export 同样派生自 codecs.rs 的单一声明表。
zephyra_codecs!(emit_audio_codec_items);

pub use buffer::AudioBuffer;
pub use codec::{AudioCodec, AudioDecoderType, AudioSampleFormat};
pub use config::AudioDecoderConfig;
pub use decoder::AudioDecoder;
pub use frame::AudioFrame;
pub use pcm::PcmDecoder;
