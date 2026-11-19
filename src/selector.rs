//! 解码器选择：同一路流，该用硬件解码还是软件解码。
//!
//! 对应 Chromium `media/filters/decoder_selector.cc`。
//!
//! # 规则
//!
//! Chromium 只做**一个**检查 —— 可见区域高度是否达到 360：
//!
//! ```text
//! at_or_above_cutoff = visible_rect.height() >= 360
//! 优先级正常 ⟺ at_or_above_cutoff == decoder.IsPlatformDecoder()
//! ```
//!
//! 也就是：**够大就用平台（硬件）解码器，不够大就用软件解码器**。
//! 被「去优先级」的候选者排到列表**末尾**（不是丢弃）—— 硬件初始化失败时
//! 仍然能退回到软件解码。
//!
//! 为什么是 360 而不是别的数：Chromium 的注释写的是「只做高度检查，
//! 宁可偏向平台解码器」。分辨率太小时硬件解码的额外开销（显存往返、
//! 驱动同步）反而更贵，而且小画面本来用户也看不出差别。

use crate::config::VideoDecoderConfig;
use crate::decoder::VideoDecoder;
use crate::status::{DecoderStatus, DecoderStatusCode};

/// 软件解码器的**高度**分界（可见区域高度）。
///
/// 来自 Chromium `decoder_selector.cc` 的 `kSoftwareDecoderHeightCutoff = 360`。
pub const SOFTWARE_DECODER_HEIGHT_CUTOFF: u32 = 360;

/// 候选解码器的优先级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderPriority {
    /// 正常优先级。
    Normal,
    /// 去优先级 —— 排到列表末尾，但**不丢弃**。
    Deprioritized,
    /// 跳过 —— 完全不用。
    Skipped,
}

/// 按 Chromium 的规则给出候选解码器的优先级。
///
/// 对应 Chromium `SelectDecoderPriority()`。
pub fn select_decoder_priority(
    config: &VideoDecoderConfig,
    is_platform_decoder: bool,
) -> DecoderPriority {
    // 只做高度检查：宁可偏向平台解码器。
    let at_or_above_cutoff = config.visible_rect().height() >= SOFTWARE_DECODER_HEIGHT_CUTOFF;
    if at_or_above_cutoff == is_platform_decoder {
        DecoderPriority::Normal
    } else {
        DecoderPriority::Deprioritized
    }
}

/// 解码器选择器。
///
/// 与 Chromium 的 `DecoderSelector` 同形：持有一串候选解码器，按优先级重排，
/// 然后**依次**尝试初始化 —— 第一个成功的胜出，全失败则报错。
pub struct DecoderSelector {
    /// 候选者。`None` 表示已被取走（已选中或已试过）。
    candidates: Vec<Option<Box<dyn VideoDecoder>>>,
    /// 试过但初始化失败的记录（按尝试顺序）。
    failures: Vec<DecoderStatus>,
}

impl DecoderSelector {
    /// 新建一个选择器。候选者的顺序**就是**优先级顺序（Chromium 里是
    /// `DefaultDecoderFactory::CreateVideoDecoders()` 给出的顺序）。
    pub fn new(decoders: Vec<Box<dyn VideoDecoder>>) -> Self {
        Self {
            candidates: decoders.into_iter().map(Some).collect(),
            failures: Vec::new(),
        }
    }

    /// 候选者数量。
    pub fn len(&self) -> usize {
        self.candidates.len()
    }

    /// 是否没有候选者了。
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// 初始化失败的记录（按尝试顺序）。
    pub fn failures(&self) -> &[DecoderStatus] {
        &self.failures
    }

    /// 按优先级重排：被去优先级的排到末尾。
    ///
    /// 对应 Chromium `FilterAndSortAvailableDecoders()`。Chromium 那一版还
    /// 会跳过「不支持解密」的候选者 —— 本 crate 不做加密，所以没有这一条。
    pub fn filter_and_sort(&mut self, config: &VideoDecoderConfig) {
        let mut normal = Vec::new();
        let mut deprioritized = Vec::new();
        for candidate in self.candidates.drain(..) {
            let Some(candidate) = candidate else { continue };
            match select_decoder_priority(config, candidate.is_platform_decoder()) {
                DecoderPriority::Skipped => continue,
                DecoderPriority::Normal => normal.push(Some(candidate)),
                DecoderPriority::Deprioritized => deprioritized.push(Some(candidate)),
            }
        }
        normal.extend(deprioritized);
        self.candidates = normal;
    }

    /// 依次尝试初始化候选者，返回第一个成功的解码器。
    ///
    /// 全失败时返回**最后一个**失败原因（Chromium 的 `ReturnSelectionError()`
    /// 在有 `reinit_cause` 时优先用它，否则是 `kUnsupportedConfig` / `kTooManyDecoders`）。
    pub fn select(
        &mut self,
        config: &VideoDecoderConfig,
        low_delay: bool,
    ) -> Result<Box<dyn VideoDecoder>, DecoderStatus> {
        self.failures.clear();
        while let Some(slot) = self.candidates.iter_mut().find(|slot| slot.is_some()) {
            let mut candidate = slot.take().expect("checked is_some");
            let status = candidate.initialize(config, low_delay);
            if status.is_ok() {
                return Ok(candidate);
            }
            self.failures.push(status);
        }
        Err(self.selection_error())
    }

    /// 全部候选者都失败时的错误。
    fn selection_error(&self) -> DecoderStatus {
        match self.failures.last() {
            Some(status) => status.clone(),
            None => DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedConfig,
                "no decoder candidates",
            ),
        }
    }
}

/// 供 [`VideoDecoder`] 实现者回答「我是不是平台（硬件）解码器」。
///
/// Chrome 的同名方法是 `Decoder::IsPlatformDecoder()`
/// （`media/base/decoder.h`，`VideoDecoder` 从它继承）。
pub trait IsPlatformDecoder {
    fn is_platform_decoder(&self) -> bool;
}

#[cfg(test)]
#[path = "../tests/unit/selector_tests.rs"]
mod tests;
