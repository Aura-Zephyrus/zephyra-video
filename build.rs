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

fn main() {
    #[cfg(feature = "uavs3d")]
    build_uavs3d();
    #[cfg(feature = "av1")]
    build_av1();
    #[cfg(any(feature = "vp9", feature = "vp8"))]
    build_vp9();
    #[cfg(feature = "hevc")]
    build_hevc();
}

#[cfg(feature = "uavs3d")]
fn build_uavs3d() {
    use std::path::Path;

    let src = Path::new("native/uavs3d/source");
    println!("cargo:rerun-if-changed=native/uavs3d");

    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let system_includes = if msvc {
        msvc_system_includes()
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
        for file in c_files(src.join(dir)) {
            base.file(file);
        }
    }

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if arch == "x86_64" || arch == "x86" {
        for file in c_files(src.join("decore/sse")) {
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
        for file in c_files(src.join("decore/avx2")) {
            avx2.file(file);
        }
        if msvc {
            avx2.flag("/arch:AVX2");
        } else {
            avx2.flag_if_supported("-mavx2");
        }
        base.compile("uavs3d");
        avx2.compile("uavs3d_avx2");
    } else {
        // ARM / 其它平台：CMakeLists 用的是 .S 汇编，这里不编，
        // 退到 decore 的纯 C 路径（uavs3d 仍然能解码，只是慢）。
        base.compile("uavs3d");
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
#[cfg(any(
    feature = "uavs3d",
    feature = "av1",
    feature = "vp9",
    feature = "vp8",
    feature = "hevc"
))]
fn msvc_system_includes() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;

    let mut out = Vec::new();

    // cl.exe 在 <MSVC>\bin\HostX64\x64\cl.exe：往上四级是 <MSVC>。
    let cl = cc::Build::new().get_compiler().path().to_path_buf();
    let mut root = cl;
    for _ in 0..4 {
        root.pop();
    }
    let crt = root.join("include");
    if crt.is_dir() {
        out.push(crt);
    }

    let pf = std::env::var("ProgramFiles(x86)")
        .or_else(|_| std::env::var("ProgramFiles"))
        .unwrap_or_else(|_| r"C:\Program Files (x86)".into());
    let kits = PathBuf::from(pf).join("Windows Kits").join("10");
    if let Some(version) = latest_subdir(&kits.join("Include")) {
        for sub in ["ucrt", "um", "shared", "winrt", "cppwinrt"] {
            let dir = version.join(sub);
            if dir.is_dir() {
                out.push(dir);
            }
        }
    }
    out
}

#[cfg(any(
    feature = "uavs3d",
    feature = "av1",
    feature = "vp9",
    feature = "vp8",
    feature = "hevc"
))]
fn latest_subdir(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_dir())
        .max()
}

#[cfg(feature = "uavs3d")]
fn c_files(dir: std::path::PathBuf) -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("c"))
        .collect();
    files.sort();
    files
}

/// 从 vendored 的 Chromium `.gni` 列表里读出一个变量的所有 `.c` 源文件。
/// 这两个列表是 Chromium 为各平台生成的精确源码清单 —— 照它编就不必自己
/// 维护一份会漂移的文件表。
#[cfg(any(feature = "av1", feature = "vp9", feature = "vp8"))]
fn gni_sources(gni: &std::path::Path, variable: &str) -> Vec<String> {
    let text = std::fs::read_to_string(gni)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", gni.display()));
    // 行首锚定：否则 `template_sources` 会命中 `x86_template_sources`（空列表）。
    // gni 里既有 `name = [` 也有 `name =\n    [`，所以先找行首的 `name =`，
    // 再找随后的第一个 `[`。
    let needle = format!("\n{variable} =");
    let start = text
        .find(&needle)
        .unwrap_or_else(|| panic!("{variable} not found in {}", gni.display()))
        + 1;
    let rest = &text[start..];
    let bracket = rest.find('[').unwrap_or(rest.len());
    let rest = &rest[bracket..];
    let end = rest.find(']').unwrap_or(rest.len());
    rest[..end]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(|entry| entry.to_string())
        .filter(|entry| entry.ends_with(".c"))
        .collect()
}

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
        for dir in msvc_system_includes() {
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
    for source in gni_sources(&root.join("dav1d_generated.gni"), "c_sources") {
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
    let templates = gni_sources(&root.join("dav1d_generated.gni"), "template_sources");
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
    build.compile("z_dav1d");
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
        for dir in msvc_system_includes() {
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
    let mut all_sources = gni_sources(&gni, variable);
    // Chromium 的 win/x64 rtcd 头**无条件**映射到 *_sse2 / *_ssse3 等符号，
    // 即使 HAVE_X86_ASM=0 也要把对应的 intrinsic C 文件编进来（它们是 .c，
    // 不是需要 nasm 的 .asm）。
    let simd_suffix = variable.strip_prefix("libvpx_srcs_").unwrap_or("x86_64");
    for group in ["mmx", "sse2", "ssse3", "sse4_1"] {
        all_sources.extend(gni_sources(
            &gni,
            &format!("libvpx_srcs_{simd_suffix}_{group}"),
        ));
    }
    for source in all_sources {
        let relative = source.strip_prefix(PREFIX).unwrap_or(&source);
        build.file(root.join(relative));
    }
    build.file(root.parent().unwrap().join("libvpx_shim.c"));
    build.compile("z_vpx");

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
        for dir in msvc_system_includes() {
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
    build.compile("z_hevc");

    if os != "windows" {
        println!("cargo:rustc-link-lib=pthread");
    }
}
