//! vendored uavs3d 的 C 源码编译。
//!
//! 只在 `uavs3d` feature 打开时做事：默认构建里连 `cc` 都不是依赖，
//! 这个脚本的 `main` 是空的。Cargo 会把 feature 作为 `--cfg` 传进来
//! （实测：build script 的 `#[cfg(feature = "...")]` 生效），所以这里
//! 既能保持 `cc` 可选，又不必在无 feature 时编译它。
//!
//! 编译的源码是 `native/uavs3d/source/` 的**逐字节副本**（BSD-3，见
//! `native/uavs3d/COPYING`）。编译选项照抄 `native/uavs3d/source/CMakeLists.txt`：
//! x86 上 SSE4 源文件用 `-msse4.2`（MSVC 默认即可）、AVX2 源文件单独编一份
//! 并加 `/arch:AVX2`（gcc 用 `-mavx2`）。

// ---- 动态/静态双路径 ----
//
// `dynamic` feature 打开时，把 vendored C 源码编成共享库（.dll/.so/.dylib），
// 并通过 `cargo:rustc-link-lib=dylib=<name>` 让 Rust 的 `extern "C"` 在链接期
// 指向它 —— 6 个 codec 模块的源码一行都不用改。共享库直接生成到 exe 所在目录。

// 构建侧的导出符号表、共享 helper 的启用条件与 main 分发全部由 codecs.rs 生成。
// 新增 codec 时只改那张表 + Cargo.toml 的 feature（新 vendored 库另需一个 build_* 函数）。
include!("codecs.rs");

macro_rules! build_support_decl {
    ($( $build:ident, [$($feature:literal),*], $lib:literal, $exports:ident, [$($export:literal),*]; )*) => {
        $(
            #[allow(dead_code)] // 静态模式不读导出符号。
            #[cfg(any($(feature = $feature),*))]
            const $exports: &[&str] = &[$($export),*];
        )*

        #[cfg(any($( $(feature = $feature),* ,)*))]
        #[allow(dead_code)]
        mod build_support;

        fn dispatch_builds() {
            $(
                #[cfg(any($(feature = $feature),*))]
                { $build(); }
            )*
        }
    };
}
zephyra_build_units!(build_support_decl);

/// CLI 通过 ZEPHYRA_LINK_MODE 选模式；显式 feature 也可打开。
#[allow(dead_code)]
fn dynamic_mode() -> bool {
    cfg!(feature = "dynamic")
        || std::env::var("ZEPHYRA_LINK_MODE")
            .map(|v| v == "dynamic")
            .unwrap_or(false)
}

fn main() {
    // 构建哪几个单元由 codecs.rs 的 zephyra_build_units! 表决定。
    dispatch_builds();
}

#[cfg(feature = "uavs3d")]
fn build_uavs3d() {
    use std::path::Path;

    let src = Path::new("native/uavs3d/source");
    println!("cargo:rerun-if-changed=native/uavs3d");

    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let system_includes = if msvc {
        build_support::msvc_system_includes()
    } else {
        Vec::new()
    };

    let mut base = cc::Build::new();
    base.include(src.join("decore"));
    for dir in &system_includes {
        base.include(dir);
    }
    base
        // 8bit 4:2:0 ⇒ NV12。10bit 需要 `COMPILE_10BIT=1` 且 Rust 侧要有
        // P010LE 的帧布局，那是另一件事。
        .define("COMPILE_10BIT", "0")
        // vendored 的 C 代码自带大量平台相关告警；它们不是本 crate 的问题。
        .warnings(false);

    for dir in ["decoder", "decore"] {
        for file in build_support::c_files(src.join(dir)) {
            base.file(file);
        }
    }

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if arch == "x86_64" || arch == "x86" {
        for file in build_support::c_files(src.join("decore/sse")) {
            base.file(file);
        }

        // AVX2 必须单独编：MSVC 要 /arch:AVX2 才认 AVX2 intrinsic，
        // 而整个库都按 AVX2 编会让没有 AVX2 的机器直接崩。CMakeLists.txt
        // 也是这么分的（sse 与 avx2 两组源码、两组编译 flags）。
        let mut avx2 = cc::Build::new();
        avx2.include(src.join("decore"));
        for dir in &system_includes {
            avx2.include(dir);
        }
        avx2.define("COMPILE_10BIT", "0").warnings(false);
        for file in build_support::c_files(src.join("decore/avx2")) {
            avx2.file(file);
        }
        if msvc {
            avx2.flag("/arch:AVX2");
        } else {
            avx2.flag_if_supported("-mavx2");
        }
        build_support::finish_uavs3d(&mut base, Some(&mut avx2));
    } else {
        // ARM / 其它平台：CMakeLists 用的是 .S 汇编，这里不编，
        // 退到 decore 的纯 C 路径（uavs3d 仍然能解码，只是慢）。
        build_support::finish_uavs3d(&mut base, None);
    }

    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os != "windows" {
        println!("cargo:rustc-link-lib=m");
        println!("cargo:rustc-link-lib=pthread");
    }
}

