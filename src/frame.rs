//! 解码出来的帧：像素格式、平面布局与数据。
//!
//! 对应 Chromium 的 `media/base/video_frame.h` + `video_frame_layout.h`。
//! 刻意只保留**渲染需要**的那部分：Chromium 的 `VideoFrame` 还带
//! `gfx::NativePixmapHandle`、色彩空间变换矩阵、参考帧 DPB 索引等一堆
//! 本仓用不上的状态；搬进来只会让「这个字段没人读」变成常态。
//!
//! # 平面模型
//!
//! 与 Chromium 一致，**半平面格式（NV12）也是两个平面**：Y 一个、交错的 UV 一个。
//! 把 NV12 当成「一个大平面」会让「平面数」这件事在两种格式之间含义漂移，
//! 而平面数是纹理上传路径要按它分支的。

use crate::geometry::{Rect, Size};
use bytes::Bytes;
use std::any::Any;
use std::sync::Arc;

/// 平台原生资源的**持有者**。
///
/// [`VideoPlane::Native`] 里的 `handle` 是**裸指针**（本 crate 的 Windows
/// 后端传的是 `ID3D11Texture2D *`）。裸指针不持有任何东西 —— 资源一旦
/// 被释放，句柄就悬空了，而悬空的句柄不会立刻崩，它会在几帧之后画出
/// 垃圾画面。
///
/// 把持有者放进帧里，「帧活得比解码器久」这件事才在**类型**上成立：
/// 消费方拿着帧的时候，底层资源一定还在。
///
/// 之所以用 `dyn Any` 而不是具体类型：帧模型是**平台无关**的，
/// `ID3D11Texture2D` 不该出现在这一层。
#[derive(Clone)]
pub struct NativeResourceOwner {
    inner: Arc<dyn Any + Send + Sync>,
}

impl NativeResourceOwner {
    /// 用一个具体类型的共享所有权造一个持有者。
    pub fn new<T: Any + Send + Sync>(resource: Arc<T>) -> Self {
        Self {
            inner: resource as Arc<dyn Any + Send + Sync>,
        }
    }

    /// 共享所有权的引用计数。
    ///
    /// 只有解码器自己持有（计数为 1）时，资源才可以安全地回收。
    pub fn strong_count(&self) -> usize {
        Arc::strong_count(&self.inner)
    }

    /// 把持有者向下转型为具体平台资源类型。
    ///
    /// 视频解码器（如 DXVA2）在原生平面里放的是 `ID3D11Texture2D*` 的共享
    /// 持有者；上层渲染后端需要按真实类型取出该句柄才能做零拷贝导入。
    /// 类型不匹配时返回 `None`。
    pub fn downcast_ref<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.inner.downcast_ref::<T>()
    }

    /// 取回底层 `Arc`，用于跨 API 共享（例如 D3D11 → D3D12 共享句柄）。
    pub fn downcast_arc<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        Arc::clone(&self.inner).downcast::<T>().ok()
    }
}

impl std::fmt::Debug for NativeResourceOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NativeResourceOwner(strong_count={})",
            self.strong_count()
        )
    }
}

