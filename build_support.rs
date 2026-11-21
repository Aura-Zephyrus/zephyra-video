// build.rs 的共享 helper：仅在至少一个 codec feature 打开时编译（见 build_support_decl）。
// 从 build.rs 移到这里，是为了让「哪些 feature 会需要 cc」只有 codecs.rs 一个来源。

/// 结束一个 codec 的构建。静态模式维持原样；动态模式输出共享库。
#[allow(dead_code)]
pub fn finish_codec(build: &mut cc::Build, name: &str, exports: &[&str]) {
    if crate::dynamic_mode() {
        build.pic(true);
        let compiler = build.get_compiler().path().to_path_buf();
        let objects = build.compile_intermediates();
        // 先打成静态归档再链进共享库：只有被导出函数引用的 object 会被拉入。
        let archive = build.create_archive(&format!("{name}_static"), &objects);
        dynamic::link_shared(name, std::slice::from_ref(&archive), exports, &compiler);
    } else {
        let _ = exports;
        build.compile(name);
    }
}

/// 把 uavs3d 的 base + AVX2 两组 object 合成一个共享库。
#[cfg(feature = "uavs3d")]
pub fn finish_uavs3d(base: &mut cc::Build, avx2: Option<&mut cc::Build>) {
    if crate::dynamic_mode() {
        base.pic(true);
        let compiler = base.get_compiler().path().to_path_buf();
        let mut objects = base.compile_intermediates();
        if let Some(avx2) = avx2 {
            avx2.pic(true);
            objects.extend(avx2.compile_intermediates());
        }
        let archive = base.create_archive("uavs3d_static", &objects);
        dynamic::link_shared(
            "uavs3d",
            std::slice::from_ref(&archive),
            crate::UAVS3D_EXPORTS,
            &compiler,
        );
    } else {
        base.compile("uavs3d");
        if let Some(avx2) = avx2 {
            avx2.compile("uavs3d_avx2");
        }
    }
}

mod dynamic {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// OUT_DIR = <target>/<triple>/<profile>/build/<crate>-<hash>/out
    /// 往上三级就是 exe 所在目录。
    fn profile_dir() -> PathBuf {
        let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
        out.ancestors()
            .nth(3)
            .expect("OUT_DIR/../../..")
            .to_path_buf()
    }

    pub fn link_shared(name: &str, objects: &[PathBuf], exports: &[&str], compiler: &Path) {
        let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
        let profile = profile_dir();
        let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
        let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

        // 运行时加载：不再发 dylib 链接指令，缺库不会让 exe 启动失败；
        // Rust 侧通过 libloading 显式加载，失败写入 DynamicLoadReport。
        let _ = out;

        let mut cmd = if os == "windows" && env == "msvc" {
            let def = out.join(format!("{name}.def"));
            let body = exports
                .iter()
                .map(|e| format!("  {e}"))
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&def, format!("LIBRARY {name}\nEXPORTS\n{body}\n")).expect("write .def");
            // link.exe 不在 build script 的 PATH 上，但和 cl.exe 在同一个 VC bin 目录。
            let linker = compiler
                .parent()
                .expect("cl.exe 应有父目录")
                .join("link.exe");
            let mut c = Command::new(linker);
            c.arg("/DLL")
                .arg("/MACHINE:X64")
                .arg("/NOLOGO")
                .arg(format!(
                    "/OUT:{}",
                    profile.join(format!("{name}.dll")).display()
                ))
                .arg(format!(
                    "/IMPLIB:{}",
                    out.join(format!("{name}.lib")).display()
                ))
                .arg(format!("/DEF:{}", def.display()));
            for lib in msvc_lib_paths(compiler) {
                c.arg(lib);
            }
            c
        } else if os == "windows" {
            // MinGW：--export-all-symbols 暴露全部全局符号。
            let mut c = Command::new(compiler);
            c.arg("-shared")
                .arg("-Wl,--export-all-symbols")
                .arg("-o")
                .arg(profile.join(format!("{name}.dll")));
            c
        } else if os == "macos" {
            println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path");
            let mut c = Command::new(compiler);
            c.arg("-dynamiclib")
                .arg("-o")
                .arg(profile.join(format!("lib{name}.dylib")));
            c
        } else {
            println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
            let mut c = Command::new(compiler);
            c.arg("-shared")
                .arg("-o")
                .arg(profile.join(format!("lib{name}.so")));
            c.arg("-lm").arg("-lpthread");
            c
        };

        // 记录实际产物文件名，供 app 启动时做缺库自检（框架读这个文件）。
        let lib_name = if os == "windows" {
            format!("{name}.dll")
        } else if os == "macos" {
            format!("lib{name}.dylib")
        } else {
            format!("lib{name}.so")
        };
        let manifest = profile.join("zephyra_dynamic_libs.txt");
        let mut existing = std::fs::read_to_string(&manifest).unwrap_or_default();
        if !existing.lines().any(|l| l == lib_name) {
            if !existing.is_empty() && !existing.ends_with('\n') {
                existing.push('\n');
            }
            existing.push_str(&lib_name);
            existing.push('\n');
            let _ = std::fs::write(&manifest, existing);
        }

        for obj in objects {
            cmd.arg(obj);
        }
        let output = cmd
            .output()
            .unwrap_or_else(|e| panic!("link shared {name}: {e}"));
        assert!(
            output.status.success(),
            "link shared {name} failed:\nSTDOUT:\n{}\nSTDERR:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// build script 的 PATH/LIB 没继承 vcvars，需自己推导出 SDK 库目录。
    fn msvc_lib_paths(compiler: &Path) -> Vec<String> {
        let mut out = Vec::new();
        // cl.exe: ...\VC\Tools\MSVC\<ver>\bin\Hostx64\x64\cl.exe
        // ancestors: 0=cl.exe, 1=x64, 2=Hostx64, 3=bin, 4=<MSVC 版本目录>
        if let Some(msvc_ver) = compiler.ancestors().nth(4) {
            out.push(format!(
                "/LIBPATH:{}",
                msvc_ver.join("lib").join("x64").display()
            ));
        }
        let sdk_root = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Lib");
        if let Ok(rd) = std::fs::read_dir(&sdk_root) {
            let mut versions: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            versions.sort();
            if let Some(v) = versions.last() {
                out.push(format!("/LIBPATH:{}", v.join("um").join("x64").display()));
                out.push(format!("/LIBPATH:{}", v.join("ucrt").join("x64").display()));
            }
        }
        out
    }
}

#[cfg(any(
    feature = "uavs3d",
    feature = "av1",
    feature = "vp9",
    feature = "vp8",
    feature = "hevc",
    feature = "evc",
    feature = "h264",
    feature = "vvc",
    feature = "theora",
    feature = "mjpeg",
    feature = "mp3",
    feature = "opus",
    feature = "vorbis",
    feature = "flac"
))]
pub fn msvc_system_includes() -> Vec<std::path::PathBuf> {
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
    feature = "hevc",
    feature = "evc",
    feature = "h264",
    feature = "vvc",
    feature = "theora",
    feature = "mjpeg",
    feature = "mp3",
    feature = "opus",
    feature = "vorbis",
    feature = "flac"
))]
pub fn latest_subdir(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_dir())
        .max()
}

#[cfg(feature = "uavs3d")]
pub fn c_files(dir: std::path::PathBuf) -> Vec<std::path::PathBuf> {
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
pub fn gni_sources(gni: &std::path::Path, variable: &str) -> Vec<String> {
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