/// cc-rs 在这台机器上没把 MSVC / Windows SDK 的头文件目录传给 `cl.exe`
/// （症状：`time.h: No such file or directory`）。这里直接检测出那些目录，
/// 以 `-I` 的形式加进去 —— 只影响 C 编译，不改全局环境变量。
#[cfg(feature = "av1")]
fn build_av1() {
    use std::path::Path;

    let root = Path::new("native/dav1d");
    println!("cargo:rerun-if-changed=native/dav1d");
    println!("cargo:rerun-if-changed=native/dav1d_shim.c");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    // Chromium 的目录名是 `x64` / `x86`，Rust 的 target arch 是 `x86_64` / `x86`。
    let chromium_arch = if arch == "x86_64" { "x64" } else { &arch };
    let config = if os == "windows" {
        format!("config/win/{chromium_arch}")
    } else if os == "macos" {
        format!("config/apple/{chromium_arch}")
    } else if arch == "x86_64" || arch == "x86" {
        format!("config/linux/{chromium_arch}")
    } else {
        "config/linux-noasm/generic".to_string()
    };

    let mut build = cc::Build::new();
    build
        .include(root.join(&config))
        .include(root.join("libdav1d"))
        .include(root.join("libdav1d/include"))
        .include(root.join("libdav1d/include/dav1d"))
        .include(root.join("libdav1d/src"))
        .include(root.join("version"))
        .warnings(false);
    if !msvc {
        build.define("_FILE_OFFSET_BITS", "64");
        build.define("_POSIX_C_SOURCE", "200112L");
    }
    if msvc {
        for dir in build_support::msvc_system_includes() {
            build.include(dir);
        }
        // MSVC 14.5x 的 C 运行时把 C11 atomics 放进了 stdatomic.h，
        // 不开 /std:c11 会直接 #error。
        build.flag("/std:c11");
        // MSVC 默认定义 __STDC_NO_ATOMICS__；dav1d 用 C11 atomics，
        // 两个开关一起开才能用 <stdatomic.h>。
        build.flag("/experimental:c11atomics");
        // 不让 dav1d 导出符号（Chromium 的 public_dav1d_config 同款）。
        build.define("DAV1D_API", "");
    }
    build.define("CONFIG_LOG", "0");

    // 非模板 C 源码（Chromium 的 c_sources 目标）。
    for source in build_support::gni_sources(&root.join("dav1d_generated.gni"), "c_sources") {
        build.file(root.join(source));
    }
    // Chromium 的 dav1d_c 目标额外加 win32 线程实现。
    if os == "windows" {
        build.file(root.join("libdav1d/src/win32/thread.c"));
    }
    // x86 CPU 探测（Chromium 的 dav1d_x86 目标）。
    if arch == "x86_64" || arch == "x86" {
        build.file(root.join("libdav1d/src/x86/cpu.c"));
    }
    // 模板源码按 8bit / 16bit 各编一遍（Chromium 的 dav1d_8bit / 10bit 目标）。
    // 生成两个 wrapper：#define BITDEPTH 后 include 原始模板 —— 这样所有东西
    // 都在**同一个** cc::Build 里编成一个 lib，避免多次 compile() 时的
    // 同名 .o / .lib 冲突（cc-rs 在这台机器上会报 hard-link 失败）。
    let templates =
        build_support::gni_sources(&root.join("dav1d_generated.gni"), "template_sources");
    let generated = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"))
        .join("dav1d_templates");
    std::fs::create_dir_all(&generated).expect("create dav1d template dir");
    for source in &templates {
        // 不能用 canonicalize()：Windows 上它返回 \\?\ 前缀路径，cl.exe 的
        // #include 认不了。用 cwd + 相对路径拼一个普通绝对路径。
        let original = std::env::current_dir()
            .expect("cwd")
            .join(root)
            .join(source);
        let name = std::path::Path::new(source)
            .file_stem()
            .expect("template has a filename")
            .to_string_lossy();
        for bitdepth in [8, 16] {
            let wrapper = generated.join(format!("{name}_{bitdepth}.c"));
            std::fs::write(
                &wrapper,
                format!(
                    "#define BITDEPTH {bitdepth}\n#include {:?}\n",
                    original.to_string_lossy().replace('\\', "/")
                ),
            )
            .expect("write dav1d template wrapper");
            build.file(wrapper);
        }
    }

    build.file(root.parent().unwrap().join("dav1d_shim.c"));
    build_support::finish_codec(&mut build, "z_dav1d", DAV1D_EXPORTS);
}

