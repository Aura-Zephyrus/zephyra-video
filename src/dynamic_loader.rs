//! 运行时动态加载 vendored 编解码器共享库。
//!
//! 仅在 `dynamic` feature 打开时编译。与"加载时链接"不同：共享库在第一次
//! 使用时由 [`libloading`] 显式打开，缺库 / 缺符号不会让进程启动失败，而是
//! 记录到 [`DynamicLoadReport`] 中，由主程序决定如何处理。
//!
//! 每个解码模块用 `codec_api!` 宏声明它需要的符号：静态构建生成 `extern "C"`
//! 声明，动态构建生成从共享库解析的函数指针包装。调用点写法完全一致。

#![allow(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 单个共享库的加载失败原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadFailure {
    /// 共享库文件不存在（通常是没跟着 exe 一起发布）。
    NotFound {
        library: &'static str,
        path: PathBuf,
    },
    /// 文件存在但系统无法打开（依赖缺失、架构不匹配等）。
    OpenFailed {
        library: &'static str,
        path: PathBuf,
        message: String,
    },
    /// 共享库打开了，但导出符号缺失（版本不匹配）。
    SymbolMissing {
        library: &'static str,
        symbol: &'static str,
        message: String,
    },
}

impl std::fmt::Display for LoadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadFailure::NotFound { library, path } => {
                write!(f, "library '{library}' not found at {}", path.display())
            }
            LoadFailure::OpenFailed {
                library,
                path,
                message,
            } => write!(
                f,
                "cannot open library '{library}' at {}: {message}",
                path.display()
            ),
            LoadFailure::SymbolMissing {
                library,
                symbol,
                message,
            } => write!(
                f,
                "library '{library}' missing symbol '{symbol}': {message}"
            ),
        }
    }
}

impl std::error::Error for LoadFailure {}

/// 一个共享库的加载结果。
#[derive(Debug, Clone)]
pub struct LibraryReport {
    pub name: &'static str,
    pub path: PathBuf,
    pub result: Result<(), LoadFailure>,
}

/// 全部共享库的加载报告。
#[derive(Debug, Default, Clone)]
pub struct DynamicLoadReport {
    libraries: Vec<LibraryReport>,
}

impl DynamicLoadReport {
    /// 所有已尝试的库是否都加载成功。
    pub fn is_ok(&self) -> bool {
        self.libraries.iter().all(|l| l.result.is_ok())
    }

    pub fn libraries(&self) -> &[LibraryReport] {
        &self.libraries
    }

    /// 只返回失败的库。
    pub fn failures(&self) -> impl Iterator<Item = &LibraryReport> {
        self.libraries.iter().filter(|l| l.result.is_err())
    }

    pub fn get(&self, name: &str) -> Option<&LibraryReport> {
        self.libraries.iter().find(|l| l.name == name)
    }

