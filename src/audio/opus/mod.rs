//! Opus decoding via vendored libopus (BSD-3), fixed-point pure-C build.

use std::cell::Cell;
use std::cell::RefCell;
use std::ffi::{c_int, c_void};

use crate::audio::buffer::AudioBuffer;
use crate::audio::codec::{AudioDecoderType, AudioSampleFormat};
use crate::audio::config::AudioDecoderConfig;
use crate::audio::decoder::AudioDecoder;
use crate::audio::frame::AudioFrame;
use crate::status::{DecoderStatus, DecoderStatusCode};

crate::codec_api!("z_opus";
    fn z_opus_open(out: *mut *mut c_void, sample_rate: c_int, channels: c_int) -> c_int;
    fn z_opus_close(handle: *mut c_void);
    fn z_opus_decode(
        handle: *mut c_void,
        data: *const u8,
        len: c_int,
        callback: extern "C" fn(c_int, *const i16),
    ) -> c_int;

);

thread_local! {
    static OUTPUT: RefCell<Vec<AudioFrame>> = const { RefCell::new(Vec::new()) };
    static CHANNELS: Cell<u16> = const { Cell::new(0) };
    static SAMPLE_RATE: Cell<i32> = const { Cell::new(0) };
    static CURRENT_TS: Cell<i64> = const { Cell::new(0) };
    static CURRENT_DUR: Cell<i64> = const { Cell::new(0) };
}

extern "C" fn on_frame(frames: c_int, pcm: *const i16) {
    let channels = CHANNELS.with(|c| c.get()) as usize;
    let sample_rate = SAMPLE_RATE.with(|c| c.get());
    if channels == 0 || sample_rate <= 0 || frames <= 0 || pcm.is_null() {
        return;
    }
    let frames = frames as usize;
    // SAFETY: shim's stack buffer holds frames*channels S16 samples.
    let interleaved = unsafe { std::slice::from_raw_parts(pcm, frames * channels) };
    let mut planes = vec![Vec::with_capacity(frames * 2); channels];
    for chunk in interleaved.chunks_exact(channels) {
        for (ch, sample) in chunk.iter().enumerate() {
            planes[ch].extend_from_slice(&sample.to_le_bytes());
        }
    }
    if let Some(frame) = AudioFrame::from_planar(
        AudioSampleFormat::PlanarS16,
        channels as u16,
        sample_rate,
        planes,
        frames,
        CURRENT_TS.with(|c| c.get()),
        CURRENT_DUR.with(|c| c.get()),
    ) {
        OUTPUT.with(|sink| sink.borrow_mut().push(frame));
    }
}

/// Opus 软件解码器（输出 16-bit planar S16）。
pub struct OpusDecoder {
    handle: *mut c_void,
    initialized: bool,
    channels: u16,
    sample_rate: i32,
}

impl Default for OpusDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl OpusDecoder {
    pub fn new() -> Self {
        Self {
            handle: std::ptr::null_mut(),
            initialized: false,
            channels: 0,
            sample_rate: 0,
        }
    }
}

impl AudioDecoder for OpusDecoder {
    fn initialize(&mut self, config: &AudioDecoderConfig) -> DecoderStatus {
        if config.codec() != crate::AudioCodec::Opus {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedCodec,
                "libopus only decodes Opus",
            );
        }
        if !config.is_valid_config() {
            return DecoderStatus::new(DecoderStatusCode::UnsupportedConfig);
        }
        if let Some(status) = crate::check_lib!("z_opus") {
            return status;
        }
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: handle is a valid out-param.
        let ret = unsafe {
            z_opus_open(
                &mut handle,
                config.sample_rate(),
                config.channels() as c_int,
            )
        };
        if ret < 0 || handle.is_null() {
            return DecoderStatus::with_message(
                DecoderStatusCode::FailedToCreateDecoder,
                format!("opus_decoder_create failed ({ret})"),
            );
        }
        self.handle = handle;
        self.initialized = true;
        self.channels = config.channels();
        self.sample_rate = config.sample_rate();
        DecoderStatus::default()
    }

    fn decode(&mut self, buffer: &AudioBuffer) -> Result<Vec<AudioFrame>, DecoderStatus> {
        if !self.initialized {
            return Err(DecoderStatus::new(DecoderStatusCode::NotInitialized));
        }
        OUTPUT.with(|sink| sink.borrow_mut().clear());
        if !buffer.is_end_of_stream() && !buffer.data().is_empty() {
            CHANNELS.with(|c| c.set(self.channels));
            SAMPLE_RATE.with(|c| c.set(self.sample_rate));
            CURRENT_TS.with(|c| c.set(buffer.timestamp()));
            CURRENT_DUR.with(|c| c.set(buffer.duration()));
            // SAFETY: data stays alive for this call.
            let ret = unsafe {
                z_opus_decode(
                    self.handle,
                    buffer.data().as_ptr(),
                    buffer.data().len() as c_int,
                    on_frame,
                )
            };
            if ret < 0 {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::MalformedBitstream,
                    format!("opus_decode failed ({ret})"),
                ));
            }
        }
        Ok(OUTPUT.with(|sink| std::mem::take(&mut *sink.borrow_mut())))
    }

    fn reset(&mut self) {
        OUTPUT.with(|sink| sink.borrow_mut().clear());
        if self.initialized {
            unsafe { z_opus_close(self.handle) };
            self.handle = std::ptr::null_mut();
            let mut fresh = std::ptr::null_mut();
            let ret = unsafe { z_opus_open(&mut fresh, self.sample_rate, self.channels as c_int) };
            if ret == 0 && !fresh.is_null() {
                self.handle = fresh;
            } else {
                self.initialized = false;
            }
        }
    }

    fn decoder_type(&self) -> AudioDecoderType {
        AudioDecoderType::LibOpus
    }
}

impl Drop for OpusDecoder {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { z_opus_close(self.handle) };
            self.initialized = false;
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/audio_opus_tests.rs"]
mod tests;