#[cfg(any(feature = "vp9", feature = "vp8"))]
fn build_vp9() {
    use std::path::Path;

    let root = Path::new("native/libvpx");
    println!("cargo:rerun-if-changed=native/libvpx");
    println!("cargo:rerun-if-changed=native/libvpx_shim.c");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let chromium_arch = if arch == "x86_64" { "x64" } else { &arch };
    let config = if os == "windows" {
        format!("config/win/{chromium_arch}")
    } else if os == "macos" {
        format!("config/mac/{chromium_arch}")
    } else if arch == "x86_64" || arch == "x86" {
        format!("config/linux/{chromium_arch}")
    } else {
        "config/linux/generic".to_string()
    };

    let mut build = cc::Build::new();
    // 与 Chromium 的 libvpx_include_dirs 一致；额外的 noasm_rtcd 放最前，
    // 让 #include "./vp9_rtcd.h" 命中无汇编版（win/x64 的 rtcd 头无条件
    // 映射到 *.asm 里的 *_sse2 符号，而本机没有 nasm/yasm 编那些 asm）。
    build
        .include(root.join("config/noasm_rtcd"))
        .include(root.join("config"))
        .include(root)
        .include(root.join(&config))
        .define("CHROMIUM", None)
        .warnings(false);
    if msvc {
        for dir in build_support::msvc_system_includes() {
            build.include(dir);
        }
        build.flag("/std:c11");
    }

    let variable = if arch == "x86_64" {
        "libvpx_srcs_x86_64"
    } else if arch == "x86" {
        "libvpx_srcs_x86"
    } else {
        "libvpx_srcs_generic"
    };
    const PREFIX: &str = "//third_party/libvpx/source/libvpx/";
    let gni = root.join("libvpx_srcs.gni");
    let mut all_sources = build_support::gni_sources(&gni, variable);
    // Chromium 的 win/x64 rtcd 头**无条件**映射到 *_sse2 / *_ssse3 等符号，
    // 即使 HAVE_X86_ASM=0 也要把对应的 intrinsic C 文件编进来（它们是 .c，
    // 不是需要 nasm 的 .asm）。
    let simd_suffix = variable.strip_prefix("libvpx_srcs_").unwrap_or("x86_64");
    for group in ["mmx", "sse2", "ssse3", "sse4_1"] {
        all_sources.extend(build_support::gni_sources(
            &gni,
            &format!("libvpx_srcs_{simd_suffix}_{group}"),
        ));
    }
    for source in all_sources {
        let relative = source.strip_prefix(PREFIX).unwrap_or(&source);
        build.file(root.join(relative));
    }
    build.file(root.parent().unwrap().join("libvpx_shim.c"));
    build_support::finish_codec(&mut build, "z_vpx", VPX_EXPORTS);

    if os != "windows" {
        println!("cargo:rustc-link-lib=pthread");
    }
}

