//! H.264 位读取器。
//!
//! 移植自 Chromium `media/parsers/h264_bit_reader.{h,cc}`。
//!
//! # 它不是「通用位读取器」
//!
//! H.264 的 RBSP 里插了**防竞争字节**（emulation prevention byte）：字节序列
//! `00 00 03` 里的 `03` 是插入的，读的时候必须跳过。这个读取器把这件事
//! 做进内核，所以上层拿到的是**纯 RBSP 位流**。
//!
//! 另外它提供 `more_rbsp_data()` 的等价物（[`H264BitReader::has_more_rbsp_data`]），
//! 用于判断「slice 里还有没有东西」—— 那是解析 slice 头尾所必需的。

/// H.264 的位读取器（自动跳过防竞争字节）。
#[derive(Debug, Clone)]
pub struct H264BitReader<'a> {
    /// 还没读的字节。
    data: &'a [u8],
    /// 当前字节的内容。
    curr_byte: u32,
    /// 当前字节里还没读的位数。
    num_remaining_bits_in_curr_byte: u32,
    /// 前两个字节（用于检测 `00 00 03`）。
    prev_two_bytes: u32,
    /// 已跳过的防竞争字节数。
    emulation_prevention_bytes: u32,
}

impl<'a> H264BitReader<'a> {
    /// 新建一个读取器。空数据视为无效（Chromium 的 `Initialize()` 也返回 false）。
    pub fn new(data: &'a [u8]) -> Option<Self> {
        if data.is_empty() {
            return None;
        }
        Some(Self {
            data,
            curr_byte: 0,
            num_remaining_bits_in_curr_byte: 0,
            // 初始设为 0xffff，这样开头的任意两字节序列都能被接受。
            prev_two_bytes: 0xffff,
            emulation_prevention_bytes: 0,
        })
    }

    /// 取下一个字节；遇到 `00 00 03` 时跳过那个 `03`。
    fn update_curr_byte(&mut self) -> bool {
        if self.data.is_empty() {
            return false;
        }
        // 防竞争三字节检测：遇到 0x000003 就丢掉最后那个 0x03。
        if self.data[0] == 0x03 && (self.prev_two_bytes & 0xffff) == 0 {
            self.data = &self.data[1..];
            self.emulation_prevention_bytes += 1;
            // 需要再攒满三个字节才能重新检测。
            self.prev_two_bytes = 0xffff;
            if self.data.is_empty() {
                return false;
            }
        }
        self.curr_byte = u32::from(self.data[0]);
        self.data = &self.data[1..];
        self.num_remaining_bits_in_curr_byte = 8;
        self.prev_two_bytes = ((self.prev_two_bytes & 0xff) << 8) | self.curr_byte;
        true
    }

    /// 读 `num_bits`（1..=31）位。位流里的第一位是结果里的最高位。
    pub fn read_bits(&mut self, num_bits: usize) -> Option<u32> {
        if num_bits == 0 || num_bits > 31 {
            return None;
        }
        let mut bits_left = num_bits as u32;
        let mut out: u32 = 0;
        while self.num_remaining_bits_in_curr_byte < bits_left {
            // 先把当前字节剩下的位取走，给后面的腾位置。
            let mask = if self.num_remaining_bits_in_curr_byte == 0 {
                0
            } else {
                (1u32 << self.num_remaining_bits_in_curr_byte) - 1
            };
            out |= (self.curr_byte & mask) << (bits_left - self.num_remaining_bits_in_curr_byte);
            bits_left -= self.num_remaining_bits_in_curr_byte;
            if !self.update_curr_byte() {
                return None;
            }
        }
        out |= self.curr_byte >> (self.num_remaining_bits_in_curr_byte - bits_left);
        out &= (1u32 << num_bits) - 1;
        self.num_remaining_bits_in_curr_byte -= bits_left;
        Some(out)
    }

    /// 读一位。
    pub fn read_bit(&mut self) -> Option<u32> {
        self.read_bits(1)
    }

    /// 读一个无符号指数哥伦布码（`ue(v)`）。
    ///
    /// H.264 里的 `ue(v)`：先数前导零的个数 `n`，再读 `n` 位，
    /// 值是 `2^n - 1 + 读到的数`。
    ///
    /// ⚠️ 读完那 `n` 位**之后**才是值的低位 —— 多读一位会让每个
    /// `ue(v)` 的值偏移 `2^n`，而**位流本身仍然解析得「成功」**。
    /// 对应 Chromium `READ_UE_WITH_BITS_READ_OR_RETURN`：
    /// `*out = (1u << n) - 1;` 然后再读 `n` 位加上去。
    pub fn read_ue(&mut self) -> Option<u32> {
        let mut leading_zero_bits = 0u32;
        loop {
            let bit = self.read_bit()?;
            if bit == 1 {
                break;
            }
            leading_zero_bits += 1;
            if leading_zero_bits > 31 {
                return None;
            }
        }
        if leading_zero_bits == 0 {
            return Some(0);
        }
        let rest = self.read_bits(leading_zero_bits as usize)?;
        Some(((1u32 << leading_zero_bits) - 1) + rest)
    }

    /// 读一个有符号指数哥伦布码（`se(v)`）。
    ///
    /// H.264 里的 `se(v)`：`k = ue(v)`，**偶数 `k` 取 `-k/2`、奇数 `k`
    /// 取 `k/2 + 1`**（Chromium `READ_SE_OR_RETURN` 的约定）。
    ///
    /// 于是 `0 → 0`、`1 → 1`、`2 → -1`、`3 → 2`。
    pub fn read_se(&mut self) -> Option<i32> {
        let k = self.read_ue()?;
        if k & 1 == 0 {
            Some(-((k / 2) as i32))
        } else {
            Some((k / 2 + 1) as i32)
        }
    }

    /// 还剩多少位。
    pub fn num_bits_left(&self) -> usize {
        (self.num_remaining_bits_in_curr_byte as usize) + self.data.len() * 8
    }

    /// RBSP 里还有没有数据（不含停止位与零填充）。
    ///
    /// 对应 Chromium 的 `HasMoreRBSPData()`。
    pub fn has_more_rbsp_data(&mut self) -> bool {
        // 先确保当前字节有内容；没有的话就没有更多数据了。
        if self.num_remaining_bits_in_curr_byte == 0 && !self.update_curr_byte() {
            return false;
        }
        // 如果没有更多 RBSP 数据，当前字节里只剩停止位与零填充。
        // （不检查停止位本身，而是把「全是零」这个非法情况同等对待。）
        let mask = if self.num_remaining_bits_in_curr_byte == 0 {
            0
        } else {
            (1u32 << (self.num_remaining_bits_in_curr_byte - 1)) - 1
        };
        if (self.curr_byte & mask) != 0 {
            return true;
        }
        // 有些流的最后一个字节是 0x00（spec 7.4.1 禁止，但现实如此）。
        let all_zeros = self.data.iter().all(|&b| b == 0);
        if all_zeros {
            self.data = &[];
        }
        !all_zeros
    }

    /// 已跳过的防竞争字节数。
    pub fn num_emulation_prevention_bytes_read(&self) -> u32 {
        self.emulation_prevention_bytes
    }
}

#[cfg(test)]
#[path = "../../tests/unit/h264_bit_reader_tests.rs"]
mod tests;
