//! 始终编译的宏定义。
//!
//! `codec_api!` / `check_lib!` 被所有解码模块使用，不能放在被 `dynamic` feature
//! 门控的模块里，否则只开某个解码器(feature)而不开 `dynamic` 时宏会消失。

/// 把可选的返回类型补全为 `()`，让宏能同时接受 `fn f();` 和 `fn f() -> T;`。
#[macro_export]
macro_rules! ret_ty {
    () => {
        ()
    };
    ($t:ty) => {
        $t
    };
}

/// 静态构建：生成与原来完全一样的 `extern "C"` 声明。
#[cfg(not(feature = "dynamic"))]
#[macro_export]
macro_rules! codec_api {
    ($lib:literal ; $( fn $sym:ident ( $($arg:ident : $ty:ty),* $(,)? ) $(-> $ret:ty)? ; )*) => {
        extern "C" {
            $( fn $sym($($arg: $ty),*) $(-> $ret)?; )*
        }
        /// 静态构建无需加载，保留同名入口以统一调用点。
        #[allow(dead_code)]
        pub fn ensure_loaded() {}
    };
}

/// 动态构建：生成运行时解析的函数指针包装，调用点与静态构建一致。
#[cfg(feature = "dynamic")]
#[macro_export]
macro_rules! codec_api {
    ($lib:literal ; $( fn $sym:ident ( $($arg:ident : $ty:ty),* $(,)? ) $(-> $ret:ty)? ; )*) => {
        #[allow(non_camel_case_types)]
        struct Api {
            _lib: libloading::Library,
            $( $sym: unsafe extern "C" fn($($ty),*) -> $crate::ret_ty!($($ret)?), )*
        }

        static API: ::std::sync::OnceLock<Result<Api, $crate::dynamic_loader::LoadFailure>> =
            ::std::sync::OnceLock::new();

        fn api() -> Option<&'static Api> {
            API.get_or_init(|| {
                let lib = $crate::dynamic_loader::load_library($lib)?;
                Ok(Api {
                    // 先解析符号（借用 lib），最后再把 lib 移进结构体，避免 use-after-move。
                    $( $sym: unsafe {
                        $crate::dynamic_loader::load_symbol(&lib, $lib, stringify!($sym))?
                    }, )*
                    _lib: lib,
                })
            })
            .as_ref()
            .ok()
        }

        /// 强制加载本模块使用的共享库（供 `load_all` 使用）。
        pub fn ensure_loaded() {
            let _ = api();
        }

        $(
            #[allow(non_snake_case, private_interfaces, clippy::missing_safety_doc)]
            /// # Safety
            /// 与上游 C API 相同：指针参数必须在调用期间有效。
            pub unsafe fn $sym($($arg: $ty),*) -> $crate::ret_ty!($($ret)?) {
                match api() {
                    Some(a) => (a.$sym)($($arg),*),
                    // 加载失败时返回安全的哨兵值；真正的错误由 initialize() 通过
                    // check_lib! 报告给主程序。
                    None => Default::default(),
                }
            }
        )*
    };
}

/// 静态构建：`initialize()` 中不需要检查共享库。
#[cfg(not(feature = "dynamic"))]
#[macro_export]
macro_rules! check_lib {
    ($lib:literal) => {
        None
    };
}

/// 动态构建：把库加载失败转成 `DecoderStatus`，由解码器 `initialize()` 返回。
///
/// 必须先触发本模块的加载：`api()` 直到第一次真正调用解码函数才执行，而
/// `check_lib!` 在 `initialize()` 开头运行 —— 若不主动加载，`library_status`
/// 此刻还是空的，缺库 / 缺符号检查形同虚设，错误会被 `Default::default()`
/// 静默吞成"成功但无帧"。
#[cfg(feature = "dynamic")]
#[macro_export]
macro_rules! check_lib {
    ($lib:literal) => {{
        // 由 codec_api! 在当前模块生成；幂等（OnceLock），多次调用无副作用。
        ensure_loaded();
        $crate::dynamic_loader::library_status($lib)
            .and_then(|r| r.err())
            .map(|e| {
                $crate::DecoderStatus::with_message(
                    $crate::DecoderStatusCode::FailedToCreateDecoder,
                    format!("dynamic library '{}' unavailable: {}", $lib, e),
                )
            })
    }};
}