#[cfg(feature = "hevc")]
fn build_hevc() {
    use std::path::Path;

    let root = Path::new("native/libde265");
    println!("cargo:rerun-if-changed=native/libde265");
    println!("cargo:rerun-if-changed=native/hevc_shim.cpp");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .include(root.join("libde265"))
        .include(root)
        .define("LIBDE265_STATIC_BUILD", None)
        .warnings(false);
    if msvc {
        for dir in build_support::msvc_system_includes() {
            build.include(dir);
        }
        build.flag("/std:c++11");
    } else {
        build.flag_if_supported("-std=c++11");
    }

    // libde265/CMakeLists.txt 的源文件列表（不含 encoder/ 与 SSE 子目录）。
    let sources = [
        "alloc_pool.cc",
        "bitstream.cc",
        "cabac.cc",
        "configparam.cc",
        "contextmodel.cc",
        "de265.cc",
        "deblock.cc",
        "decctx.cc",
        "dpb.cc",
        "en265.cc",
        "fallback-dct.cc",
        "fallback-motion.cc",
        "fallback.cc",
        "image-io.cc",
        "image.cc",
        "intrapred.cc",
        "md5.cc",
        "motion.cc",
        "nal-parser.cc",
        "nal.cc",
        "pps.cc",
        "quality.cc",
        "refpic.cc",
        "sao.cc",
        "scan.cc",
        "sei.cc",
        "slice.cc",
        "sps.cc",
        "threads.cc",
        "transform.cc",
        "util.cc",
        "visualize.cc",
        "vps.cc",
        "vui.cc",
    ];
    for source in sources {
        build.file(root.join("libde265").join(source));
    }
    if msvc || os == "windows" {
        build.file(root.join("extra/win32cond.c"));
    }
    build.file(root.parent().unwrap().join("hevc_shim.cpp"));
    build_support::finish_codec(&mut build, "z_hevc", HEVC_EXPORTS);

    if os != "windows" {
        println!("cargo:rustc-link-lib=pthread");
    }
}

/// EVC (MPEG-5) decoding via vendored xevd (BSD-3-Clause, Samsung).
///
/// Only `src_base/` is compiled. Without `X86`/`ARM` defines xevd_port.h
/// leaves `X86_SSE`/`ARM_NEON` undefined, so the pure-C fallback is used and
/// no SIMD sources / assembler are needed.
#[cfg(feature = "evc")]
fn build_evc() {
    use std::path::{Path, PathBuf};

    let root = Path::new("native/xevd");
    println!("cargo:rerun-if-changed=native/xevd");
    println!("cargo:rerun-if-changed=native/xevd_shim.c");

    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    let mut build = cc::Build::new();
    build
        .include(root.join("inc"))
        .include(root.join("src_base"))
        .define("ANY", None)
        // xevd uses assert() for malformed-bitstream checks; without NDEBUG a
        // bad stream aborts the whole process instead of returning an error.
        .define("NDEBUG", None)
        .warnings(false);
    if msvc {
        for dir in build_support::msvc_system_includes() {
            build.include(dir);
        }
        build.define("_CRT_SECURE_NO_WARNINGS", None);
    }
    // xevd_tp.c gates its Win32 thread pool on `WIN32`/`WIN64` (not `_WIN32`).
    if os == "windows" {
        build.define("WIN32", None);
    } else {
        build.define("LINUX", None);
    }

    // Every base .c under src_base (excluding the sse/avx/neon subdirs).
    let base = root.join("src_base");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&base)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", base.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("c"))
        .collect();
    files.sort();
    for file in files {
        build.file(file);
    }

    build.file(root.parent().unwrap().join("xevd_shim.c"));
    build_support::finish_codec(&mut build, "z_evc", EVC_EXPORTS);

    if os != "windows" {
        println!("cargo:rustc-link-lib=m");
        println!("cargo:rustc-link-lib=pthread");
    }
}

