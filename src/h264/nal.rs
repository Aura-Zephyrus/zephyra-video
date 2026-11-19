//! H.264 的 NAL 单元：类型、Annex-B 切分、AVCC 配置记录。
//!
//! 对应 Chromium `media/parsers/h264_parser.h` 的 `H264NALU` 与
//! `media/formats/mp4/box_definitions.h` 的 `AVCDecoderConfigurationRecord`。

/// NAL 单元类型。判别值与 Chromium `H264NALU::Type` 对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum NalUnitType {
    Unspecified = 0,
    /// 非 IDR 切片。
    NonIdrSlice = 1,
    /// 切片数据 A（2-4 属于同一类）。
    SliceDataA = 2,
    SliceDataB = 3,
    SliceDataC = 4,
    /// IDR 切片。
    IdrSlice = 5,
    /// 补充增强信息（SEI）。
    SeiMessage = 6,
    /// 序列参数集。
    Sps = 7,
    /// 图像参数集。
    Pps = 8,
    /// 访问单元定界符。
    Aud = 9,
    /// 序列结束。
    EoSequence = 10,
    /// 流结束。
    EoStream = 11,
    /// 填充数据。
    Filler = 12,
    /// SPS 扩展。
    SpsExt = 13,
    /// 前缀 NAL。
    Prefix = 14,
    /// 子集 SPS。
    SubsetSps = 15,
    /// 深度参数集。
    Dps = 16,
    /// 切片辅助编码信息。
    CodedSliceAux = 19,
    /// 切片扩展。
    CodedSliceExtension = 20,
    /// 未定义（24..31）。
    Unspecified24 = 24,
    Unspecified25 = 25,
    Unspecified26 = 26,
    Unspecified27 = 27,
    Unspecified28 = 28,
    Unspecified29 = 29,
    Unspecified30 = 30,
    Unspecified31 = 31,
}

impl NalUnitType {
    /// 从 NAL 头字节的低 5 位解析类型。
    pub fn from_byte(byte: u8) -> Self {
        match byte & 0x1f {
            0 => NalUnitType::Unspecified,
            1 => NalUnitType::NonIdrSlice,
            2 => NalUnitType::SliceDataA,
            3 => NalUnitType::SliceDataB,
            4 => NalUnitType::SliceDataC,
            5 => NalUnitType::IdrSlice,
            6 => NalUnitType::SeiMessage,
            7 => NalUnitType::Sps,
            8 => NalUnitType::Pps,
            9 => NalUnitType::Aud,
            10 => NalUnitType::EoSequence,
            11 => NalUnitType::EoStream,
            12 => NalUnitType::Filler,
            13 => NalUnitType::SpsExt,
            14 => NalUnitType::Prefix,
            15 => NalUnitType::SubsetSps,
            16 => NalUnitType::Dps,
            19 => NalUnitType::CodedSliceAux,
            20 => NalUnitType::CodedSliceExtension,
            other => match other {
                24 => NalUnitType::Unspecified24,
                25 => NalUnitType::Unspecified25,
                26 => NalUnitType::Unspecified26,
                27 => NalUnitType::Unspecified27,
                28 => NalUnitType::Unspecified28,
                29 => NalUnitType::Unspecified29,
                30 => NalUnitType::Unspecified30,
                _ => NalUnitType::Unspecified31,
            },
        }
    }

    /// 是否是 **VCL**（有图像数据的切片）。
    pub const fn is_vcl(self) -> bool {
        matches!(
            self,
            NalUnitType::NonIdrSlice
                | NalUnitType::SliceDataA
                | NalUnitType::SliceDataB
                | NalUnitType::SliceDataC
                | NalUnitType::IdrSlice
        )
    }

    /// 是否是参数集（SPS / PPS）。
    pub const fn is_parameter_set(self) -> bool {
        matches!(self, NalUnitType::Sps | NalUnitType::Pps)
    }
}

