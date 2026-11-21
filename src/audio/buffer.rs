//! 送进音频解码器的压缩数据单元（对应 Chromium `DecoderBuffer` 的音频用法）。

/// 一个待解码的压缩音频单元。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioBuffer {
    data: Vec<u8>,
    timestamp: i64,
    duration: i64,
    is_end_of_stream: bool,
}

impl AudioBuffer {
    /// 新建一个普通码流单元。时间戳与时长单位为微秒。
    pub fn new(data: Vec<u8>, timestamp: i64, duration: i64) -> Self {
        Self {
            data,
            timestamp,
            duration,
            is_end_of_stream: false,
        }
    }

    /// 新建一个流结束单元，让解码器吐出剩余帧。
    pub fn end_of_stream(timestamp: i64) -> Self {
        Self {
            data: Vec::new(),
            timestamp,
            duration: 0,
            is_end_of_stream: true,
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
    pub fn timestamp(&self) -> i64 {
        self.timestamp
    }
    pub fn duration(&self) -> i64 {
        self.duration
    }
    pub fn is_end_of_stream(&self) -> bool {
        self.is_end_of_stream
    }
}