/// H.264 software decoding via vendored OpenH264 (BSD-2-Clause, Cisco).
///
/// Only the decoder half is compiled (common + decoder core + `plus`), with
/// `X86_ASM` undefined so the generic C++ fallbacks are used — no nasm/yasm.
#[cfg(feature = "h264")]
fn build_h264() {
    use std::path::Path;

    let root = Path::new("native/openh264");
    println!("cargo:rerun-if-changed=native/openh264");
    println!("cargo:rerun-if-changed=native/openh264_shim.c");

    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    // OpenH264's build script generates this from version_gen.h.template.
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(
        out.join("version_gen.h"),
        "#ifndef VERSION_GEN_H\n#define VERSION_GEN_H\n#define VERSION_NUMBER \"2.5.0\"\n#endif\n",
    )
    .expect("write OpenH264 version_gen.h");

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .include(out.clone())
        .include(root.join("codec/api/wels"))
        .include(root.join("codec/common/inc"))
        .include(root.join("codec/decoder/core/inc"))
        .include(root.join("codec/decoder/plus/inc"))
        .define("NDEBUG", None)
        .warnings(false);
    if msvc {
        for dir in build_support::msvc_system_includes() {
            build.include(dir);
        }
        build.flag("/std:c++11");
    } else {
        build.flag_if_supported("-std=c++11");
    }

    // Exact source lists from codec/{common,decoder}/targets.mk (Makefile).
    const COMMON: &[&str] = &[
        "common_tables.cpp",
        "copy_mb.cpp",
        "cpu.cpp",
        "crt_util_safe_x.cpp",
        "deblocking_common.cpp",
        "expand_pic.cpp",
        "intra_pred_common.cpp",
        "mc.cpp",
        "memory_align.cpp",
        "sad_common.cpp",
        "utils.cpp",
        "welsCodecTrace.cpp",
        "WelsTaskThread.cpp",
        "WelsThread.cpp",
        "WelsThreadLib.cpp",
        "WelsThreadPool.cpp",
    ];
    const DECODER: &[&str] = &[
        "au_parser.cpp",
        "bit_stream.cpp",
        "cabac_decoder.cpp",
        "deblocking.cpp",
        "decode_mb_aux.cpp",
        "decode_slice.cpp",
        "decoder.cpp",
        "decoder_core.cpp",
        "decoder_data_tables.cpp",
        "error_concealment.cpp",
        "fmo.cpp",
        "get_intra_predictor.cpp",
        "manage_dec_ref.cpp",
        "memmgr_nal_unit.cpp",
        "mv_pred.cpp",
        "parse_mb_syn_cabac.cpp",
        "parse_mb_syn_cavlc.cpp",
        "pic_queue.cpp",
        "rec_mb.cpp",
        "wels_decoder_thread.cpp",
    ];
    for source in COMMON {
        build.file(root.join("codec/common/src").join(source));
    }
    for source in DECODER {
        build.file(root.join("codec/decoder/core/src").join(source));
    }
    build.file(root.join("codec/decoder/plus/src/welsDecoderExt.cpp"));
    build.file(root.parent().unwrap().join("openh264_shim.c"));
    build_support::finish_codec(&mut build, "z_h264", H264_EXPORTS);

    if os != "windows" {
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=dl");
    }
}

/// VVC / H.266 decoding via vendored vvdec (Clear BSD, Fraunhofer HHI).
///
/// X86/ARM SIMD sources are skipped (no nasm and to keep the build portable);
/// vvdec's scalar fallbacks are used. simde is still needed by BitStream.h.
#[cfg(feature = "vvc")]
fn build_vvc() {
    use std::path::{Path, PathBuf};

    let root = Path::new("native/vvdec");
    println!("cargo:rerun-if-changed=native/vvdec");
    println!("cargo:rerun-if-changed=native/vvdec_shim.cpp");

    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    // CMake configures these two headers into ${CMAKE_BINARY_DIR}.
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let generated = out.join("vvdec");
    std::fs::create_dir_all(&generated).expect("create vvdec generated dir");
    let header_in =
        std::fs::read_to_string(root.join("include/vvdec/vvdec.h.in")).expect("read vvdec.h.in");
    std::fs::write(
        generated.join("vvdec.h"),
        header_in.replace("@VVDEC_USE_UNSTABLE_API@", "0"),
    )
    .expect("write generated vvdec.h");
    std::fs::write(
        generated.join("version.h"),
        "#ifndef VVDEC_VERSION\n#define VVDEC_VERSION \"3.3.0\"\n#define VVDEC_VERSION_MAJOR 3\n#define VVDEC_VERSION_MINOR 3\n#define VVDEC_VERSION_PATCH 0\n#endif\n",
    )
    .expect("write generated vvdec version.h");

    let lib = root.join("source/Lib");
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .include(out.clone())
        .include(root.join("include"))
        // vvdecimpl.h includes "DecoderLib/DecLib.h" etc. relative to source/Lib.
        .include(lib.clone())
        .include(lib.join("vvdec"))
        .include(lib.join("CommonLib"))
        .include(lib.join("CommonLib/x86"))
        .include(lib.join("DecoderLib"))
        .include(lib.join("Utilities"))
        .include(lib.join("libmd5"))
        .include(lib.join("FilmGrain"))
        .include(root.join("thirdparty"))
        .define("VVDEC_SOURCE", None)
        .warnings(false);
    if msvc {
        for dir in build_support::msvc_system_includes() {
            build.include(dir);
        }
    } else {
        build.flag_if_supported("-std=c++17");
    }

    // Mirrors source/Lib/vvdec/CMakeLists.txt with SIMD disabled.
    for dir in ["vvdec", "CommonLib", "Utilities", "DecoderLib", "libmd5"] {
        let mut files: Vec<PathBuf> = std::fs::read_dir(lib.join(dir))
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir))
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("cpp"))
            .collect();
        files.sort();
        for file in files {
            build.file(file);
        }
    }
    for source in ["FilmGrain.cpp", "FilmGrainImpl.cpp"] {
        build.file(lib.join("FilmGrain").join(source));
    }
    build.file(root.parent().unwrap().join("vvdec_shim.cpp"));
    build_support::finish_codec(&mut build, "z_vvc", VVC_EXPORTS);

    if os != "windows" {
        println!("cargo:rustc-link-lib=pthread");
    }
}