/// 一个 NAL 单元。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H264Nalu {
    /// `nal_ref_idc`（NAL 头的第 5..6 位）。
    pub nal_ref_idc: u8,
    pub nal_unit_type: NalUnitType,
    /// NAL 单元的负载（**不含**起始码，也不含 NAL 头字节）。
    pub data: Vec<u8>,
}

impl H264Nalu {
    /// NAL 头字节。
    pub fn header_byte(&self) -> u8 {
        ((self.nal_ref_idc & 0x03) << 5) | (self.nal_unit_type as u8 & 0x1f)
    }
}

/// 在 `data` 里找下一个起始码。
///
/// 返回 `Some((offset, start_code_size))`；`start_code_size` 是 3 或 4。
/// 找不到时返回 `None`。
///
/// 对应 Chromium `H264Parser::FindStartCode()`：先找 `0x01`，再回看两个字节
/// 是不是 `00 00`；若起始码**前面还有一个 0x00**，那就是四字节起始码。
pub fn find_start_code(data: &[u8]) -> Option<(usize, usize)> {
    // 逐位找 "\0\0\1"。写成「找 0x01 再回看两个字节」的形式在边界上
    // 容易写错（回看的位置与返回值不是同一个偏移），而这里必须保证
    // 返回的 `offset` 正好指向起始码的**第一个**字节 —— `parse_nalus`
    // 靠它切负载。
    let mut i = 0usize;
    while i + 2 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            // 前面还有一个 0x00 ⇒ 四字节起始码。
            if i > 0 && data[i - 1] == 0 {
                return Some((i - 1, 4));
            }
            return Some((i, 3));
        }
        i += 1;
    }
    None
}

/// 把 Annex-B 码流切成 NAL 单元列表。
///
/// 对应 Chromium `H264Parser::ParseNALUs()`。起始码与前导零字节都被丢掉，
/// NAL 头字节保留在 `data` 的**第一个**字节里（与 Chromium 一致：
/// 它的 `nalu.data` 指向 NAL 头之后，但 `nal_unit_type` 是从头字节解析的 ——
/// 这里为了少一次间接，保留整段负载，头字节在 `data[0]`）。
pub fn parse_nalus(stream: &[u8]) -> Vec<H264Nalu> {
    let mut nalus = Vec::new();
    let mut rest = stream;
    while let Some((offset, start_code_size)) = find_start_code(rest) {
        let after_start_code = &rest[offset + start_code_size..];
        // 元组顺序是 (负载, 下一个待解析区段) —— 两个分支必须一致，
        // 写反了不会编译报错，只会把 NAL 的头字节换成下一个起始码的 0x00。
        let (nal_payload, next) = match find_start_code(after_start_code) {
            Some((next_offset, _)) => {
                let payload = &after_start_code[..next_offset];
                // `next` 仍以起始码开头 —— 下一轮再由 `find_start_code` 定位。
                (payload, &after_start_code[next_offset..])
            }
            None => {
                let payload = after_start_code;
                (payload, &after_start_code[after_start_code.len()..])
            }
        };
        if !nal_payload.is_empty() {
            let header = nal_payload[0];
            nalus.push(H264Nalu {
                nal_ref_idc: (header >> 5) & 0x03,
                nal_unit_type: NalUnitType::from_byte(header),
                data: nal_payload.to_vec(),
            });
        }
        rest = next;
    }
    nalus
}

