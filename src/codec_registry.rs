// 由 `codecs.rs` 的单一声明表派生 Rust 侧的 codec 接线。
//
// 本文件被 `lib.rs` 用 `include!` 放进 crate 根，因此这里生成的 `pub mod`
// 与 `pub use` 就是 crate 根的模块和 re-export；child module（`audio` /
// `dynamic_loader`）在 `lib.rs` 中排在本文件之后，可以直接用这里定义的宏。
//
// `VideoCodec` 的判别值钉死 Chromium、不能由表生成；但「哪些后端接线到 crate 根」
// 这件事只有 `codecs.rs` 一个来源。

include!("../codecs.rs");

// `uavs3d` / `h264` 的模块本身是纯逻辑、默认构建就要存在（NAL 切分、位流解析），
// 只有解码器类型才是 feature 门控的 —— 这两个模块声明留给 lib.rs 显式写。
macro_rules! codec_root_module {
    // 捕获的 `literal` 片段不能和具体字面量再匹配，但 `ident` 可以。
    (video $feature:literal uavs3d) => {};
    (video $feature:literal h264) => {};
    (video $feature:literal $module:ident) => {
        #[cfg(feature = $feature)]
        #[allow(unsafe_code)]
        pub mod $module;
    };
    (audio $feature:literal $module:ident) => {};
}

macro_rules! codec_root_reexport {
    (video $feature:literal $module:ident $decoder:ident) => {
        #[cfg(feature = $feature)]
        pub use $module::$decoder;
    };
    // 音频解码器同时从 crate 根 re-export（历史 API，主仓 media 按这个路径引）。
    (audio $feature:literal $module:ident $decoder:ident) => {
        #[cfg(feature = $feature)]
        pub use audio::$module::$decoder;
    };
}

macro_rules! codec_audio_module {
    (audio $feature:literal $module:ident $decoder:ident) => {
        #[cfg(feature = $feature)]
        #[allow(unsafe_code)]
        pub mod $module;
        #[cfg(feature = $feature)]
        pub use $module::$decoder;
    };
    (video $feature:literal $module:ident $decoder:ident) => {};
}

/// `load_all()` 的加载入口。`h264` 的 `codec_api!` 住在 `decoder` 子模块里。
#[allow(unused_macros)] // 默认构建没有 dynamic feature，load_all 不编译。
macro_rules! codec_load_call {
    (video root $feature:literal $module:ident) => {
        #[cfg(feature = $feature)]
        crate::$module::ensure_loaded();
    };
    (video nested $feature:literal $module:ident) => {
        #[cfg(feature = $feature)]
        crate::$module::decoder::ensure_loaded();
    };
    (audio root $feature:literal $module:ident) => {
        #[cfg(feature = $feature)]
        crate::audio::$module::ensure_loaded();
    };
    (audio nested $feature:literal $module:ident) => {
        #[cfg(feature = $feature)]
        crate::audio::$module::decoder::ensure_loaded();
    };
}

macro_rules! emit_root_codec_items {
    ($( $feature:literal, $module:ident, $decoder:ident, $build:ident, $group:ident, $loader:ident );* $(;)?) => {
        $( codec_root_module!($group $feature $module); )*
        $( codec_root_reexport!($group $feature $module $decoder); )*

        /// 单一声明表在编译期的投影：`(feature, 模块名, 解码器类型名)`。
        /// 供一致性测试核对「表里的 feature 在 Cargo.toml 里真的存在」。
        #[allow(dead_code)] // 只在 codec 一致性测试里读。
        pub(crate) const CODEC_MANIFEST: &[(&str, &str, &str)] = &[
            $( ($feature, stringify!($module), stringify!($decoder)), )*
        ];
    };
}

macro_rules! emit_audio_codec_items {
    ($( $feature:literal, $module:ident, $decoder:ident, $build:ident, $group:ident, $loader:ident );* $(;)?) => {
        $( codec_audio_module!($group $feature $module $decoder); )*
    };
}

#[allow(unused_macros)]
macro_rules! emit_load_all_calls {
    ($( $feature:literal, $module:ident, $decoder:ident, $build:ident, $group:ident, $loader:ident );* $(;)?) => {
        $( codec_load_call!($group $loader $feature $module); )*
    };
}

zephyra_codecs!(emit_root_codec_items);
