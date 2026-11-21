//! MP3 decoding via vendored minimp3 (CC0 public domain).

use std::cell::RefCell;
use std::ffi::{c_int, c_void};

use crate::audio::buffer::AudioBuffer;
use crate::audio::codec::{AudioDecoderType, AudioSampleFormat};
use crate::audio::config::AudioDecoderConfig;
use crate::audio::decoder::AudioDecoder;
use crate::audio::frame::AudioFrame;
use crate::status::{DecoderStatus, DecoderStatusCode};

crate::codec_api!("z_mp3";
    fn z_mp3_open(out: *mut *mut c_void) -> c_int;
    fn z_mp3_close(handle: *mut c_void);
    fn z_mp3_decode(
        handle: *mut c_void,
        data: *const u8,
        len: c_int,
        callback: extern "C" fn(c_int, c_int, c_int, *const i16),
    ) -> c_int;

);

thread_local! {
    static OUTPUT: RefCell<Vec<AudioFrame>> = const { RefCell::new(Vec::new()) };
    static CURRENT_TS: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
    static CURRENT_DUR: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
}

extern "C" fn on_frame(channels: c_int, sample_rate: c_int, frames: c_int, pcm: *const i16) {
    if channels <= 0 || sample_rate <= 0 || frames <= 0 || pcm.is_null() {
        return;
    }
    let channels = channels as usize;
    let frames = frames as usize;
    let samples = frames * channels;
    // SAFETY: shim allocates a stack buffer of MINIMP3_MAX_SAMPLES_PER_FRAME.
    let interleaved = unsafe { std::slice::from_raw_parts(pcm, samples) };
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

/// MP3 软件解码器（输出 16-bit planar S16）。
pub struct Mp3Decoder {
    handle: *mut c_void,
    initialized: bool,
}

impl Default for Mp3Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Mp3Decoder {
    pub fn new() -> Self {
        Self {
            handle: std::ptr::null_mut(),
            initialized: false,
        }
    }
}

impl AudioDecoder for Mp3Decoder {
    fn initialize(&mut self, config: &AudioDecoderConfig) -> DecoderStatus {
        if config.codec() != crate::AudioCodec::Mp3 {
            return DecoderStatus::with_message(
                DecoderStatusCode::UnsupportedCodec,
                "minimp3 only decodes MP3",
            );
        }
        if !config.is_valid_config() {
            return DecoderStatus::new(DecoderStatusCode::UnsupportedConfig);
        }
        if let Some(status) = crate::check_lib!("z_mp3") {
            return status;
        }
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: handle is a valid out-param.
        let ret = unsafe { z_mp3_open(&mut handle) };
        if ret < 0 || handle.is_null() {
            return DecoderStatus::with_message(
                DecoderStatusCode::FailedToCreateDecoder,
                format!("mp3dec init failed ({ret})"),
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
            // SAFETY: data stays alive for this call; the shim copies samples.
            let ret = unsafe {
                z_mp3_decode(
                    self.handle,
                    buffer.data().as_ptr(),
                    buffer.data().len() as c_int,
                    on_frame,
                )
            };
            if ret < 0 {
                return Err(DecoderStatus::with_message(
                    DecoderStatusCode::MalformedBitstream,
                    format!("mp3dec_decode_frame failed ({ret})"),
                ));
            }
        }
        Ok(OUTPUT.with(|sink| std::mem::take(&mut *sink.borrow_mut())))
    }

    fn reset(&mut self) {
        OUTPUT.with(|sink| sink.borrow_mut().clear());
        if self.initialized {
            unsafe { z_mp3_close(self.handle) };
            self.handle = std::ptr::null_mut();
            let mut fresh = std::ptr::null_mut();
            if unsafe { z_mp3_open(&mut fresh) } == 0 && !fresh.is_null() {
                self.handle = fresh;
            } else {
                self.initialized = false;
            }
        }
    }

    fn decoder_type(&self) -> AudioDecoderType {
        AudioDecoderType::Minimimp3
    }
}

impl Drop for Mp3Decoder {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { z_mp3_close(self.handle) };
            self.initialized = false;
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/audio_mp3_tests.rs"]
mod tests;
