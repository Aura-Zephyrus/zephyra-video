# zephyra-video

视频解码库，按 Chromium `media/` 的形状组织。原为 ZephyraUI 框架的子 crate，
现已独立成仓：**与本库没有代码依赖，也不依赖 ZephyraUI 的任何 crate**。

默认构建零依赖、零 `unsafe`、纯逻辑：解码配置、缓冲、帧布局、解码器选择、
H.264 位流解析与 Annex-B 转换、DXVA2 参数、AVS3 NAL 切分，全部可以在没有
GPU 的机器上单测。软件解码后端按需打开 feature：AV1 / VP8 / VP9 / HEVC / H.264 /
EVC / VVC，全部是 vendored C/C++ 库 + 薄 shim，默认构建一行 C 都不编。

## 为什么要独立

视频解码不是 UI 词汇。GPUI、egui、iced、Qt、Flutter 都不把解码放进框架——
那会让每个使用者被迫装上编解码库，而绝大多数 UI 根本不播视频。

## 与 Chromium 的对应

| 本库 | Chromium |
|---|---|
| `config::VideoDecoderConfig` | `media/base/video_decoder_config.h` |
| `buffer::DecoderBuffer` | `media/base/decoder_buffer.h` |
| `frame::VideoFrame` / `frame::VideoFrameLayout` | `media/base/video_frame.h` / `video_frame_layout.h` |
| `decoder::VideoDecoder` | `media/base/video_decoder.h` |
| `selector::DecoderSelector` | `media/filters/decoder_selector.cc` |
| `h264` | `media/parsers/h264_parser.{h,cc}` + `media/filters/h264_to_annex_b_bitstream_converter.cc` |
| `dxva2`（Windows feature） | `media/gpu/windows/d3d11_video_decoder_wrapper.cc` + `d3d_h264_accelerator.cc` |

硬件解码走 **DXVA2**（`ID3D11VideoDevice::CreateVideoDecoder` +
`ID3D11VideoContext::DecoderBeginFrame` / `SubmitDecoderBuffers` / `DecoderEndFrame`），
不是 Media Foundation：Chromium 的 Windows 硬件路径同样直接对 D3D11 视频设备建解码器，
输出是 GPU 纹理，从不落到系统内存。

## feature

| feature | 依赖 | 内容 |
|---|---|---|
| 无（默认） | 无 | 纯逻辑，零依赖：配置 / 缓冲 / 帧 / 选择器 / H.264 位流 / DXVA2 参数 / AVS3 NAL |
| `dxva2` | `windows`（仅 Windows） | DXVA2 硬件解码（`ID3D11VideoDecoder`） |
| `uavs3d` | `cc`（构建期） | vendored uavs3d C 解码器 + FFI（全平台，输出 8bit NV12） |
| `av1` | `cc`（构建期） | vendored dav1d + FFI（全平台，输出 8bit I420） |
| `vp9` | `cc`（构建期） | vendored libvpx + FFI（全平台，输出 8bit I420） |
| `vp8` | `cc`（构建期） | 同一份 libvpx，换 VP8 codec iface（输出 8bit I420） |
| `hevc` | `cc`（构建期） | vendored libde265（LGPL-3.0+，输出 8bit I420） |
| `h264` | `cc`（构建期） | vendored OpenH264（BSD-2-Clause，输出 8bit I420） |
| `evc` | `cc`（构建期） | vendored xevd（BSD-3-Clause，输出 8bit I420） |
| `vvc` | `cc`（构建期） | vendored vvdec（Clear BSD-3-Clause，输出 8bit I420） |
| `theora` | `cc`（构建期） | vendored libtheora + libogg headers（BSD-3，输出 8bit I420） |
| `mjpeg` | `cc`（构建期） | vendored libjpeg-turbo（BSD-3/IJG，输出 8bit I420） |
| `mp3` | `cc`（构建期） | 音频：vendored minimp3（CC0，输出 planar S16） |
| `opus` | `cc`（构建期） | 音频：vendored libopus（BSD-3，输出 planar S16） |
| `vorbis` | `cc`（构建期） | 音频：vendored libvorbis（BSD-3，输出 planar F32） |
| `flac` | `cc`（构建期） | 音频：vendored libFLAC（BSD-3，输出 planar S32） |
| `dynamic` | `libloading`（运行期） | vendored C 库编成共享库 + 运行时加载；缺库记入 `DynamicLoadReport`（详见下文） |
| `video` | 上方全部视频 feature | 一键开启所有软件视频解码器（不含 `dxva2`） |
| `audio` | 上方全部音频 feature | 一键开启所有音频解码器（PCM 解码器默认可用，无 feature） |
| `all-codecs` | `video` + `audio` | 一键开启所有编解码器（不含 `dynamic`） |

