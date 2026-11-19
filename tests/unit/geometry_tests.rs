//! `geometry.rs` 的断言：整数像素几何。

use super::super::{Rect, Size};

#[test]
fn empty_size_has_zero_dimension() {
    assert!(Size::new(0, 100).is_empty());
    assert!(Size::new(100, 0).is_empty());
    assert!(!Size::new(1, 1).is_empty());
}

#[test]
fn rect_right_and_bottom_are_exclusive() {
    let rect = Rect::new(10, 20, 30, 40);
    assert_eq!(rect.right(), 40);
    assert_eq!(rect.bottom(), 60);
}

#[test]
fn rect_size_matches_width_and_height() {
    let rect = Rect::new(1, 2, 3, 4);
    assert_eq!(rect.size(), Size::new(3, 4));
    assert_eq!(rect.width(), 3);
    assert_eq!(rect.height(), 4);
}

#[test]
fn empty_rect_has_zero_extent() {
    assert!(Rect::new(0, 0, 0, 10).is_empty());
    assert!(Rect::new(0, 0, 10, 0).is_empty());
    assert!(!Rect::new(0, 0, 1, 1).is_empty());
}

#[test]
fn contains_is_inclusive_on_the_far_edge() {
    let outer = Rect::new(0, 0, 100, 50);
    assert!(outer.contains(&Rect::new(0, 0, 100, 50)));
    assert!(outer.contains(&Rect::new(10, 10, 1, 1)));
    // Chromium `gfx::Rect::Contains` 的右 / 下边界是**闭**的：
    // `other.right() <= right()`。贴边算在里面。
    assert!(outer.contains(&Rect::new(10, 10, 90, 40)));
    // 越界则不算。
    assert!(!outer.contains(&Rect::new(10, 10, 91, 40)));
    assert!(!outer.contains(&Rect::new(-1, 0, 10, 10)));
}

#[test]
fn is_inside_requires_full_containment() {
    let bounds = Rect::new(0, 0, 64, 64);
    assert!(Rect::new(0, 0, 64, 64).is_inside(&bounds));
    assert!(Rect::new(16, 16, 32, 32).is_inside(&bounds));
    assert!(!Rect::new(32, 32, 64, 64).is_inside(&bounds));
}
