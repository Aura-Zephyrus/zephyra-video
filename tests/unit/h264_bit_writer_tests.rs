//! `h264/bit_writer.rs` 的断言：位写入器。

use super::super::H264BitstreamWriter;

#[test]
fn writes_bits_most_significant_first() {
    let mut writer = H264BitstreamWriter::new();
    writer.write_bits(0b1011_0010, 8);
    assert_eq!(writer.finish(), vec![0b1011_0010]);
}

#[test]
fn writes_bits_across_byte_boundaries() {
    let mut writer = H264BitstreamWriter::new();
    writer.write_bits(0xFFF, 12);
    writer.write_bits(0x0, 4);
    // 0xFFF 占满第一个字节 + 第二个字节的高 4 位 ⇒ [0xFF, 0xF0]。
    assert_eq!(writer.finish(), vec![0xFF, 0xF0]);
}

#[test]
fn writes_exp_golomb_codes() {
    let mut writer = H264BitstreamWriter::new();
    writer.write_ue(0);
    writer.write_ue(1);
    writer.write_ue(2);
    writer.write_ue(3);
    // 0 → 1, 1 → 010, 2 → 011, 3 → 00100 ⇒ 1010 0110 0100 0000
    assert_eq!(writer.finish(), vec![0xA6, 0x40]);
}

#[test]
fn writes_signed_exp_golomb_codes() {
    let mut writer = H264BitstreamWriter::new();
    writer.write_se(0);
    writer.write_se(1);
    writer.write_se(-1);
    // 0 → 1, 1 → 010, -1 → 011 ⇒ 1010 0110
    assert_eq!(writer.finish(), vec![0xA6]);
}

#[test]
fn trailing_bits_pad_to_a_byte_boundary() {
    let mut writer = H264BitstreamWriter::new();
    writer.write_bit(true);
    writer.write_rbsp_trailing_bits();
    // 1 位数据 + 停止位 1 + 6 个 0 ⇒ 0b1100_0000
    assert_eq!(writer.finish(), vec![0b1100_0000]);
}

#[test]
fn byte_count_tracks_completed_bytes() {
    let mut writer = H264BitstreamWriter::new();
    writer.write_bits(0xFF, 8);
    assert_eq!(writer.num_bytes_written(), 1);
    writer.write_bit(false);
    assert_eq!(writer.num_bytes_written(), 1);
    writer.write_bits(0xFF, 7);
    assert_eq!(writer.num_bytes_written(), 2);
}

#[test]
fn writer_and_reader_round_trip() {
    let mut writer = H264BitstreamWriter::new();
    writer.write_ue(7);
    writer.write_se(-3);
    writer.write_bits(0xABCD, 16);
    let bytes = writer.finish();

    let mut reader = super::super::H264BitReader::new(&bytes).unwrap();
    assert_eq!(reader.read_ue(), Some(7));
    assert_eq!(reader.read_se(), Some(-3));
    assert_eq!(reader.read_bits(16), Some(0xABCD));
}