所有单独 feature 仍可任意组合，例如 `--features h264,mp3`。`video` 刻意不含
`dxva2`（Windows 专属硬件后端）；`all-codecs` 刻意不含 `dynamic`（链接模式），
两者都需要时显式加上。

新增 codec 的接线只需要两处：仓库根 `codecs.rs` 的一行声明，和 `Cargo.toml`
的一个 feature。Rust 侧的模块 / re-export / `load_all()` 与 build.rs 的导出
符号表 / 构建分发都从那张表派生；`tests/unit/codec_tests.rs` 的一致性测试会
把「表里有、Cargo.toml 没有」变成硬失败。若引入一套**新的** vendored C 库，
还需在 `build.rs` 加一个 `build_*` 函数（C 源码列表属于那套库，无法从声明派生）。

音频侧与视频侧对称：`AudioCodec` / `AudioDecoderConfig` / `AudioBuffer` / `AudioFrame` / `AudioDecoder`。
10bit（YUV420P10）目前由 AV1 / VP9 / HEVC / EVC / VVC 五个后端支持；H.264 与 AVS3 仍是 8bit。

`dxva2` 模块**默认就存在**——里面的 DPB 与图片参数是纯逻辑，没有 GPU 的机器
也要能单测它们；feature 只多出真正碰 D3D11 的那一半。

## 动态库模式（普通构建）

`dynamic` feature（或 CLI 设 `ZEPHYRA_LINK_MODE=dynamic`）把 vendored C 编解码器
链成共享库而不是静态塞进 exe：

- Windows：`link.exe /DLL` + `.def`，产出 `<name>.dll`（与 exe 同目录）
- Linux：`cc -shared`，产出 `lib<name>.so`，Rust 二进制带 `$ORIGIN` rpath
- macOS：`cc -dynamiclib`，产出 `lib<name>.dylib`，带 `@loader_path`

**运行时加载，缺库不影响启动**：`build.rs` 不再发 `rustc-link-lib=dylib`。
Rust 侧通过 [`libloading`] 在第一次使用某解码器时显式打开共享库并解析符号，
调用点写法与静态构建完全一致（`codec_api!` 宏在两种模式下生成同样的函数名）。
共享库在 exe 所在目录（或其上一级、`CWD`）中查找。

加载失败**不会**让进程启动失败，而是记录在 `DynamicLoadReport` 里，交给主程序决策：

```rust
use zephyra_video::{load_all, current_report, library_status};

let report = load_all();              // 主动加载所有已启用的共享库
if !report.is_ok() {
    for lib in report.failures() {
        eprintln!("{:?}: {}", lib.name, lib.result);
    }
    // 由主程序决定：继续运行部分功能 / 提示用户 / 退出
}
let _ = current_report();             // 当前快照
let _ = library_status("z_h264");     // 查询单个库
```

另外，每个解码器的 `initialize()` 也会检查对应库：若加载失败，返回带具体
原因的 `DecoderStatus`（`FailedToCreateDecoder` + "dynamic library ... unavailable"）。
未开 `dynamic` feature 时这些 API 不编译，构建仍是零新增依赖。

共享库产物文件名同时写进 `zephyra_dynamic_libs.txt`（exe 同目录）供打包脚本使用。

分发时必须带上这些共享库与 `native/*/` 下的许可证文件；LGPL 的 libde265
另有源码提供义务。

## 门禁

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test                                    # 默认：纯逻辑
cargo doc --no-deps