impl PartialEq for NativeResourceOwner {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for NativeResourceOwner {}

/// 像素格式。判别值与 Chromium `VideoPixelFormat` 对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum VideoPixelFormat {
    Unknown = 0,
    /// 12bpp YUV 平面，4:2:0（又名 YU12）。
    I420 = 1,
    /// 12bpp YVU 平面，4:2:0。
    YV12 = 2,
    /// 16bpp YUV 平面，4:2:2。
    I422 = 3,
    /// 20bpp YUVA 平面，4:2:0。
    I420A = 4,
    /// 24bpp YUV 平面，无下采样。
    I444 = 5,
    /// 12bpp：Y 平面 + 2×2 交错的 UV 平面。
    NV12 = 6,
    /// 12bpp：Y 平面 + 2×2 交错的 VU 平面。
    NV21 = 7,
    /// 16bpp 交错 UYVY。
    UYVY = 8,
    /// 16bpp 交错 YUY2。
    YUY2 = 9,
    /// 32bpp BGRA。
    ARGB = 10,
    /// 24bpp BGRX。
    XRGB = 11,
    /// 24bpp BGR。
    RGB24 = 12,
    /// MJPEG 压缩数据。
    MJPEG = 14,
    /// 15bpp YUV 平面 4:2:0（I010）。
    YUV420P10 = 17,
    /// 20bpp YUV 平面 4:2:2。
    YUV422P10 = 19,
    /// 30bpp YUV 平面 4:4:4。
    YUV444P10 = 21,
    /// 18bpp YUV 平面 4:2:0。
    YUV420P12 = 22,
    /// 24bpp YUV 平面 4:2:2。
    YUV422P12 = 23,
    /// 36bpp YUV 平面 4:4:4。
    YUV444P12 = 24,
    /// 单个 16bpp 平面。
    Y16 = 26,
    /// 32bpp RGBA。
    ABGR = 27,
    /// 24bpp RGBX。
    XBGR = 28,
    /// 15bpp YUV 平面 4:2:0，10 bit，little-endian。
    P010LE = 29,
    /// 32bpp BGRX，10 bit。
    XR30 = 30,
    /// 32bpp RGBX，10 bit。
    XB30 = 31,
    /// 32bpp BGRA。
    BGRA = 32,
    /// 64bpp 半浮点 RGBA。
    RGBAF16 = 33,
    /// 24bpp YUVA 平面 4:2:2。
    I422A = 34,
    /// 32bpp YUVA 平面 4:4:4。
    I444A = 35,
    /// YUVA 平面，10 bit，4:2:0。
    YUV420AP10 = 36,
    /// YUVA 平面，10 bit，4:2:2。
    YUV422AP10 = 37,
    /// YUVA 平面，10 bit，4:4:4。
    YUV444AP10 = 38,
    /// 20bpp YUVA 平面 4:2:0。
    NV12A = 39,
    /// 16bpp YUV 平面 4:2:2，交错 UV。
    NV16 = 40,
    /// 24bpp YUV 平面 4:4:4，交错 UV。
    NV24 = 41,
    /// 30bpp YUV 平面 4:2:0，10 bit。
    P210LE = 42,
    /// 36bpp YUV 平面 4:4:4，10 bit。
    P410LE = 43,
}

impl VideoPixelFormat {
    /// 该格式有几个颜色平面。
    ///
    /// 对应 Chromium `VideoFrameLayout::NumPlanes()`。
    pub const fn num_planes(self) -> usize {
        match self {
            VideoPixelFormat::Unknown => 0,
            VideoPixelFormat::UYVY
            | VideoPixelFormat::YUY2
            | VideoPixelFormat::ARGB
            | VideoPixelFormat::BGRA
            | VideoPixelFormat::XRGB
            | VideoPixelFormat::RGB24
            | VideoPixelFormat::MJPEG
            | VideoPixelFormat::Y16
            | VideoPixelFormat::ABGR
            | VideoPixelFormat::XBGR
            | VideoPixelFormat::XR30
            | VideoPixelFormat::XB30
            | VideoPixelFormat::RGBAF16 => 1,
            VideoPixelFormat::NV12
            | VideoPixelFormat::NV16
            | VideoPixelFormat::NV21
            | VideoPixelFormat::NV24
            | VideoPixelFormat::P010LE
            | VideoPixelFormat::P210LE
            | VideoPixelFormat::P410LE => 2,
            VideoPixelFormat::I420
            | VideoPixelFormat::YV12
            | VideoPixelFormat::I422
            | VideoPixelFormat::I444
            | VideoPixelFormat::NV12A
            | VideoPixelFormat::YUV420P10
            | VideoPixelFormat::YUV422P10
            | VideoPixelFormat::YUV444P10
            | VideoPixelFormat::YUV420P12
            | VideoPixelFormat::YUV422P12
            | VideoPixelFormat::YUV444P12 => 3,
            VideoPixelFormat::I420A
            | VideoPixelFormat::I422A
            | VideoPixelFormat::I444A
            | VideoPixelFormat::YUV420AP10
            | VideoPixelFormat::YUV422AP10
            | VideoPixelFormat::YUV444AP10 => 4,
        }
    }

