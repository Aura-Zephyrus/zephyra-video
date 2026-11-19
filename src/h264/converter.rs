//! AVCC ↔ Annex-B 位流转换。
//!
//! 对应 Chromium `media/filters/h264_to_annex_b_bitstream_converter.cc`
//! （`H264ToAnnexBBitstreamConverter`）与
//! `media/formats/mp4/h264_annex_b_to_avc_bitstream_converter.cc`。
//!
//! # 为什么需要它
//!
//! 容器（MP4）里存的是 **AVCC**：NAL 用长度前缀，参数集在 `avcC` 盒里。
//! 而 DXVA2 的 `ID3D11VideoDecoder` 只吃 **Annex-B**：NAL 用起始码，
//! 参数集在码流里。Chromium 的做法就是在解码前插一个转换器
//! （`FFmpegH264ToAnnexBBitstreamConverter` / `H264ToAnnexBBitstreamConverter`）。
//!
//! # 起始码的长度规则
//!
//! Chromium 的写入规则是：**一个访问单元的第一个 NAL 用四字节起始码**
//! （`00 00 00 01`），其余用三字节（`00 00 01`）。
//! 「访问单元的第一个」是**跨包**记录的 —— 转换器持有这个状态。

use super::nal::{AvcDecoderConfigurationRecord, H264Nalu, NalUnitType};

/// 三字节起始码。
pub const START_CODE_PREFIX: [u8; 3] = [0, 0, 1];

/// 该 NAL 类型是否标记**访问单元边界**。
///
/// 对应 Chromium `IsAccessUnitBoundaryNal()`：SEI / PPS / SPS / AUD
/// 以及 14..18 的保留类型。
pub fn is_access_unit_boundary_nal(nal_unit_type: NalUnitType) -> bool {
    matches!(
        nal_unit_type,
        NalUnitType::SeiMessage
            | NalUnitType::Pps
            | NalUnitType::Sps
            | NalUnitType::Aud
            | NalUnitType::Prefix
            | NalUnitType::SubsetSps
            | NalUnitType::Dps
            | NalUnitType::Unspecified24
            | NalUnitType::Unspecified25
            | NalUnitType::Unspecified26
            | NalUnitType::Unspecified27
            | NalUnitType::Unspecified28
    )
}

/// 该 NAL 是否应当**丢弃**。
///
/// 对应 Chromium `ShouldSkipNalUnit()`：某些编码器会插入长度为 1 的
/// 「未指定类型」NAL 充数。
fn should_skip_nal_unit(nal_unit_length: usize, nal_unit_type: NalUnitType) -> bool {
    nal_unit_length == 1 && nal_unit_type == NalUnitType::Unspecified
}

/// AVCC → Annex-B 转换器。
///
/// 状态：跨包记住「上一个包的最后一个 NAL 是不是访问单元的第一个」。
pub struct H264ToAnnexBConverter {
    config: AvcDecoderConfigurationRecord,
    /// 参数集是否已经写进过码流（只需写一次）。
    configuration_processed: bool,
    /// 下一个包是否是一个新访问单元的第一个 NAL。
    first_nal_unit_in_access_unit: bool,
}

impl H264ToAnnexBConverter {
    /// 用 avcC 配置记录新建转换器。
    pub fn new(config: AvcDecoderConfigurationRecord) -> Self {
        Self {
            config,
            configuration_processed: false,
            first_nal_unit_in_access_unit: true,
        }
    }

    /// 复位（换流 / seek 之后要调用）。
    pub fn reset(&mut self) {
        self.configuration_processed = false;
        self.first_nal_unit_in_access_unit = true;
    }