/// Theora decoding via vendored libtheora (BSD-3-Clause, Xiph.org).
///
/// Only the decoder half (`decoder_sources` from lib/Makefile.am) is compiled;
/// SIMD sources are skipped so no assembler is needed. The matching libogg
/// headers (also BSD-3) are vendored alongside — libtheora only uses the
/// `ogg_packet` type, no libogg symbols.
#[cfg(feature = "theora")]
fn build_theora() {
    use std::path::Path;

    let root = Path::new("native/theora");
    println!("cargo:rerun-if-changed=native/theora");
    println!("cargo:rerun-if-changed=native/theora_shim.c");

    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    let mut build = cc::Build::new();
    build
        .include(root.join("include"))
        .include(root.join("lib"))
        .warnings(false);
    if msvc {
        for dir in build_support::msvc_system_includes() {
            build.include(dir);
        }
        build.define("_CRT_SECURE_NO_WARNINGS", None);
    }

    // lib/Makefile.am: decoder_sources (without the optional arch SIMD).
    const SOURCES: &[&str] = &[
        "apiwrapper.c",
        "bitpack.c",
        "decapiwrapper.c",
        "decinfo.c",
        "decode.c",
        "dequant.c",
        "fragment.c",
        "huffdec.c",
        "idct.c",
        "info.c",
        "internal.c",
        "quant.c",
        "state.c",
    ];
    for source in SOURCES {
        build.file(root.join("lib").join(source));
    }
    build.file(root.parent().unwrap().join("theora_shim.c"));
    build_support::finish_codec(&mut build, "z_theora", THEORA_EXPORTS);

    if os != "windows" {
        println!("cargo:rustc-link-lib=m");
    }
}