cargo test --features dxva2                   # Windows：DXVA2 硬件路径
cargo test --features uavs3d                  # vendored uavs3d
cargo test --features av1
cargo test --features vp9
cargo test --features vp8
cargo test --features hevc
cargo test --features h264
cargo test --features evc
cargo test --features vvc
cargo test --features theora
cargo test --features mjpeg
cargo test --features mp3,opus,vorbis,flac
cargo test --features video                              # 所有软件视频解码器
cargo test --features audio                              # 所有音频解码器
cargo test --features all-codecs                         # 全部编解码器
cargo test --features dynamic,h264,evc,vvc,theora,mjpeg   # 共享库链接模式
cargo test --all-features
```

本机实测（2026-11-21，Windows x64 / MSVC 14.51，14 个 codec feature）：

| 命令 | 结果 |
|---|---|
| `cargo test`（默认，零 feature） | 184 passed / 0 failed（183 单测 + 1 集成） |
| `cargo test --all-features` | 268 passed / 0 failed（230 单测 + 5 音频回放 + 12 视频回放 + 21 真实码流） |
| `cargo clippy --all-targets --all-features -- -D warnings` | 0 warning |
| `cargo tree -p zephyra-video --depth 1` | 只有 crate 自身一行 —— 默认构建零外部依赖 |

## 许可证

本库自己的 Rust 代码是 **MIT OR Apache-2.0**（见 `LICENSE-MIT` / `LICENSE-APACHE`）。

vendored 的 C/C++ 代码各有许可证，`native/` 下各目录保留了原始
`COPYING` / `LICENSE` / `PATENTS` 与源文件的版权头：

| 目录 | 许可证 |
|---|---|
| `native/uavs3d/` | BSD-3-Clause |
| `native/dav1d/` | BSD-2-Clause |
| `native/libvpx/` | BSD-3-Clause |
| `native/libde265/` | **LGPL-3.0-or-later**（分发义务比 BSD 重） |
| `native/openh264/` | BSD-2-Clause |
| `native/xevd/` | BSD-3-Clause |
| `native/vvdec/` | Clear BSD-3-Clause（含 `thirdparty/simde`，BSD-2-Clause） |
| `native/theora/` | BSD-3-Clause（Xiph.org；含 libogg 的 BSD-3 headers） |
| `native/libjpeg/` | BSD-3-Clause / IJG License（libjpeg-turbo 2.1.5.1） |
| `native/minimp3/` | CC0 1.0（minimp3） |
| `native/libopus/` | BSD-3-Clause（libopus） |
| `native/libvorbis/` + `native/libogg/` | BSD-3-Clause（Xiph.org） |
| `native/libflac/` | BSD-3-Clause（Xiph.org） |

再分发时必须一并带上这些许可证文件。

## 已知限制

- 音频解码不含 AAC：没有干净的 BSD/CC0 解码器，需引入 FFmpeg(LGPL) 或 libfaad2(GPL)。
- Opus/Vorbis 的 Ogg 封装需调用方解复用成一个个 packet 后送入 `AudioBuffer`；MP3/FLAC/PCM 可直接送整段字节流。
- 硬件解码只有 Windows DXVA2，没有 Media Foundation / VideoToolbox / VAAPI。
- `tests/real_streams.rs`（21 个测试）验证 9 种格式能出帧、格式尺寸正确、reset 后能再解码；`tests/playback.rs`（12 个测试）用 ffmpeg 预生成的 8 帧参考 YUV 做逐帧 Y 平面比对，并验证时间戳与帧数。
- HEVC 回放 fixture 带 B 帧；按 `DecoderBuffer = 一个访问单元` 的契约逐 AU 送入即可完整输出 8 帧（整段流塞进一个 buffer 会丢尾帧）。Theora 的输出与 ffmpeg 参考在像素值上有系统差异（后处理/色度范围），回放测试改为校验帧数、时基与"画面确实逐帧变化"，不做逐像素比对。
- 10bit 仅覆盖 AV1 / VP9 / HEVC / EVC / VVC；输出为 3 平面 `YUV420P10`（`P010LE` 布局已支持但暂未接入）。
- Theora 包在 Ogg 页里：调用方需先做 Ogg 解复用，每个 `DecoderBuffer` 传一个 Theora packet（含头部包）。
- MJPEG 每个 `DecoderBuffer` 必须是一个完整 JPEG 帧；输出固定 4:2:0（其它色度采样先上采样再 2×2 下采样）。
- OpenH264 只吃 Annex-B（`needs_bitstream_conversion()` 返回 `true`），调用方需先做 AVCC → Annex-B 转换。
- 不支持 AVS2：现有开源解码器（uavs2d / davs2）都是 GPL-2.0-or-later，与本仓的 BSD/MIT/Apache 许可不兼容。
