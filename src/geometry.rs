//! 视频用的**整数**像素几何（对应 Chromium 的 `gfx::Size` / `gfx::Rect`）。
//!
//! 刻意**不复用** `zephyra_core::core::Rect`：那个类型是 f32 的 **DIP** 矩形
//! （屏幕坐标、y 向下、可带小数），而视频的 coded/visible 矩形是**像素整数**。
//! 两者混用会让「半像素裁剪」这类错误静默通过 —— f32 装得下 0.5 像素，
//! 但 DXVA 的裁剪偏移是整数，编译期与运行期都不会拦你。
//!
//! 这里只放视频解码真正需要的两件东西：尺寸与矩形。

/// 像素尺寸（对应 `gfx::Size`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// 宽或高为 0 时为空。对应 Chromium `gfx::Size::IsEmpty()`。
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// 像素矩形（对应 `gfx::Rect`）。
///
/// 原点左上、y 向下，与 Chromium 的 `gfx::Rect` 一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn right(&self) -> i32 {
        self.x + self.width as i32
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }

    /// 宽度（像素）。对应 Chromium `gfx::Rect::width()`。
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// 高度（像素）。对应 Chromium `gfx::Rect::height()`。
    ///
    /// 解码器选择器的 360 像素分界就是拿它比的（Chromium
    /// `decoder_selector.cc` 里的 `visible_rect.height()`）。
    pub const fn height(&self) -> u32 {
        self.height
    }

    pub const fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    /// 宽或高为 0 时为空。对应 Chromium `gfx::Rect::IsEmpty()`。
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// 矩形是否完全落在 `bounds` 内。用于校验 visible_rect 不越出 coded 区域。
    pub fn is_inside(&self, bounds: &Rect) -> bool {
        self.x >= bounds.x
            && self.y >= bounds.y
            && self.right() <= bounds.right()
            && self.bottom() <= bounds.bottom()
    }

    /// `other` 是否完全落在本矩形内。
    ///
    /// 对应 Chromium `gfx::Rect::Contains(const gfx::Rect&)` ——
    /// `VideoDecoderConfig::IsValidConfig()` 用它保证 visible_rect 不越界。
    pub fn contains(&self, other: &Rect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }
}

#[cfg(test)]
#[path = "../tests/unit/geometry_tests.rs"]
mod tests;