/// MJPEG decoding via vendored libjpeg-turbo 2.1.5.1 (BSD-3 / IJG license).
///
/// SIMD is disabled (`WITH_SIMD 0`), so no nasm/yasm is needed. `jconfig.h`
/// and `jversion.h` are generated into OUT_DIR from the upstream templates.
#[cfg(feature = "mjpeg")]
fn build_mjpeg() {
    use std::path::Path;

    let root = Path::new("native/libjpeg");
    println!("cargo:rerun-if-changed=native/libjpeg");
    println!("cargo:rerun-if-changed=native/mjpeg_shim.c");

    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");

    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(
        out.join("jconfig.h"),
        "#define JPEG_LIB_VERSION 62\n\
         #define LIBJPEG_TURBO_VERSION \"2.1.5.1\"\n\
         #define LIBJPEG_TURBO_VERSION_NUMBER 21051\n\
         #define MEM_SRCDST_SUPPORTED 1\n\
         #define WITH_SIMD 0\n\
         #define BITS_IN_JSAMPLE 8\n",
    )
    .expect("write jconfig.h");
    let version = std::fs::read_to_string(root.join("jversion.h.in")).expect("read jversion.h.in");
    std::fs::write(
        out.join("jversion.h"),
        version.replace("@COPYRIGHT_YEAR@", "2026"),
    )
    .expect("write jversion.h");

    let mut build = cc::Build::new();
    build
        .include(out.clone())
        .include(root)
        // jconfigint.h.in is configured by CMake; supply a minimal one.
        .warnings(false);
    if msvc {
        for dir in build_support::msvc_system_includes() {
            build.include(dir);
        }
        build.define("_CRT_SECURE_NO_WARNINGS", None);
    }

    // jconfigint.h is generated by CMake from jconfigint.h.in. The sources
    // use SIZEOF_SIZE_T in #if, so it must be a literal, and INLINE for the
    // compiler in use.
    let pointer_width =
        std::env::var("CARGO_CFG_TARGET_POINTER_WIDTH").unwrap_or_else(|_| "64".to_string());
    let size_of_size_t = pointer_width.parse::<usize>().unwrap_or(64) / 8;
    let inline = if msvc { "__inline" } else { "inline" };
    std::fs::write(
        out.join("jconfigint.h"),
        format!(
            "#ifndef JCONFIGINT_H\n#define JCONFIGINT_H\n\
             #define BUILD \"\"\n\
             #define INLINE {inline}\n\
             #define THREAD_LOCAL\n\
             #define PACKAGE_NAME \"libjpeg-turbo\"\n\
             #define VERSION \"2.1.5.1\"\n\
             #define SIZEOF_SIZE_T {size_of_size_t}\n\
             #define FALLTHROUGH\n\
             #endif\n"
        ),
    )
    .expect("write jconfigint.h");

    // CMakeLists.txt JPEG_SOURCES (2.1.x), SIMD files skipped.
    const SOURCES: &[&str] = &[
        "jcapimin.c",
        "jcapistd.c",
        "jccoefct.c",
        "jccolor.c",
        "jcdctmgr.c",
        "jchuff.c",
        "jcicc.c",
        "jcinit.c",
        "jcmainct.c",
        "jcmarker.c",
        "jcmaster.c",
        "jcomapi.c",
        "jcparam.c",
        "jcphuff.c",
        "jcprepct.c",
        "jcsample.c",
        "jctrans.c",
        "jdapimin.c",
        "jdapistd.c",
        "jdatadst.c",
        "jdatasrc.c",
        "jdcoefct.c",
        "jdcolor.c",
        "jddctmgr.c",
        "jdhuff.c",
        "jdicc.c",
        "jdinput.c",
        "jdmainct.c",
        "jdmarker.c",
        "jdmaster.c",
        "jdmerge.c",
        "jdphuff.c",
        "jdpostct.c",
        "jdsample.c",
        "jdtrans.c",
        "jerror.c",
        "jfdctflt.c",
        "jfdctfst.c",
        "jfdctint.c",
        "jidctflt.c",
        "jidctfst.c",
        "jidctint.c",
        "jidctred.c",
        "jquant1.c",
        "jquant2.c",
        "jutils.c",
        "jmemmgr.c",
        "jmemnobs.c",
        "jsimd_none.c",
    ];
    for source in SOURCES {
        build.file(root.join(source));
    }
    build.file(root.parent().unwrap().join("mjpeg_shim.c"));
    build_support::finish_codec(&mut build, "z_mjpeg", MJPEG_EXPORTS);
}

/// MP3 decoding via vendored minimp3 (CC0 public domain).
#[cfg(feature = "mp3")]
fn build_mp3() {
    use std::path::Path;

    let root = Path::new("native/minimp3");
    println!("cargo:rerun-if-changed=native/minimp3");
    println!("cargo:rerun-if-changed=native/mp3_shim.c");

    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let mut build = cc::Build::new();
    build.include(root).warnings(false);
    if msvc {
        for dir in build_support::msvc_system_includes() {
            build.include(dir);
        }
        build.define("_CRT_SECURE_NO_WARNINGS", None);
    }
    build.file(root.parent().unwrap().join("mp3_shim.c"));
    build_support::finish_codec(&mut build, "z_mp3", MP3_EXPORTS);
}