/// 把 NAL 单元列表按**访问单元**分组。
///
/// 规则（与 FFmpeg 的 `av_parser` 一致）：
///
/// - 非 VCL 的 NAL（SPS / PPS / SEI / AUD …）先攒着；
/// - 遇到 VCL NAL 时，如果当前单元里已经有 VCL NAL，就把当前单元收尾、
///   开一个新的；否则把攒着的非 VCL NAL 并进当前单元。
///
/// 参数集因此落在**它后面那个**访问单元里 —— 这正是解码器需要的语义。
///
/// # 限制：这是 **NAL 级**的启发式
///
/// 规格允许一个访问单元里有多个切片（第二个切片的
/// `first_mb_in_slice` 不为 0）。NAL 头里**没有**这个信息，所以本函数
/// 分不出「同一图像的第二个切片」与「下一张图像的第一个切片」——
/// 它把每个 VCL NAL 都当成一个新单元的起点。
///
/// 需要按图像分组时用 [`super::parser::H264Parser::split_access_units`]：
/// 那一侧会解析切片头，用 `first_mb_in_slice == 0` 判定图像边界。
pub fn split_access_units(nalus: &[H264Nalu]) -> Vec<Vec<H264Nalu>> {
    let mut units: Vec<Vec<H264Nalu>> = Vec::new();
    let mut current: Vec<H264Nalu> = Vec::new();
    let mut pending: Vec<H264Nalu> = Vec::new();

    let flush = |current: &mut Vec<H264Nalu>, units: &mut Vec<Vec<H264Nalu>>| {
        if !current.is_empty() {
            units.push(std::mem::take(current));
        }
    };

    for nalu in nalus {
        if nalu.nal_unit_type.is_vcl() {
            if current.iter().any(|n| n.nal_unit_type.is_vcl()) {
                flush(&mut current, &mut units);
            }
            current.append(&mut pending);
            current.push(nalu.clone());
        } else {
            pending.push(nalu.clone());
        }
    }
    flush(&mut current, &mut units);
    units
}

/// AVCC 解码配置记录（avcC）。
///
/// 对应 Chromium `media/formats/mp4/box_definitions.h` 的
/// `AVCDecoderConfigurationRecord`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvcDecoderConfigurationRecord {
    /// profile_idc（SPS 里的那个）。
    pub profile_indication: u8,
    /// profile_compatibility。
    pub profile_compatibility: u8,
    /// level_idc。
    pub level_indication: u8,
    /// NAL 长度字段的字节数（1 / 2 / 4）。
    pub nal_unit_length_size: usize,
    /// 序列参数集（每个是**含 NAL 头**的完整 NAL）。
    pub sps_list: Vec<Vec<u8>>,
    /// 图像参数集（每个是**含 NAL 头**的完整 NAL）。
    pub pps_list: Vec<Vec<u8>>,
}

impl AvcDecoderConfigurationRecord {
    /// 解析一段 avcC 记录。
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 7 {
            return None;
        }
        let profile_indication = data[1];
        let profile_compatibility = data[2];
        let level_indication = data[3];
        // 低 2 位是 lengthSizeMinusOne。
        let nal_unit_length_size = ((data[4] & 0x03) + 1) as usize;
        if nal_unit_length_size == 3 {
            // 3 是保留值，不合法。
            return None;
        }
        let num_sps = (data[5] & 0x1f) as usize;
        let mut offset = 6usize;
        let mut sps_list = Vec::with_capacity(num_sps);
        for _ in 0..num_sps {
            let (len, next) = read_u16_len(data, offset)?;
            offset = next;
            if offset + len > data.len() {
                return None;
            }
            sps_list.push(data[offset..offset + len].to_vec());
            offset += len;
        }
        if offset >= data.len() {
            return None;
        }
        let num_pps = data[offset] as usize;
        offset += 1;
        let mut pps_list = Vec::with_capacity(num_pps);
        for _ in 0..num_pps {
            let (len, next) = read_u16_len(data, offset)?;
            offset = next;
            if offset + len > data.len() {
                return None;
            }
            pps_list.push(data[offset..offset + len].to_vec());
            offset += len;
        }
        Some(Self {
            profile_indication,
            profile_compatibility,
            level_indication,
            nal_unit_length_size,
            sps_list,
            pps_list,
        })
    }
}

fn read_u16_len(data: &[u8], offset: usize) -> Option<(usize, usize)> {
    if offset + 2 > data.len() {
        return None;
    }
    let len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
    Some((len, offset + 2))
}

#[cfg(test)]
#[path = "../../tests/unit/h264_nal_tests.rs"]
mod tests;