    /// 该格式是否为 **4:2:0**（本仓唯一走通的上传路径）。
    pub const fn is_420(self) -> bool {
        matches!(
            self,
            VideoPixelFormat::I420
                | VideoPixelFormat::YV12
                | VideoPixelFormat::NV12
                | VideoPixelFormat::NV21
                | VideoPixelFormat::NV12A
        )
    }

    /// 每像素的字节数（半平面与交错格式按整帧算，这里只给「每像素」的平均值）。
    ///
    /// 仅用于日志与预算估算；精确的字节数要看 [`VideoFrameLayout`]。
    pub const fn bytes_per_pixel(self) -> f32 {
        match self {
            VideoPixelFormat::I420
            | VideoPixelFormat::YV12
            | VideoPixelFormat::NV12
            | VideoPixelFormat::NV21 => 1.5,
            VideoPixelFormat::I422 | VideoPixelFormat::NV16 => 2.0,
            VideoPixelFormat::I444 | VideoPixelFormat::NV24 => 3.0,
            VideoPixelFormat::ARGB
            | VideoPixelFormat::BGRA
            | VideoPixelFormat::ABGR
            | VideoPixelFormat::XRGB
            | VideoPixelFormat::XBGR
            | VideoPixelFormat::XR30
            | VideoPixelFormat::XB30 => 4.0,
            VideoPixelFormat::RGB24 => 3.0,
            // 10/12-bit 4:2:0：每像素 1.5 个 16-bit 采样 = 3 字节。
            VideoPixelFormat::YUV420P10
            | VideoPixelFormat::YUV420P12
            | VideoPixelFormat::P010LE
            | VideoPixelFormat::YUV420AP10 => 3.0,
            VideoPixelFormat::YUV422P10
            | VideoPixelFormat::YUV422P12
            | VideoPixelFormat::YUV422AP10
            | VideoPixelFormat::P210LE => 4.0,
            VideoPixelFormat::YUV444P10
            | VideoPixelFormat::YUV444P12
            | VideoPixelFormat::YUV444AP10
            | VideoPixelFormat::P410LE => 6.0,
            _ => 0.0,
        }
    }
}

/// 一个颜色平面在缓冲区里的位置。
///
/// 对应 Chromium `media/base/color_plane_layout.h` 的 `ColorPlaneLayout`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ColorPlaneLayout {
    /// 平面起始偏移（相对整帧缓冲区）。
    pub offset: usize,
    /// 每行字节数。
    pub stride: usize,
}

/// 一帧的**几何**布局：格式、尺寸、每个平面的偏移与步长。
///
/// 对应 Chromium `VideoFrameLayout`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrameLayout {
    format: VideoPixelFormat,
    coded_size: Size,
    planes: Vec<ColorPlaneLayout>,
}

