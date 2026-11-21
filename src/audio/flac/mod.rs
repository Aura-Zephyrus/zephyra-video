//! FLAC decoding via vendored libFLAC (BSD-3), decoder only.

use std::cell::{Cell, RefCell};
use std::ffi::{c_int, c_void};

use crate::audio::buffer::AudioBuffer;
use crate::audio::codec::{AudioDecoderType, AudioSampleFormat};
use crate::audio::config::AudioDecoderConfig;
use crate::audio::decoder::AudioDecoder;
use crate::audio::frame::AudioFrame;
use crate::status::{DecoderStatus, DecoderStatusCode};

crate::codec_api!("z_flac";
    fn z_flac_open(out: *mut *mut c_void) -> c_int;
    fn z_flac_close(handle: *mut c_void);
    fn z_flac_decode(
        handle: *mut c_void,
        data: *const u8,
        len: c_int,
        callback: extern "C" fn(c_int, c_int, c_int, c_int, *const *const i32),
    ) -> c_int;

);

thread_local! {
    static OUTPUT: RefCell<Vec<AudioFrame>> = const { RefCell::new(Vec::new()) };
    static CURRENT_TS: Cell<i64> = const { Cell::new(0) };
    static CURRENT_DUR: Cell<i64> = const { Cell::new(0) };
}

extern "C" fn on_frame(
    channels: c_int,
    sample_rate: c_int,
    frames: c_int,
    _bits: c_int,
    planes: *const *const i32,
) {
    if channels <= 0 || sample_rate <= 0 || frames <= 0 || planes.is_null() {
        return;
    }
    let channels = channels as usize;
    let frames = frames as usize;
    // SAFETY: shim passes `channels` plane pointers valid for this callback.
    let raw = unsafe { std::slice::from_raw_parts(planes, channels) };
    let mut out = Vec::with_capacity(channels);
    for &ptr in raw {
        if ptr.is_null() {
            return;
        }
        let samples = unsafe { std::slice::from_raw_parts(ptr, frames) };
        let mut bytes = Vec::with_capacity(frames * 4);
        for s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        out.push(bytes);
    }
    if let Some(frame) = AudioFrame::from_planar(
        AudioSampleFormat::PlanarS32,
        channels as u16,
        sample_rate,
        out,
        frames,
        CURRENT_TS.with(|c| c.get()),
        CURRENT_DUR.with(|c| c.get()),
    ) {
        OUTPUT.with(|sink| sink.borrow_mut().push(frame));
    }
}

/// FLAC 软件解码器（输出 32-bit 整数 planar）。
pub struct FlacDecoder {
    handle: *mut c_void,
    initialized: bool,
}

impl Default for FlacDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FlacDecoder {
    pub fn new() -> Self {
        Self {
            handle: std::ptr::null_mut(),
            initialized: false,
        }
    }
}

impl AudioDecoder for FlacDecoder {
    fn initialize(&mut self, config: &AudioDecoderConfig) -> DecoderStatus {
        if config.codec() != crate::AudioCodec::Flac {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedCodec,
                "libFLAC only decodes FLAC",
            );
        }
        if !config.is_valid_config() {
            return DecoderStatus::new(DecoderStatusCode::UnsupportedConfig);
        }
        if let Some(status) = crate::check_lib!("z_flac") {
            return status;
        }
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: handle is a valid out-param.
        let ret = unsafe { z_flac_open(&mut handle) };
        if ret < 0 || handle.is_null() {
            return DecoderStatus::with_message(
                DecoderStatusCode::FailedToCreateDecoder,
                format!("FLAC__stream_decoder_new failed ({ret})"),
            );
        }
        self.handle = handle;
        self.initialized = true;
        DecoderStatus::default()
    }

    fn decode(&mut self, buffer: &AudioBuffer) -> Result<Vec<AudioFrame>, DecoderStatus> {
        if !self.initialized {
            return Err(DecoderStatus::new(DecoderStatusCode::NotInitialized));
        }
        OUTPUT.with(|sink| sink.borrow_mut().clear());
        if !buffer.is_end_of_stream() && !buffer.data().is_empty() {
            CURRENT_TS.with(|c| c.set(buffer.timestamp()));
            CURRENT_DUR.with(|c| c.set(buffer.duration()));
            // SAFETY: data stays alive for this call; libFLAC reads via callback.
            let ret = unsafe {
                z_flac_decode(
                    self.handle,
                    buffer.data().as_ptr(),
                    buffer.data().len() as c_int,
                    on_frame,
                )
            };
            if ret < 0 {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::MalformedBitstream,
                    format!("FLAC decode failed ({ret})"),
                ));
            }
        }
        Ok(OUTPUT.with(|sink| std::mem::take(&mut *sink.borrow_mut())))
    }

    fn reset(&mut self) {
        OUTPUT.with(|sink| sink.borrow_mut().clear());
        if self.initialized {
            unsafe { z_flac_close(self.handle) };
            self.handle = std::ptr::null_mut();
            let mut fresh = std::ptr::null_mut();
            if unsafe { z_flac_open(&mut fresh) } == 0 && !fresh.is_null() {
                self.handle = fresh;
            } else {
                self.initialized = false;
            }
        }
    }

    fn decoder_type(&self) -> AudioDecoderType {
        AudioDecoderType::LibFlac
    }
}

impl Drop for FlacDecoder {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { z_flac_close(self.handle) };
            self.initialized = false;
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/audio_flac_tests.rs"]
mod tests;
