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

10bit（YUV420P10）目前由 AV1 / VP9 / HEVC / EVC / VVC 五个后端支持；H.264 与 AVS3 仍是 8bit。

`dxva2` 模块**默认就存在**——里面的 DPB 与图片参数是纯逻辑，没有 GPU 的机器
也要能单测它们；feature 只多出真正碰 D3D11 的那一半。

## 动态库模式（普通构建）

`dynamic` feature（或 CLI 设 `ZEPHYRA_LINK_MODE=dynamic`）把 vendored C 编解码器
链成共享库而不是静态塞进 exe：

- Windows：`link.exe /DLL` + `.def`，产出 `<name>.dll`（与 exe 同目录）
- Linux：`cc -shared`，产出 `lib<name>.so`，Rust 二进制带 `$ORIGIN` rpath
- macOS：`cc -dynamiclib`，产出 `lib<name>.dylib`，带 `@loader_path`

Rust 侧 `extern "C"` 声明不用改——build.rs 发 `cargo:rustc-link-lib=dylib=<name>`
让它们直接动态链接。实现上先把源码打成静态归档，再用常规归档语义链进共享库，
只拉入被引用的 object，避免把 vendored 源码里未使用的内部符号暴露出来。

共享库产物文件名会写进 `zephyra_dynamic_libs.txt`（exe 同目录），
`zephyra_fx::link_check::ensure_dynamic_libs()` 据此在启动时做缺库自检。

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
cargo test --all-features
```

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

再分发时必须一并带上这些许可证文件。

## 已知限制

- 没有音频解码器。
- 硬件解码只有 Windows DXVA2，没有 Media Foundation / VideoToolbox / VAAPI。
- 软件解码器的 FFI 烟测不覆盖真实码流（本机无样片）。
- 10bit 仅覆盖 AV1 / VP9 / HEVC / EVC / VVC；输出为 3 平面 `YUV420P10`（`P010LE` 布局已支持但暂未接入）。
- Theora 包在 Ogg 页里：调用方需先做 Ogg 解复用，每个 `DecoderBuffer` 传一个 Theora packet（含头部包）。
- MJPEG 每个 `DecoderBuffer` 必须是一个完整 JPEG 帧；输出固定 4:2:0（其它色度采样先上采样再 2×2 下采样）。
- OpenH264 只吃 Annex-B（`needs_bitstream_conversion()` 返回 `true`），调用方需先做 AVCC → Annex-B 转换。
- 不支持 AVS2：现有开源解码器（uavs2d / davs2）都是 GPL-2.0-or-later，与本仓的 BSD/MIT/Apache 许可不兼容。