impl VideoFrameLayout {
    /// 按格式与尺寸推导布局（CPU 帧用）。
    ///
    /// 对应 Chromium `VideoFrameLayout::Create()`，但 Chromium 那一版把
    /// stride / offset 全部留 0（「无法从参数可靠推断」）。本 crate 推导出来，
    /// 因为本仓的纹理上传路径**需要** stride —— 而推导规则是确定的
    /// （H.264 的 4:2:0 帧宽高必是偶数，色度平面宽高各减半）。
    pub fn create(format: VideoPixelFormat, coded_size: Size) -> Option<Self> {
        let width = coded_size.width as usize;
        let height = coded_size.height as usize;
        if format == VideoPixelFormat::Unknown || width == 0 || height == 0 {
            return None;
        }
        let chroma_width = width.div_ceil(2);
        let chroma_height = height.div_ceil(2);
        let planes = match format {
            VideoPixelFormat::I420 => vec![
                ColorPlaneLayout {
                    offset: 0,
                    stride: width,
                },
                ColorPlaneLayout {
                    offset: width * height,
                    stride: chroma_width,
                },
                ColorPlaneLayout {
                    offset: width * height + chroma_width * chroma_height,
                    stride: chroma_width,
                },
            ],
            VideoPixelFormat::YV12 => vec![
                ColorPlaneLayout {
                    offset: 0,
                    stride: width,
                },
                ColorPlaneLayout {
                    offset: width * height,
                    stride: chroma_width,
                },
                ColorPlaneLayout {
                    offset: width * height + chroma_width * chroma_height,
                    stride: chroma_width,
                },
            ],
            VideoPixelFormat::NV12 | VideoPixelFormat::NV21 => vec![
                ColorPlaneLayout {
                    offset: 0,
                    stride: width,
                },
                ColorPlaneLayout {
                    offset: width * height,
                    stride: chroma_width * 2,
                },
            ],
            VideoPixelFormat::I444 => vec![
                ColorPlaneLayout {
                    offset: 0,
                    stride: width,
                },
                ColorPlaneLayout {
                    offset: width * height,
                    stride: width,
                },
                ColorPlaneLayout {
                    offset: 2 * width * height,
                    stride: width,
                },
            ],
            // 10-bit 4:2:0 planar：每个采样 2 字节。
            VideoPixelFormat::YUV420P10 | VideoPixelFormat::YUV420P12 => vec![
                ColorPlaneLayout {
                    offset: 0,
                    stride: width * 2,
                },
                ColorPlaneLayout {
                    offset: width * height * 2,
                    stride: chroma_width * 2,
                },
                ColorPlaneLayout {
                    offset: (width * height + chroma_width * chroma_height) * 2,
                    stride: chroma_width * 2,
                },
            ],
            // 10-bit 4:2:0 semi-planar：Y + 交错 UV。
            VideoPixelFormat::P010LE => vec![
                ColorPlaneLayout {
                    offset: 0,
                    stride: width * 2,
                },
                ColorPlaneLayout {
                    offset: width * height * 2,
                    stride: chroma_width * 4,
                },
            ],
            _ => {
                // 其余格式的 stride 规则各不相同，先不给它们编布局 ——
                // 「猜一个错的 stride」比「没有布局」糟得多。
                return None;
            }
        };
        Some(Self {
            format,
            coded_size,
            planes,
        })
    }

    pub fn format(&self) -> VideoPixelFormat {
        self.format
    }

    pub fn coded_size(&self) -> Size {
        self.coded_size
    }

    pub fn planes(&self) -> &[ColorPlaneLayout] {
        &self.planes
    }

    pub fn num_planes(&self) -> usize {
        self.planes.len()
    }

    /// 第 `index` 个平面有多少**行**。
    ///
    /// 色度平面的行数是亮度的一半（4:2:0），而交错平面的**宽度**补回来
    /// （NV12 的 UV 一行有两个字节，但行数仍然减半）。把这两件事分开算，
    /// 是为了「缓冲区至少要多大」与「平面数据够不够」两处判定用**同一条**
    /// 规则 —— 否则「够不够」会算错（4:2:0 的第三个平面按亮度行数判，
    /// 会把合法的帧判成越界）。
    pub fn plane_rows(&self, index: usize) -> usize {
        let height = self.coded_size.height as usize;
        match self.format {
            VideoPixelFormat::I420
            | VideoPixelFormat::YV12
            | VideoPixelFormat::YUV420P10
            | VideoPixelFormat::YUV420P12 => {
                if index == 0 {
                    height
                } else {
                    height.div_ceil(2)
                }
            }
            VideoPixelFormat::NV12 | VideoPixelFormat::NV21 | VideoPixelFormat::P010LE => {
                if index == 0 {
                    height
                } else {
                    height.div_ceil(2)
                }
            }
            _ => height,
        }
    }

