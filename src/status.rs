//! 解码状态码。
//!
//! 对应 Chromium `media/base/decoder_status.h` 的 `DecoderStatus::Codes`。
//! 判别值照搬（它们会进 UMA 直方图，改值等于改历史）。

/// 解码器状态码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum DecoderStatusCode {
    // —— 通用 ——
    Ok = 0,
    Failed = 1,
    /// 解码被中止（不一定意味着出错，例如 `Reset()` 触发）。
    Aborted = 2,
    InvalidArgument = 3,
    Interrupted = 4,
    /// 远端连接丢失（Mojo 通道断开）。
    Disconnected = 5,
    OutOfMemory = 6,

    // —— 解码失败的原因 ——
    NotInitialized = 100,
    MissingCdm = 101,
    FailedToGetVideoFrame = 102,
    /// 平台（硬件）解码失败。
    PlatformDecodeFailure = 103,
    MalformedBitstream = 104,
    FailedToGetDecoderBuffer = 107,
    DecoderStreamInErrorState = 108,
    DecoderStreamDemuxerError = 110,
    /// 需要关键帧才能继续。
    KeyFrameRequired = 111,
    MissingTimestamp = 112,

    // —— 初始化失败的原因 ——
    UnsupportedProfile = 200,
    UnsupportedCodec = 201,
    UnsupportedConfig = 202,
    UnsupportedEncryptionMode = 203,
    CantChangeCodec = 204,
    FailedToCreateDecoder = 205,
    /// 候选解码器全部试完仍未成功。
    TooManyDecoders = 206,
    MediaFoundationNotAvailable = 207,

    // —— 成功，但需要下游配合 ——
    /// 因配置变更而省略了流结束（不必重新初始化解码器）。
    ElidedEndOfStreamForConfigChange = 300,
}

/// 解码状态：码 + 可选的说明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecoderStatus {
    code: DecoderStatusCode,
    message: String,
}

impl DecoderStatus {
    pub fn new(code: DecoderStatusCode) -> Self {
        Self {
            code,
            message: String::new(),
        }
    }

    /// 带说明的状态。说明只用于日志 —— **不参与**任何判定。
    pub fn with_message(code: DecoderStatusCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> DecoderStatusCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn is_ok(&self) -> bool {
        self.code == DecoderStatusCode::Ok
    }

    pub fn is_error(&self) -> bool {
        !self.is_ok()
    }

    /// 该状态是否表示「解码器需要 `Reset()` 才能继续」。
    ///
    /// 对应 Chromium 的约定：除 `kOk` / `kAborted` / `kPending` 一类之外，
    /// 任何解码错误都要求先复位。
    pub fn requires_reset(&self) -> bool {
        matches!(
            self.code,
            DecoderStatusCode::Failed
                | DecoderStatusCode::PlatformDecodeFailure
                | DecoderStatusCode::MalformedBitstream
                | DecoderStatusCode::DecoderStreamInErrorState
                | DecoderStatusCode::FailedToGetDecoderBuffer
        )
    }
}

impl Default for DecoderStatus {
    fn default() -> Self {
        Self::new(DecoderStatusCode::Ok)
    }
}

impl std::fmt::Display for DecoderStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.code)?;
        if !self.message.is_empty() {
            write!(f, " ({})", self.message)?;
        }
        Ok(())
    }
}

/// 便捷构造：成功。
pub fn ok() -> DecoderStatus {
    DecoderStatus::new(DecoderStatusCode::Ok)
}

#[cfg(test)]
#[path = "../tests/unit/status_tests.rs"]
mod tests;
