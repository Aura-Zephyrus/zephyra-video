//! H.264 位流层：NAL 切分、参数集解析、AVCC ↔ Annex-B 转换。
//!
//! 这一层是**纯逻辑**：不碰 GPU、不碰系统 API，因此可以在任何机器上单测。
//! 它存在的理由是硬件解码的**前提** —— DXVA2 要的是解析出来的
//! `DXVA_PicParams_H264`，不是一整段码流。

pub mod bit_reader;
pub mod bit_writer;
pub mod converter;
#[cfg(feature = "h264")]
#[allow(unsafe_code)]
pub mod decoder;
pub mod nal;
pub mod parser;

pub use bit_reader::H264BitReader;
pub use bit_writer::H264BitstreamWriter;
pub use converter::{
    annex_b_to_avcc, extract_parameter_sets, is_access_unit_boundary_nal, H264ToAnnexBConverter,
    START_CODE_PREFIX,
};
#[cfg(feature = "h264")]
pub use decoder::OpenH264Decoder;
pub use nal::{
    find_start_code, parse_nalus, split_access_units, AvcDecoderConfigurationRecord, H264Nalu,
    NalUnitType,
};
pub use parser::{
    parse_pps_payload, parse_slice_header_payload, parse_sps_payload, DecRefPicMarking,
    H264ParseError, H264Parser, H264Pps, H264Slice, H264SliceHeader, H264Sps, SliceType,
};
