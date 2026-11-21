// Codec 声明表 —— `build.rs` 与 `src/` 共享的**唯一**后端清单。
//
// 这个文件被 `build.rs` 与 `src/codec_registry.rs` 同时 `include!`：
// 两边各自提供消费宏，把同一份声明派生出构建脚本的导出符号表 / 构建分发，
// 以及 Rust 侧的模块声明、re-export、`load_all()` 调用。
//
// 新增一个复用现有 vendored 库的 codec = 在 `zephyra_codecs!` 里加一行 +
// `Cargo.toml` 里加一个 feature。引入一套新的 vendored C/C++ 库时，还要在
// `zephyra_build_units!` 里加一行（C 源码列表本身属于那套库），并在
// `build.rs` 里加一个 `build_*` 函数 —— 这是唯一无法从声明派生的部分。

// 两个宏都会被两个 include 方引入，但各自只用其中一个。
#[allow(unused_macros)]
macro_rules! zephyra_codecs {
    ($emit:ident) => {
        $emit! {
            // feature     模块名        解码器类型           构建单元         归属   加载入口
            "uavs3d",    uavs3d,       Uavs3dDecoder,       build_uavs3d,    video, root;
            "av1",       av1,          Dav1dDecoder,        build_av1,       video, root;
            "vp8",       vp8,          Vpx8Decoder,         build_vp9,       video, root;
            "vp9",       vp9,          Vpx9Decoder,         build_vp9,       video, root;
            "hevc",      hevc,         HevcDecoder,         build_hevc,      video, root;
            "evc",       evc,          XevdDecoder,         build_evc,       video, root;
            "h264",      h264,         OpenH264Decoder,     build_h264,      video, nested;
            "vvc",       vvc,          VvdecDecoder,        build_vvc,       video, root;
            "theora",    theora,       TheoraDecoder,       build_theora,    video, root;
            "mjpeg",     mjpeg,        MjpegDecoder,        build_mjpeg,     video, root;
            "mp3",       mp3,          Mp3Decoder,          build_mp3,       audio, root;
            "opus",      opus,         OpusDecoder,         build_opus,      audio, root;
            "vorbis",    vorbis,       VorbisDecoder,       build_vorbis,    audio, root;
            "flac",      flac,         FlacDecoder,         build_flac,      audio, root;
        }
    };
}

// 构建单元 = 一套 vendored 库 → 一个共享库 + 一组导出符号。
// vp8 / vp9 共用 libvpx，所以只有一个 build_vp9 单元、两个 feature。
#[allow(unused_macros)]
macro_rules! zephyra_build_units {
    ($emit:ident) => {
        $emit! {
            // 构建函数        启用 feature             共享库名   导出常量          导出符号
            build_uavs3d,     ["uavs3d"],              "uavs3d",  UAVS3D_EXPORTS,  ["uavs3d_create", "uavs3d_delete", "uavs3d_reset", "uavs3d_decode", "uavs3d_flush"];
            build_av1,        ["av1"],                 "z_dav1d", DAV1D_EXPORTS,   ["z_dav1d_open", "z_dav1d_close", "z_dav1d_decode"];
            build_vp9,        ["vp8", "vp9"],          "z_vpx",   VPX_EXPORTS,     ["z_vpx8_open", "z_vpx9_open", "z_vpx_close", "z_vpx_decode"];
            build_hevc,       ["hevc"],                "z_hevc",  HEVC_EXPORTS,    ["z_hevc_open", "z_hevc_close", "z_hevc_reset", "z_hevc_decode", "z_hevc_flush"];
            build_evc,        ["evc"],                 "z_evc",   EVC_EXPORTS,     ["z_evc_open", "z_evc_close", "z_evc_decode"];
            build_h264,       ["h264"],                "z_h264",  H264_EXPORTS,    ["z_h264_open", "z_h264_close", "z_h264_decode", "z_h264_flush"];
            build_vvc,        ["vvc"],                 "z_vvc",   VVC_EXPORTS,     ["z_vvc_open", "z_vvc_close", "z_vvc_decode", "z_vvc_flush"];
            build_theora,     ["theora"],              "z_theora", THEORA_EXPORTS,["z_theora_open", "z_theora_close", "z_theora_decode", "z_theora_flush"];
            build_mjpeg,      ["mjpeg"],               "z_mjpeg", MJPEG_EXPORTS,   ["z_mjpeg_open", "z_mjpeg_close", "z_mjpeg_decode"];
            build_mp3,        ["mp3"],                 "z_mp3",   MP3_EXPORTS,     ["z_mp3_open", "z_mp3_close", "z_mp3_decode"];
            build_opus,       ["opus"],                "z_opus",  OPUS_EXPORTS,    ["z_opus_open", "z_opus_close", "z_opus_decode"];
            build_vorbis,     ["vorbis"],              "z_vorbis",VORBIS_EXPORTS,  ["z_vorbis_open", "z_vorbis_close", "z_vorbis_decode"];
            build_flac,       ["flac"],                "z_flac",  FLAC_EXPORTS,    ["z_flac_open", "z_flac_close", "z_flac_decode"];
        }
    };
}