    /// 把一个 AVCC 包转换成 Annex-B。
    ///
    /// 参数集（SPS / PPS）会被写进**第一个**转换后的包的开头 ——
    /// 与 Chromium 一致（`ConvertAVCDecoderConfigToByteStream`）。
    pub fn convert(&mut self, packet: &[u8]) -> Option<Vec<u8>> {
        if packet.is_empty() {
            return None;
        }
        let mut output: Vec<u8> = Vec::with_capacity(packet.len() + 64);
        let mut nal_units = self.split_avcc(packet)?;
        if !self.configuration_processed {
            // 参数集放在最前面。
            for sps in &self.config.sps_list {
                write_nal_with_start_code(&mut output, sps, 4);
            }
            for pps in &self.config.pps_list {
                write_nal_with_start_code(&mut output, pps, 4);
            }
            self.configuration_processed = true;
        }
        let mut first_in_au = self.first_nal_unit_in_access_unit;
        for nal in nal_units.drain(..) {
            // Chromium 的规则（`h264_to_annex_b_bitstream_converter.cc`）：
            // **访问单元的第一个 NAL 用四字节起始码**，其余用三字节。
            // 而「访问单元边界 NAL」（SPS / PPS / SEI / AUD …）**自己**就是
            // 一个访问单元的开始 —— 它拿四字节起始码，**它后面那个** NAL
            // 拿三字节。
            let is_boundary = is_access_unit_boundary_nal(nal.nal_unit_type);
            let start_code_len = if first_in_au || is_boundary { 4 } else { 3 };
            first_in_au = false;
            write_nal_with_start_code(&mut output, &nal.data, start_code_len);
        }
        self.first_nal_unit_in_access_unit = first_in_au;
        Some(output)
    }

    /// 把 AVCC 包切成 NAL 单元（长度前缀 → 完整 NAL，含头字节）。
    fn split_avcc(&self, packet: &[u8]) -> Option<Vec<H264Nalu>> {
        let width = self.config.nal_unit_length_size;
        let mut out = Vec::new();
        let mut offset = 0usize;
        while offset < packet.len() {
            if offset + width > packet.len() {
                return None;
            }
            let mut length = 0usize;
            for i in 0..width {
                length = (length << 8) | usize::from(packet[offset + i]);
            }
            offset += width;
            if length == 0 {
                // 没有更多数据了。
                break;
            }
            if offset + length > packet.len() {
                return None;
            }
            let data = &packet[offset..offset + length];
            offset += length;
            let nal_type = NalUnitType::from_byte(data[0]);
            if should_skip_nal_unit(length, nal_type) {
                continue;
            }
            out.push(H264Nalu {
                nal_ref_idc: (data[0] >> 5) & 0x03,
                nal_unit_type: nal_type,
                data: data.to_vec(),
            });
        }
        Some(out)
    }
}

/// 写一个 NAL（含起始码）。
fn write_nal_with_start_code(output: &mut Vec<u8>, nal: &[u8], start_code_len: usize) {
    if start_code_len == 4 {
        output.push(0);
    }
    output.extend_from_slice(&START_CODE_PREFIX);
    output.extend_from_slice(nal);
}

/// 把 Annex-B 码流里的 NAL 重新包成 AVCC（长度前缀）。
///
/// 对应 Chromium `H264AnnexBToAvcBitstreamConverter::ConvertChunk()` 的
/// 「NAL → AVCC」那一半；`length_size` 固定为 4（Chromium 也固定为 4）。
pub fn annex_b_to_avcc(stream: &[u8]) -> Vec<Vec<u8>> {
    let nalus = super::nal::parse_nalus(stream);
    let mut out = Vec::with_capacity(nalus.len());
    for nal in nalus {
        let mut packet = Vec::with_capacity(nal.data.len() + 4);
        packet.extend_from_slice(&(nal.data.len() as u32).to_be_bytes());
        packet.extend_from_slice(&nal.data);
        out.push(packet);
    }
    out
}

/// 从 Annex-B 码流里抽出参数集（SPS / PPS），按 avcC 的顺序（SPS 在前）。
pub fn extract_parameter_sets(stream: &[u8]) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let nalus = super::nal::parse_nalus(stream);
    let mut sps = Vec::new();
    let mut pps = Vec::new();
    for nal in nalus {
        match nal.nal_unit_type {
            NalUnitType::Sps => sps.push(nal.data),
            NalUnitType::Pps => pps.push(nal.data),
            _ => {}
        }
    }
    (sps, pps)
}

#[cfg(test)]
#[path = "../../tests/unit/h264_converter_tests.rs"]
mod tests;
