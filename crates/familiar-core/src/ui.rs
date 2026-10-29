//! Generated Slint components plus glue shared by every surface.

use familiar_sys::Hwnd;
use familiar_theme::{Color as ThemeColor, Theme as ThemeSpec};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

slint::include_modules!();

pub fn color(c: ThemeColor) -> slint::Color {
    slint::Color::from_argb_u8(c.a, c.r, c.g, c.b)
}

/// The native window handle; `None` until the window has been shown once.
pub fn hwnd_of(window: &slint::Window) -> Option<Hwnd> {
    let handle = window.window_handle();
    let raw = handle.window_handle().ok()?.as_raw();
    match raw {
        RawWindowHandle::Win32(h) => Some(Hwnd(h.hwnd.get())),
        _ => None,
    }
}

pub fn image_from_rgba(width: u32, height: u32, rgba: &[u8]) -> Image {
    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba, width, height);
    Image::from_rgba8(buf)
}

pub fn load_icon(path: &std::path::Path) -> Image {
    if path.as_os_str().is_empty() || !path.exists() {
        return Image::default();
    }
    Image::load_from_path(path).unwrap_or_default()
}

/// Pushes theme values into a component's `Theme` global (every component
/// instance has its own copy of Slint globals).
pub fn push_theme<C>(component: &C, t: &ThemeSpec)
where
    C: ComponentHandle,
    for<'a> Theme<'a>: slint::Global<'a, C>,
{
    let g = component.global::<Theme>();
    g.set_tint(color(t.colors.background_tint));
    g.set_accent(color(t.colors.accent));
    g.set_text(color(t.colors.text_primary));
    g.set_text_secondary(color(t.colors.text_secondary));
    g.set_glow(color(t.colors.border_glow));
    g.set_island(color(t.colors.island_background));
    g.set_radius(t.metrics.corner_radius as f32);
    g.set_bar_height(t.metrics.top_bar_height as f32);
}

/// `0xAABBGGRR` tint for the Windows 10 acrylic fallback.
pub fn abgr(c: ThemeColor) -> u32 {
    (u32::from(c.a) << 24) | c.to_colorref()
}

/// Moves/resizes a top-level surface through Slint, so the renderer and the
/// layout pick up the new size. (A raw `SetWindowPos` moved the native window
/// but left Slint drawing at the old size, which clipped the dock.)
pub fn set_bounds(window: &slint::Window, r: familiar_sys::window::Rect) {
    window.set_position(slint::PhysicalPosition::new(r.x, r.y));
    window.set_size(slint::PhysicalSize::new(r.w.max(1) as u32, r.h.max(1) as u32));
}

/// Shows/hides a shell surface without activating it (Slint's own `show()`
/// uses `SW_SHOW`, which would steal focus when leaving calm mode).
pub fn set_native_visible(hwnd: Option<Hwnd>, visible: bool) {
    if let Some(h) = hwnd {
        familiar_sys::window::show_no_activate(h.get(), visible);
        if visible {
            familiar_sys::window::raise_topmost_if_topmost(h.get());
        }
    }
}

/// Clips a card-shaped window to the theme's corner radius so the DWM
/// backdrop follows the rounded shape.
pub fn round_region(hwnd: Hwnd, window: &slint::Window, radius_logical: f32) {
    let size = window.size();
    let r = (radius_logical * window.scale_factor()).round() as i32;
    let rect = familiar_sys::window::Rect { x: 0, y: 0, w: size.width as i32, h: size.height as i32 };
    familiar_sys::window::set_region(hwnd.get(), &[familiar_sys::window::RegionPart { rect, radius: r }]);
}

/// Applies the theme's DWM material and the matching light/dark mode.
pub fn apply_material(hwnd: Hwnd, t: &ThemeSpec, rounded: bool) {
    use familiar_sys::dwm;
    let h = hwnd.get();
    dwm::disable_transitions(h);
    dwm::hide_border(h);
    dwm::set_rounded(h, rounded);
    dwm::set_dark_mode(h, t.colors.background_tint.luminance() < 0.5);
    dwm::set_backdrop(h, t.metrics.blur_type, abgr(t.colors.background_tint));
}
