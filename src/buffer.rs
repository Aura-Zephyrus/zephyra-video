//! 送进解码器的一帧码流。
//!
//! 对应 Chromium `media/base/decoder_buffer.h`。时间戳用**微秒**（`i64`），
//! 与 `base::TimeDelta` 的内部单位一致；用 `i64` 而不是 `f64` 是因为时间戳
//! 要能精确比较与去重，而浮点相等是这类代码最常见的坑。

/// 一个待解码的码流单元（通常是**一个访问单元**）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecoderBuffer {
    data: Vec<u8>,
    timestamp: i64,
    duration: i64,
    is_key_frame: bool,
    is_end_of_stream: bool,
    discontinuity: bool,
    /// 该单元在原始码流中的字节偏移（用于日志与回溯）。
    offset: u64,
}

impl DecoderBuffer {
    /// 新建一个普通码流单元。
    pub fn new(data: Vec<u8>, timestamp: i64, duration: i64) -> Self {
        Self {
            data,
            timestamp,
            duration,
            is_key_frame: false,
            is_end_of_stream: false,
            discontinuity: false,
            offset: 0,
        }
    }

    /// 新建一个**关键帧**单元（I 帧 / IDR）。
    pub fn key_frame(data: Vec<u8>, timestamp: i64, duration: i64) -> Self {
        Self {
            data,
            timestamp,
            duration,
            is_key_frame: true,
            is_end_of_stream: false,
            discontinuity: false,
            offset: 0,
        }
    }

    /// 新建一个**流结束**单元。
    ///
    /// 它没有数据，作用是让解码器把队列里剩下的帧全部吐出来（flush）。
    /// 对应 Chromium `DecoderBuffer::is_end_of_stream()`。
    pub fn end_of_stream(timestamp: i64) -> Self {
        Self {
            data: Vec::new(),
            timestamp,
            duration: 0,
            is_key_frame: false,
            is_end_of_stream: true,
            discontinuity: false,
            offset: 0,
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// 呈现时间戳（微秒）。
    pub fn timestamp(&self) -> i64 {
        self.timestamp
    }

    pub fn set_timestamp(&mut self, timestamp: i64) {
        self.timestamp = timestamp;
    }

    /// 持续时间（微秒）。
    pub fn duration(&self) -> i64 {
        self.duration
    }

    pub fn set_duration(&mut self, duration: i64) {
        self.duration = duration;
    }

    /// 是否是关键帧。关键帧可以**独立**解码，是 seek 与错误恢复的锚点。
    pub fn is_key_frame(&self) -> bool {
        self.is_key_frame
    }

    pub fn set_is_key_frame(&mut self, is_key_frame: bool) {
        self.is_key_frame = is_key_frame;
    }

    /// 是否是流结束单元。
    pub fn is_end_of_stream(&self) -> bool {
        self.is_end_of_stream
    }

    /// 是否存在**不连续**（丢帧、时间戳跳变）。
    ///
    /// 解码器据此决定是否要重新同步。对应 Chromium 的 `discontinuity`。
    pub fn discontinuity(&self) -> bool {
        self.discontinuity
    }

    pub fn set_discontinuity(&mut self, discontinuity: bool) {
        self.discontinuity = discontinuity;
    }

    /// 在原始码流中的字节偏移。
    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn set_offset(&mut self, offset: u64) {
        self.offset = offset;
    }

    /// 单元是否为空（没有数据且不是流结束）。
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

#[cfg(test)]
#[path = "../tests/unit/buffer_tests.rs"]
mod tests;