    /// 整帧缓冲区至少要多大（最后一个平面的末尾）。
    pub fn required_buffer_size(&self) -> usize {
        self.planes
            .iter()
            .enumerate()
            .map(|(i, plane)| plane.offset + plane.stride * self.plane_rows(i))
            .max()
            .unwrap_or(0)
    }
}

/// 一个颜色平面的数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoPlane {
    /// 系统内存里的像素。
    ///
    /// 用 `Arc<[u8]>` 而不是 `Vec<u8>`：解码器交出平面后，帧在压缩缓冲
    /// 复用 / 解码器销毁之后仍然有效；`Vec<u8> -> Arc<[u8]>` 转移的是
    /// 原来的分配，不复制一个字节，渲染侧也能把同一个 `Arc` 直接借给契约层。
    Cpu { data: Arc<[u8]> },
    /// 平台原生资源。`handle` 是**不透明**的，含义由生产方与消费方约定。
    ///
    /// 本 crate 的 Windows 后端用它携带 `ID3D11Texture2D` 指针 ——
    /// 硬件解码的输出**从不**落到系统内存（Chromium 同理，见
    /// `media/gpu/windows/d3d11_video_decoder_wrapper.cc`）。
    Native { handle: u64 },
}

impl VideoPlane {
    /// 是否是系统内存里的像素。
    pub fn is_cpu(&self) -> bool {
        matches!(self, VideoPlane::Cpu { .. })
    }

    /// 系统内存数据（不是 CPU 平面则返回 `None`）。
    pub fn cpu_data(&self) -> Option<&[u8]> {
        match self {
            VideoPlane::Cpu { data } => Some(data.as_ref()),
            VideoPlane::Native { .. } => None,
        }
    }

    /// 共享所有权的数据（不是 CPU 平面则返回 `None`）。
    ///
    /// 渲染后端做纹理上传时可以 clone 这个 `Arc`（只加引用计数），
    /// 不必再把平面拷一遍。
    pub fn cpu_arc(&self) -> Option<&Arc<[u8]>> {
        match self {
            VideoPlane::Cpu { data } => Some(data),
            VideoPlane::Native { .. } => None,
        }
    }

    /// 零拷贝的 `Bytes` 出口：把 [`VideoPlane::Cpu`] 的 `Arc<[u8]>` 所有权交给
    /// `Bytes::from_owner`，**只加一次引用计数，平面字节一个都不复制**。
    ///
    /// 下游若用 `Arc<[u8]>` 存平面就必须 memcpy；用 `bytes::Bytes` 存平面的契约
    /// （如主仓 `CpuPlanes`）直接用这个方法即可零拷贝交付。`slice` 同样零拷贝。
    pub fn bytes(&self) -> Option<Bytes> {
        match self {
            VideoPlane::Cpu { data } => Some(Bytes::from_owner(Arc::clone(data))),
            VideoPlane::Native { .. } => None,
        }
    }
}

/// 解码出来的一帧。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    format: VideoPixelFormat,
    coded_size: Size,
    visible_rect: Rect,
    natural_size: Size,
    layout: VideoFrameLayout,
    planes: Vec<VideoPlane>,
    timestamp: i64,
    duration: i64,
    /// 原生资源的持有者（CPU 帧没有）。
    native_owner: Option<NativeResourceOwner>,
}

