//! `buffer.rs` 的断言：送进解码器的一个码流单元。

use super::super::DecoderBuffer;

#[test]
fn plain_buffer_is_not_a_key_frame() {
    let buffer = DecoderBuffer::new(vec![1, 2, 3], 1000, 33333);
    assert_eq!(buffer.data(), &[1, 2, 3]);
    assert_eq!(buffer.timestamp(), 1000);
    assert_eq!(buffer.duration(), 33333);
    assert!(!buffer.is_key_frame());
    assert!(!buffer.is_end_of_stream());
    assert!(!buffer.discontinuity());
    assert!(!buffer.is_empty());
}

#[test]
fn key_frame_buffer_is_marked() {
    let buffer = DecoderBuffer::key_frame(vec![0x65], 0, 33333);
    assert!(buffer.is_key_frame());
    assert!(!buffer.is_end_of_stream());
    assert_eq!(buffer.timestamp(), 0);
}

#[test]
fn end_of_stream_buffer_has_no_data() {
    let buffer = DecoderBuffer::end_of_stream(9999);
    assert!(buffer.is_end_of_stream());
    assert!(buffer.is_empty());
    assert!(buffer.data().is_empty());
    assert_eq!(buffer.timestamp(), 9999);
    assert_eq!(buffer.duration(), 0);
}

#[test]
fn setters_update_the_fields() {
    let mut buffer = DecoderBuffer::new(Vec::new(), 0, 0);
    buffer.set_timestamp(42);
    buffer.set_duration(43);
    buffer.set_is_key_frame(true);
    buffer.set_discontinuity(true);
    buffer.set_offset(4096);
    assert_eq!(buffer.timestamp(), 42);
    assert_eq!(buffer.duration(), 43);
    assert!(buffer.is_key_frame());
    assert!(buffer.discontinuity());
    assert_eq!(buffer.offset(), 4096);
}

#[test]
fn into_data_moves_the_payload_out() {
    let buffer = DecoderBuffer::new(vec![7, 8], 0, 0);
    assert_eq!(buffer.into_data(), vec![7, 8]);
}

#[test]
fn discontinuity_defaults_to_false() {
    let buffer = DecoderBuffer::new(vec![0], 0, 0);
    assert!(!buffer.discontinuity());
    assert_eq!(buffer.offset(), 0);
}
