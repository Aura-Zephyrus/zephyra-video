//! 解码出来的音频帧：每声道一个平面，对应 Chromium `AudioBuffer`。

use crate::audio::codec::AudioSampleFormat;
use std::sync::Arc;

/// 一帧 PCM 数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFrame {
    format: AudioSampleFormat,
    channels: u16,
    sample_rate: i32,
    /// 每个声道一个平面；交错格式则只有一个平面。
    ///
    /// `Arc<[u8]>` 让音频帧与视频帧共享同一套零拷贝所有权模型：
    /// 解码器把分配交给帧，消费方 clone 时只加引用计数。
    planes: Vec<Arc<[u8]>>,
    /// 本帧包含的采样帧数（每个声道相同）。
    frames: usize,
    timestamp: i64,
    duration: i64,
}

impl AudioFrame {
    /// 从每声道一个平面构造帧。
    pub fn from_planar<T: Into<Arc<[u8]>> + AsRef<[u8]>>(
        format: AudioSampleFormat,
        channels: u16,
        sample_rate: i32,
        planes: Vec<T>,
        frames: usize,
        timestamp: i64,
        duration: i64,
    ) -> Option<Self> {
        if channels == 0 || sample_rate <= 0 || frames == 0 {
            return None;
        }
        let bps = format.bytes_per_sample() as usize;
        if bps == 0 {
            return None;
        }
        let expected_planes = if format.is_planar() {
            channels as usize
        } else {
            1
        };
        if planes.len() != expected_planes {
            return None;
        }
        let needed = frames.checked_mul(bps)?;
        if planes.iter().any(|p| p.as_ref().len() < needed) {
            return None;
        }
        Some(Self {
            format,
            channels,
            sample_rate,
            planes: planes.into_iter().map(Into::into).collect(),
            frames,
            timestamp,
            duration,
        })
    }

    pub fn format(&self) -> AudioSampleFormat {
        self.format
    }
    pub fn channels(&self) -> u16 {
        self.channels
    }
    pub fn sample_rate(&self) -> i32 {
        self.sample_rate
    }
    pub fn planes(&self) -> &[Arc<[u8]>] {
        &self.planes
    }

    /// 第 `channel` 个平面的共享所有权句柄（clone 只加引用计数）。
    pub fn plane_arc(&self, channel: usize) -> Option<&Arc<[u8]>> {
        self.planes.get(channel)
    }
    pub fn frames(&self) -> usize {
        self.frames
    }
    pub fn timestamp(&self) -> i64 {
        self.timestamp
    }
    pub fn duration(&self) -> i64 {
        self.duration
    }

    /// 字节序无关地取第 `channel` 声道第 `index` 个 S16 采样。
    pub fn s16_sample(&self, channel: usize, index: usize) -> Option<i16> {
        if self.format.bytes_per_sample() != 2 || channel >= self.planes.len() {
            return None;
        }
        let bytes = self.planes[channel].get(index * 2..index * 2 + 2)?;
        Some(i16::from_le_bytes([bytes[0], bytes[1]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planar_frame_validation() {
        let planes = vec![vec![0u8; 16], vec![0u8; 16]];
        let f =
            AudioFrame::from_planar(AudioSampleFormat::PlanarS16, 2, 48000, planes, 8, 100, 200)
                .unwrap();
        assert_eq!(f.channels(), 2);
        assert_eq!(f.frames(), 8);
        assert_eq!(f.s16_sample(1, 3), Some(0));
    }

    #[test]
    fn planar_frame_keeps_the_original_arc_allocation() {
        let plane: Arc<[u8]> = vec![3u8; 16].into();
        let source_ptr = plane.as_ptr();
        let frame = AudioFrame::from_planar(
            AudioSampleFormat::PlanarS16,
            2,
            48000,
            vec![plane.clone(), vec![0u8; 16].into()],
            8,
            0,
            0,
        )
        .unwrap();
        let stored = frame.plane_arc(0).expect("first channel");
        assert!(Arc::ptr_eq(&plane, stored), "from_planar must not copy");
        assert_eq!(stored.as_ptr(), source_ptr);
        let cloned = frame.clone();
        assert_eq!(cloned.plane_arc(0).map(|p| p.as_ptr()), Some(source_ptr));
    }

    #[test]
    fn rejects_short_planes() {
        assert!(AudioFrame::from_planar(
            AudioSampleFormat::PlanarS16,
            2,
            48000,
            vec![vec![0u8; 8], vec![0u8; 16]],
            8,
            0,
            0
        )
        .is_none());
    }
}