impl VideoFrame {
    /// 用**系统内存**里的像素造一帧。
    ///
    /// `planes` 接受 `Vec<u8>` 或 `Arc<[u8]>`：前者在 `Into` 那一步转移分配，
    /// 后者连引用计数都不用动。两种输入都不会产生平面数据拷贝。
    ///
    /// `planes` 的长度必须等于格式的平面数；每个平面的数据长度必须至少
    /// 满足该平面的 `stride × 行数`。不满足就返回 `None` —— 宁可造不出帧，
    /// 也不要造一帧「看起来能显示、实际越界」的东西。
    pub fn from_cpu<T: Into<Arc<[u8]>> + AsRef<[u8]>>(
        format: VideoPixelFormat,
        coded_size: Size,
        visible_rect: Rect,
        natural_size: Size,
        planes: Vec<T>,
        timestamp: i64,
        duration: i64,
    ) -> Option<Self> {
        let layout = VideoFrameLayout::create(format, coded_size)?;
        if planes.len() != layout.num_planes() {
            return None;
        }
        for (i, plane) in planes.iter().enumerate() {
            let geom = layout.planes()[i];
            let rows = layout.plane_rows(i);
            if plane.as_ref().len() < geom.stride * rows {
                return None;
            }
        }
        Some(Self {
            format,
            coded_size,
            visible_rect,
            natural_size,
            layout,
            planes: planes
                .into_iter()
                .map(|data| VideoPlane::Cpu { data: data.into() })
                .collect(),
            timestamp,
            duration,
            native_owner: None,
        })
    }

    /// 用**平台原生资源**造一帧，并**持有**那些资源。
    ///
    /// 与 [`Self::from_native`] 的区别：后者只给句柄，资源的生命周期由
    /// 调用方自己保证；这一版把所有权放进帧里 —— 帧活着，资源就活着。
    ///
    /// `owner` 是**所有平面共享**的一个持有者（DXVA2 的 NV12 输出是
    /// 一张纹理，两个平面只是它的两个通道）。
    #[allow(clippy::too_many_arguments)]
    pub fn from_native_owned(
        format: VideoPixelFormat,
        coded_size: Size,
        visible_rect: Rect,
        natural_size: Size,
        handles: Vec<u64>,
        owner: NativeResourceOwner,
        timestamp: i64,
        duration: i64,
    ) -> Option<Self> {
        let mut frame = Self::from_native(
            format,
            coded_size,
            visible_rect,
            natural_size,
            handles,
            timestamp,
            duration,
        )?;
        frame.native_owner = Some(owner);
        Some(frame)
    }

    /// 用**平台原生资源**造一帧。
    ///
    /// `handles` 的长度必须等于平面数；每个句柄的含义由后端约定。
    pub fn from_native(
        format: VideoPixelFormat,
        coded_size: Size,
        visible_rect: Rect,
        natural_size: Size,
        handles: Vec<u64>,
        timestamp: i64,
        duration: i64,
    ) -> Option<Self> {
        let layout = VideoFrameLayout::create(format, coded_size)?;
        if handles.len() != layout.num_planes() {
            return None;
        }
        Some(Self {
            format,
            coded_size,
            visible_rect,
            natural_size,
            layout,
            planes: handles
                .into_iter()
                .map(|handle| VideoPlane::Native { handle })
                .collect(),
            timestamp,
            duration,
            native_owner: None,
        })
    }

    pub fn format(&self) -> VideoPixelFormat {
        self.format
    }
    pub fn coded_size(&self) -> Size {
        self.coded_size
    }
    pub fn visible_rect(&self) -> Rect {
        self.visible_rect
    }
    pub fn natural_size(&self) -> Size {
        self.natural_size
    }
    pub fn layout(&self) -> &VideoFrameLayout {
        &self.layout
    }
    pub fn planes(&self) -> &[VideoPlane] {
        &self.planes
    }
    pub fn num_planes(&self) -> usize {
        self.planes.len()
    }
    /// 呈现时间戳（微秒）。
    pub fn timestamp(&self) -> i64 {
        self.timestamp
    }
    pub fn duration(&self) -> i64 {
        self.duration
    }

    /// 该帧是否完全在系统内存里。
    pub fn is_cpu(&self) -> bool {
        self.planes.iter().all(VideoPlane::is_cpu)
    }

    /// 原生资源的持有者（CPU 帧没有）。
    pub fn native_owner(&self) -> Option<&NativeResourceOwner> {
        self.native_owner.as_ref()
    }
}

#[cfg(test)]
#[path = "../tests/unit/frame_tests.rs"]
mod tests;
