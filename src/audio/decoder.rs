//! 音频解码器接口（对应 Chromium `media/base/audio_decoder.h`）。

use crate::audio::buffer::AudioBuffer;
use crate::audio::config::AudioDecoderConfig;
use crate::audio::frame::AudioFrame;
use crate::audio::AudioDecoderType;
use crate::status::DecoderStatus;

/// 音频解码器。与视频解码器一样采用同步拉取模型。
pub trait AudioDecoder {
    /// 用给定配置初始化解码器。
    fn initialize(&mut self, config: &AudioDecoderConfig) -> DecoderStatus;

    /// 解码一个缓冲，返回这一调用产生的帧。
    fn decode(&mut self, buffer: &AudioBuffer) -> Result<Vec<AudioFrame>, DecoderStatus>;

    /// 复位解码器状态，丢弃内部缓冲的帧。
    fn reset(&mut self);

    /// 解码器种类。
    fn decoder_type(&self) -> AudioDecoderType;
}