/// Opus decoding via vendored libopus (BSD-3), pure-C fixed-point build.
#[cfg(feature = "opus")]
fn build_opus() {
    use std::path::PathBuf;

    let root = std::path::Path::new("native/libopus");
    println!("cargo:rerun-if-changed=native/libopus");
    println!("cargo:rerun-if-changed=native/opus_shim.c");

    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let mut build = cc::Build::new();
    build
        .include(root.join("include"))
        .include(root.join("src"))
        .include(root.join("celt"))
        .include(root.join("silk"))
        .include(root.join("silk/fixed"))
        .define("FIXED_POINT", "1")
        .define("NONTHREADSAFE_PSEUDOSTACK", None)
        .define("OPUS_BUILD", None)
        .warnings(false);
    if msvc {
        for dir in build_support::msvc_system_includes() {
            build.include(dir);
        }
        build.define("_CRT_SECURE_NO_WARNINGS", None);
    }

    // Top-level sources of each subproject (no SIMD asm, no demos/tests).
    const EXCLUDE: &[&str] = &[
        "opus_demo.c",
        "repacketizer_demo.c",
        "opus_compare.c",
        "osce_compare.c",
        "qext_compare.c",
        "analysis.c",
        "mlp.c",
        "mlp_data.c",
    ];
    for dir in ["src", "celt", "silk", "silk/fixed"] {
        let path = root.join(dir);
        let mut files: Vec<PathBuf> = std::fs::read_dir(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension().and_then(|s| s.to_str()) == Some("c")
                    && !EXCLUDE.contains(&p.file_name().unwrap().to_str().unwrap_or(""))
            })
            .collect();
        files.sort();
        for file in files {
            build.file(file);
        }
    }
    build.file(root.parent().unwrap().join("opus_shim.c"));
    build_support::finish_codec(&mut build, "z_opus", OPUS_EXPORTS);
}

/// Vorbis decoding via vendored libvorbis (BSD-3), decoder only.
#[cfg(feature = "vorbis")]
fn build_vorbis() {
    let root = std::path::Path::new("native/libvorbis");
    println!("cargo:rerun-if-changed=native/libvorbis");
    println!("cargo:rerun-if-changed=native/vorbis_shim.c");

    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let mut build = cc::Build::new();
    build
        .include(root.join("include"))
        .include(root)
        .include(root.join("modes"))
        // libogg headers are already vendored for Theora; sources live in libogg/.
        .include(std::path::Path::new("native/theora/include"))
        .include(std::path::Path::new("native/libogg"))
        .warnings(false);
    if msvc {
        for dir in build_support::msvc_system_includes() {
            build.include(dir);
        }
        build.define("_CRT_SECURE_NO_WARNINGS", None);
    }

    const SOURCES: &[&str] = &[
        "mdct.c",
        "smallft.c",
        "block.c",
        "envelope.c",
        "window.c",
        "lsp.c",
        "lpc.c",
        "synthesis.c",
        "info.c",
        "psy.c",
        "floor1.c",
        "floor0.c",
        "res0.c",
        "mapping0.c",
        "registry.c",
        "codebook.c",
        "sharedbook.c",
        "lookup.c",
        "bitrate.c",
    ];
    for source in SOURCES {
        build.file(root.join(source));
    }
    let libogg = std::path::Path::new("native/libogg");
    build.file(libogg.join("bitwise.c"));
    build.file(libogg.join("framing.c"));
    build.file(root.parent().unwrap().join("vorbis_shim.c"));
    build_support::finish_codec(&mut build, "z_vorbis", VORBIS_EXPORTS);
}

/// FLAC decoding via vendored libFLAC (BSD-3), decoder only, no SIMD.
#[cfg(feature = "flac")]
fn build_flac() {
    let root = std::path::Path::new("native/libflac");
    println!("cargo:rerun-if-changed=native/libflac");
    println!("cargo:rerun-if-changed=native/flac_shim.c");

    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let mut build = cc::Build::new();
    build
        .include(root.join("include"))
        .include(root.join("include/share"))
        .include(root)
        .define("FLAC__NO_DLL", None)
        .define("PACKAGE_VERSION", "\"1.5.0\"")
        .define("FLAC__HAS_OGG", "0")
        .warnings(false);
    if msvc {
        for dir in build_support::msvc_system_includes() {
            build.include(dir);
        }
        build.define("_CRT_SECURE_NO_WARNINGS", None);
    }

    const SOURCES: &[&str] = &[
        "bitmath.c",
        "bitreader.c",
        "bitwriter.c",
        "cpu.c",
        "crc.c",
        "fixed.c",
        "float.c",
        "format.c",
        "lpc.c",
        "md5.c",
        "memory.c",
        "metadata_iterators.c",
        "metadata_object.c",
        "stream_decoder.c",
        "window.c",
        "win_utf8_io.c",
    ];
    for source in SOURCES {
        build.file(root.join(source));
    }
    build.file(root.parent().unwrap().join("flac_shim.c"));
    build_support::finish_codec(&mut build, "z_flac", FLAC_EXPORTS);
}
