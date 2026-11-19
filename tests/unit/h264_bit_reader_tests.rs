//! `h264/bit_reader.rs` 的断言：位读取器。

use super::super::H264BitReader;

#[test]
fn empty_input_is_rejected() {
    assert!(H264BitReader::new(&[]).is_none());
}

#[test]
fn reads_bits_most_significant_first() {
    let mut br = H264BitReader::new(&[0b1011_0010]).unwrap();
    assert_eq!(br.read_bits(4), Some(0b1011));
    assert_eq!(br.read_bits(4), Some(0b0010));
    assert_eq!(br.read_bits(1), None);
}

#[test]
fn reads_across_byte_boundaries() {
    let mut br = H264BitReader::new(&[0xFF, 0x00]).unwrap();
    // 前 12 位 = 0xFF 的 8 位 + 0x00 的高 4 位 = 0xFF0。
    assert_eq!(br.read_bits(12), Some(0xFF0));
    assert_eq!(br.read_bits(4), Some(0x0));
}

#[test]
fn rejects_out_of_range_bit_counts() {
    let mut br = H264BitReader::new(&[0xFF]).unwrap();
    assert_eq!(br.read_bits(0), None);
    assert_eq!(br.read_bits(32), None);
    assert_eq!(br.read_bits(33), None);
}

#[test]
fn reads_exp_golomb_codes() {
    // ue(v) 的编码：v+1 的二进制 + 前导零。
    // 1 → 0b1, 0 → 0b010, 2 → 0b011, 3 → 0b00100
    // 合起来是 12 位：1010 0110 0100 0000
    let mut br = H264BitReader::new(&[0xA6, 0x40]).unwrap();
    assert_eq!(br.read_ue(), Some(0));
    assert_eq!(br.read_ue(), Some(1));
    assert_eq!(br.read_ue(), Some(2));
    assert_eq!(br.read_ue(), Some(3));
}

#[test]
fn reads_signed_exp_golomb_codes() {
    // se(v)：k 为偶数取 -k/2，为奇数取 k/2 + 1（Chromium 的约定）。
    // 0 → 1, 1 → 010, -1 → 011 ⇒ 1010 0110
    let mut br = H264BitReader::new(&[0xA6]).unwrap();
    assert_eq!(br.read_se(), Some(0));
    assert_eq!(br.read_se(), Some(1));
    assert_eq!(br.read_se(), Some(-1));
}

#[test]
fn skips_emulation_prevention_bytes() {
    // 00 00 03 01 ⇒ 真正的数据是 00 00 01。
    //
    // ⚠️ 防竞争字节是**懒跳过**的（Chromium `H264BitReader::UpdateCurrByte()`）：
    // 只有当读取器**真的需要**那个字节时才会检查并跳过它。所以前两个字节
    // （0x00 0x00）是直接读出来的，0x03 要到第三次取字节时才被丢掉。
    let mut br = H264BitReader::new(&[0x00, 0x00, 0x03, 0x01]).unwrap();
    assert_eq!(br.read_bits(16), Some(0x0000));
    // 这一读才会撞上 0x03 ⇒ 跳过它，读到 0x01。
    assert_eq!(br.read_bits(8), Some(0x01));
    assert_eq!(br.num_emulation_prevention_bytes_read(), 1);
}

#[test]
fn emulation_prevention_detection_needs_two_zero_bytes() {
    // 00 03 01 —— 03 前面只有一个 00，不是防竞争字节。
    let mut br = H264BitReader::new(&[0x00, 0x03, 0x01]).unwrap();
    assert_eq!(br.read_bits(16), Some(0x0003));
    assert_eq!(br.num_emulation_prevention_bytes_read(), 0);
}

#[test]
fn reports_remaining_bits() {
    let mut br = H264BitReader::new(&[0xFF, 0xFF]).unwrap();
    assert_eq!(br.num_bits_left(), 16);
    br.read_bits(3);
    assert_eq!(br.num_bits_left(), 13);
}

#[test]
fn has_more_rbsp_data_sees_the_trailing_zero_padding() {
    // 0x80 = 一个 1 位 + 停止位后全 0 ⇒ 没有更多 RBSP 数据。
    let mut br = H264BitReader::new(&[0x80]).unwrap();
    assert!(!br.has_more_rbsp_data());

    let mut br = H264BitReader::new(&[0xFF]).unwrap();
    assert!(br.has_more_rbsp_data());
}

#[test]
fn has_more_rbsp_data_treats_all_zero_tail_as_empty() {
    let mut br = H264BitReader::new(&[0x00, 0x00]).unwrap();
    assert!(!br.has_more_rbsp_data());
}

#[test]
fn reading_past_the_end_returns_none() {
    let mut br = H264BitReader::new(&[0xFF]).unwrap();
    assert_eq!(br.read_bits(8), Some(0xFF));
    assert_eq!(br.read_bits(1), None);
    assert_eq!(br.read_ue(), None);
}
