//! H.264 位流写入器。
//!
//! 与 [`super::bit_reader::H264BitReader`] 成对：读的那一半移植自 Chromium
//! `media/parsers/h264_bit_reader.{h,cc}`，写的这一半是它的镜像。
//!
//! # 它存在的理由
//!
//! 两个：
//!
//! 1. **测试夹具不依赖外部文件**。合成一个 SPS / PPS / 切片头只需要十几行
//!    位级代码；把它写成字节数组（十六进制常量）会让「这个字段的值到底是
//!    多少」这件事看不出来，改一个值还要重新数比特。
//! 2. **打包**。`ue(v)` / `se(v)` 的编码是机械活，写一次比每次手算可靠。
//!
//! ⚠️ 本写入器**不做**防竞争字节插入（emulation prevention）。那是**码流
//! 封装**的一步（Annex-B / AVCC 的 NAL 负载必须是 RBSP 之外的 EBSP），
//! 混进「写语法元素」这一层会让「我写的这些位」与「字节流里看到的位」
//! 不再一一对应。需要 EBSP 时用 [`super::converter`] 那侧的 NAL 打包。

/// H.264 位流写入器。
#[derive(Debug, Clone, Default)]
pub struct H264BitstreamWriter {
    /// 已完成的字节。
    bytes: Vec<u8>,
    /// 当前正在填的字节。
    current_byte: u8,
    /// 当前字节里已经用掉的位数（0..8）。
    num_bits_used: u32,
}

impl H264BitstreamWriter {
    /// 新建一个空写入器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 写一位。
    pub fn write_bit(&mut self, bit: bool) {
        self.current_byte <<= 1;
        if bit {
            self.current_byte |= 1;
        }
        self.num_bits_used += 1;
        if self.num_bits_used == 8 {
            self.bytes.push(self.current_byte);
            self.current_byte = 0;
            self.num_bits_used = 0;
        }
    }

    /// 写 `num_bits` 位（高位在前）。`num_bits` 必须 ≤ 32。
    pub fn write_bits(&mut self, value: u32, num_bits: u32) {
        assert!(num_bits <= 32, "write_bits supports at most 32 bits");
        for i in (0..num_bits).rev() {
            self.write_bit((value >> i) & 1 != 0);
        }
    }

    /// 写一个无符号指数哥伦布码（`ue(v)`）。
    ///
    /// 值 `v` 编成 `v + 1` 的二进制，前面补 `位宽 - 1` 个零 ——
    /// 总位数是 `2 * floor(log2(v + 1)) + 1`。
    ///
    /// ⚠️ 那个前导零是**必须**的：漏掉它，读回来的是另一个值（而且
    /// 位流照样「解析成功」）。
    pub fn write_ue(&mut self, value: u32) {
        let code = value.checked_add(1).expect("ue(v) cannot encode u32::MAX");
        let num_bits = 32 - code.leading_zeros();
        self.write_bits(0, num_bits - 1);
        self.write_bits(code, num_bits);
    }

    /// 写一个有符号指数哥伦布码（`se(v)`）。
    ///
    /// `v > 0 ⇒ ue(2v - 1)`；`v <= 0 ⇒ ue(-2v)`。
    pub fn write_se(&mut self, value: i32) {
        let mapped = if value <= 0 {
            (-2i64 * i64::from(value)) as u32
        } else {
            (2i64 * i64::from(value) - 1) as u32
        };
        self.write_ue(mapped);
    }

    /// 写 RBSP 的结束位（`rbsp_trailing_bits()`）。
    ///
    /// 规格 7.4.1：一个 1 位 + 若干个 0 位补齐到字节边界。
    pub fn write_rbsp_trailing_bits(&mut self) {
        self.write_bit(true);
        while self.num_bits_used != 0 {
            self.write_bit(false);
        }
    }

    /// 当前已经写出的字节数（不含尚未补齐的最后一个字节）。
    pub fn num_bytes_written(&self) -> usize {
        self.bytes.len()
    }

    /// 取出写好的字节流。
    ///
    /// 最后一个字节若还没填满，会用 0 补齐 —— 与规格一致（`rbsp_trailing_bits`
    /// 已经写了那个 1 位，剩下的 0 位由这里补）。
    pub fn finish(mut self) -> Vec<u8> {
        if self.num_bits_used != 0 {
            self.current_byte <<= 8 - self.num_bits_used;
            self.bytes.push(self.current_byte);
            self.current_byte = 0;
            self.num_bits_used = 0;
        }
        self.bytes
    }
}

#[cfg(test)]
#[path = "../../tests/unit/h264_bit_writer_tests.rs"]
mod tests;