    /// 人类可读的失败摘要，便于主程序打日志或弹窗。
    pub fn failure_summary(&self) -> String {
        self.failures()
            .map(|l| match &l.result {
                Err(e) => e.to_string(),
                Ok(()) => String::new(),
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

static REPORTS: Mutex<Vec<LibraryReport>> = Mutex::new(Vec::new());

fn record(report: LibraryReport) {
    if let Ok(mut reports) = REPORTS.lock() {
        if reports.iter().any(|r| r.name == report.name) {
            return;
        }
        reports.push(report);
    }
}

/// 查询某个库的当前加载状态；未尝试过则返回 `None`。
pub fn library_status(name: &'static str) -> Option<Result<(), LoadFailure>> {
    REPORTS.lock().ok().and_then(|reports| {
        reports
            .iter()
            .find(|r| r.name == name)
            .map(|r| r.result.clone())
    })
}

/// 返回当前已记录的加载报告快照。
pub fn current_report() -> DynamicLoadReport {
    DynamicLoadReport {
        libraries: REPORTS.lock().map(|r| r.clone()).unwrap_or_default(),
    }
}

/// 生成共享库路径并尝试打开。失败原因会记入全局报告。
pub fn load_library(name: &'static str) -> Result<libloading::Library, LoadFailure> {
    let exe = std::env::current_exe().map_err(|e| LoadFailure::OpenFailed {
        library: name,
        path: PathBuf::from("<current_exe>"),
        message: e.to_string(),
    })?;
    let filename = if cfg!(windows) {
        format!("{name}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{name}.dylib")
    } else {
        format!("lib{name}.so")
    };
    // exe 所在目录、其上一级（cargo test 的 deps/ 子目录）、当前工作目录都找一遍。
    let exe_dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let mut candidates = vec![exe_dir.to_path_buf()];
    if let Some(parent) = exe_dir.parent() {
        candidates.push(parent.to_path_buf());
    }
    candidates.push(std::env::current_dir().unwrap_or_default());
    let path = candidates
        .iter()
        .map(|d| d.join(&filename))
        .find(|p| p.exists())
        .unwrap_or_else(|| exe_dir.join(&filename));

    if !path.exists() {
        let failure = LoadFailure::NotFound {
            library: name,
            path: path.clone(),
        };
        record(LibraryReport {
            name,
            path,
            result: Err(failure.clone()),
        });
        return Err(failure);
    }
    match unsafe { libloading::Library::new(&path) } {
        Ok(lib) => {
            record(LibraryReport {
                name,
                path,
                result: Ok(()),
            });
            Ok(lib)
        }
        Err(e) => {
            let failure = LoadFailure::OpenFailed {
                library: name,
                path,
                message: e.to_string(),
            };
            record(LibraryReport {
                name,
                path: PathBuf::new(),
                result: Err(failure.clone()),
            });
            Err(failure)
        }
    }
}

/// 记录符号解析失败（库已打开，但版本不匹配）。
pub fn record_symbol_failure(library: &'static str, symbol: &'static str, message: String) {
    let failure = LoadFailure::SymbolMissing {
        library,
        symbol,
        message,
    };
    let path = library_status(library)
        .and_then(|r| r.ok())
        .map(|_| PathBuf::new())
        .unwrap_or_default();
    // 若该库已记录成功，改成失败报告，让主程序看到问题。
    if let Ok(mut reports) = REPORTS.lock() {
        if let Some(r) = reports.iter_mut().find(|r| r.name == library) {
            r.result = Err(failure);
            return;
        }
        reports.push(LibraryReport {
            name: library,
            path,
            result: Err(failure),
        });
    }
}

/// 从共享库解析一个函数符号。
///
/// # Safety
/// 调用方必须确保 `lib` 在返回的指针使用期间保持有效，且 `T` 是库中该符号的
/// 真实函数签名。
pub unsafe fn load_symbol<T: Copy>(
    lib: &libloading::Library,
    library: &'static str,
    symbol: &'static str,
) -> Result<T, LoadFailure> {
    lib.get::<T>(symbol.as_bytes()).map(|f| *f).map_err(|e| {
        let message = e.to_string();
        record_symbol_failure(library, symbol, message.clone());
        LoadFailure::SymbolMissing {
            library,
            symbol,
            message,
        }
    })
}

/// 主动加载所有已启用解码器的共享库，并返回完整报告。
///
/// 主程序可在启动时调用它，根据报告决定是继续运行、提示用户，还是退出。
pub fn load_all() -> DynamicLoadReport {
    #[cfg(feature = "uavs3d")]
    crate::uavs3d::ensure_loaded();
    #[cfg(feature = "av1")]
    crate::av1::ensure_loaded();
    #[cfg(feature = "vp8")]
    crate::vp8::ensure_loaded();
    #[cfg(feature = "vp9")]
    crate::vp9::ensure_loaded();
    #[cfg(feature = "hevc")]
    crate::hevc::ensure_loaded();
    #[cfg(feature = "h264")]
    crate::h264::decoder::ensure_loaded();
    #[cfg(feature = "evc")]
    crate::evc::ensure_loaded();
    #[cfg(feature = "vvc")]
    crate::vvc::ensure_loaded();
    #[cfg(feature = "theora")]
    crate::theora::ensure_loaded();
    #[cfg(feature = "mjpeg")]
    crate::mjpeg::ensure_loaded();
    #[cfg(feature = "mp3")]
    crate::audio::mp3::ensure_loaded();
    #[cfg(feature = "opus")]
    crate::audio::opus::ensure_loaded();
    #[cfg(feature = "vorbis")]
    crate::audio::vorbis::ensure_loaded();
    #[cfg(feature = "flac")]
    crate::audio::flac::ensure_loaded();
    current_report()
}

#[cfg(all(test, feature = "dynamic"))]
mod tests {
    use super::*;

    #[test]
    fn load_all_reports_each_enabled_library() {
        let report = load_all();
        // 每次构建启用的 codec 不同，但报告至少覆盖已启用后端使用的库。
        #[cfg(feature = "h264")]
        assert!(report.get("z_h264").is_some());
        #[cfg(feature = "av1")]
        assert!(report.get("z_dav1d").is_some());
        #[cfg(feature = "mp3")]
        assert!(report.get("z_mp3").is_some());
        // 只检查已启用 codec 对应的库；其他测试可能故意注入失败库到全局报告。
        for name in [
            #[cfg(feature = "uavs3d")]
            "uavs3d",
            #[cfg(feature = "av1")]
            "z_dav1d",
            #[cfg(feature = "vp8")]
            "z_vpx",
            #[cfg(feature = "vp9")]
            "z_vpx",
            #[cfg(feature = "hevc")]
            "z_hevc",
            #[cfg(feature = "h264")]
            "z_h264",
            #[cfg(feature = "evc")]
            "z_evc",
            #[cfg(feature = "vvc")]
            "z_vvc",
            #[cfg(feature = "theora")]
            "z_theora",
            #[cfg(feature = "mjpeg")]
            "z_mjpeg",
            #[cfg(feature = "mp3")]
            "z_mp3",
            #[cfg(feature = "opus")]
            "z_opus",
            #[cfg(feature = "vorbis")]
            "z_vorbis",
            #[cfg(feature = "flac")]
            "z_flac",
        ] {
            assert!(
                matches!(report.get(name).map(|l| &l.result), Some(Ok(()))),
                "enabled library {name} failed to load"
            );
        }
    }

    #[test]
    fn unknown_library_has_no_status() {
        assert!(library_status("__definitely_not_a_codec__").is_none());
    }

    #[test]
    fn missing_library_is_reported_not_fatal() {
        // 直接尝试加载一个不存在的库：应返回 Err 并写入报告，不能 panic。
        let err = load_library("__missing_codec_dll__").unwrap_err();
        assert!(matches!(err, LoadFailure::NotFound { .. }));
        match library_status("__missing_codec_dll__") {
            Some(Err(LoadFailure::NotFound { library, .. })) => {
                assert_eq!(library, "__missing_codec_dll__")
            }
            other => panic!("expected NotFound report, got {other:?}"),
        }
        assert!(!current_report().is_ok());
    }

    #[test]
    fn failure_display_is_actionable() {
        let e = LoadFailure::NotFound {
            library: "z_h264",
            path: PathBuf::from(r"C:\app\z_h264.dll"),
        };
        let msg = e.to_string();
        assert!(msg.contains("z_h264"));
        assert!(msg.contains("not found"));
    }
}
