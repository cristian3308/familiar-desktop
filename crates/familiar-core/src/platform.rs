//! A tiny Slint platform for the shell: Win32 layered windows + Slint's
//! software renderer, no OpenGL and no winit.
//!
//! Why: with one OpenGL context per surface (dock, bar, island, widgets)
//! the GPU driver alone reserved ~40 MB per window. Here each surface is a
//! `WS_EX_LAYERED` popup; Slint renders into a 32-bit premultiplied DIB on
//! the CPU only when something changed, and `UpdateLayeredWindow` hands the
//! pixels to DWM. Fully transparent pixels are also click-through, which is
//! exactly the hit-testing the dock and the island need.
//!
//! The event loop blocks in `MsgWaitForMultipleObjects` until a window
//! message, a cross-thread wake-up, or the next Slint timer/animation frame.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::num::NonZeroIsize;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::platform::software_renderer::{PremultipliedRgbaColor, RepaintBufferType, SoftwareRenderer, TargetPixel};
use slint::platform::{EventLoopProxy, Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{EventLoopError, LogicalPosition, PhysicalPosition, PhysicalSize, PlatformError};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC, ScreenToClient,
    SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_BACK, VK_CONTROL,
    VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_LMENU, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT,
    VK_SHIFT, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, HWND_MESSAGE, IDC_ARROW, LoadCursorW, MSG,
    MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, PostMessageW, QS_ALLINPUT, RegisterClassW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage,
    ULW_ALPHA, UpdateLayeredWindow, WM_APP, WM_CHAR, WM_CLOSE, WM_DPICHANGED, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETFOCUS, WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, w};

const WM_WAKE: u32 = WM_APP + 0x60;
const WM_MOUSELEAVE: u32 = 0x02A3;
const FRAME: Duration = Duration::from_millis(16);

// ---------------------------------------------------------------------------
// Pixel format: BGRA, premultiplied — what UpdateLayeredWindow expects.

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Bgra {
    b: u8,
    g: u8,
    r: u8,
    a: u8,
}

impl TargetPixel for Bgra {
    fn blend(&mut self, c: PremultipliedRgbaColor) {
        let inv = (255 - c.alpha) as u16;
        let mix = |dst: u8, src: u8| ((dst as u16 * inv + 127) / 255) as u8 + src;
        self.r = mix(self.r, c.red);
        self.g = mix(self.g, c.green);
        self.b = mix(self.b, c.blue);
        self.a = mix(self.a, c.alpha);
    }

    fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self { b, g, r, a: 255 }
    }

    /// Transparent, so `background: transparent` windows stay see-through.
    fn background() -> Self {
        Self::default()
    }
}

// ---------------------------------------------------------------------------
// The off-screen surface (a DIB section selected into a memory DC).

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut Bgra,
    width: u32,
    height: u32,
}

impl Surface {
    fn new(width: u32, height: u32) -> Option<Self> {
        unsafe {
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    biHeight: -(height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let dc = CreateCompatibleDC(None);
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            let old = SelectObject(dc, HGDIOBJ(bitmap.0));
            Some(Self { dc, bitmap, old, bits: bits as *mut Bgra, width, height })
        }
    }

    fn pixels(&mut self) -> &mut [Bgra] {
        // SAFETY: the DIB section holds width*height 32-bit pixels for as
        // long as `bitmap` lives, and we hold `&mut self`.
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.width * self.height) as usize) }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

// ---------------------------------------------------------------------------
// Window adapter.

pub struct LayeredWindow {
    me: Weak<LayeredWindow>,
    window: slint::Window,
    renderer: SoftwareRenderer,
    hwnd: HWND,
    size: Cell<PhysicalSize>,
    position: Cell<PhysicalPosition>,
    visible: Cell<bool>,
    needs_redraw: Cell<bool>,
    tracking_leave: Cell<bool>,
    high_surrogate: Cell<Option<u16>>,
    surface: RefCell<Option<Surface>>,
}

thread_local! {
    static WINDOWS: RefCell<HashMap<isize, Weak<LayeredWindow>>> = RefCell::new(HashMap::new());
    /// Input events are queued and delivered from the event loop, never from
    /// inside a window procedure: our own callbacks call Win32 APIs (hide,
    /// activate...) that send messages synchronously, and handing those to
    /// Slint while it is still dispatching the previous event re-enters it.
    static PENDING: RefCell<VecDeque<(Weak<LayeredWindow>, WindowEvent)>> = const { RefCell::new(VecDeque::new()) };
}

fn deliver_pending() {
    loop {
        let Some((target, event)) = PENDING.with(|p| p.borrow_mut().pop_front()) else { return };
        if let Some(w) = target.upgrade() {
            w.window.dispatch_event(event);
        }
    }
}

fn adapter_for(hwnd: HWND) -> Option<Rc<LayeredWindow>> {
    WINDOWS.with(|w| w.borrow().get(&(hwnd.0 as isize)).and_then(Weak::upgrade))
}

impl LayeredWindow {
    fn new() -> Result<Rc<Self>, PlatformError> {
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                SURFACE_CLASS,
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(GetModuleHandleW(None).unwrap_or_default().into()),
                None,
            )
        }
        .map_err(|e| PlatformError::Other(format!("CreateWindowExW: {e}")))?;

        let this = Rc::new_cyclic(|weak: &Weak<Self>| Self {
            me: weak.clone(),
            window: slint::Window::new(weak.clone() as Weak<dyn WindowAdapter>),
            renderer: SoftwareRenderer::new_with_repaint_buffer_type(RepaintBufferType::NewBuffer),
            hwnd,
            size: Cell::new(PhysicalSize::new(0, 0)),
            position: Cell::new(PhysicalPosition::new(0, 0)),
            visible: Cell::new(false),
            needs_redraw: Cell::new(true),
            tracking_leave: Cell::new(false),
            high_surrogate: Cell::new(None),
            surface: RefCell::new(None),
        });
        WINDOWS.with(|w| w.borrow_mut().insert(hwnd.0 as isize, Rc::downgrade(&this)));
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        this.window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: dpi as f32 / 96.0 });
        Ok(this)
    }

    fn scale(&self) -> f32 {
        self.window.scale_factor()
    }

    fn logical(&self, lp: LPARAM) -> LogicalPosition {
        let x = (lp.0 & 0xFFFF) as i16 as f32;
        let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32;
        LogicalPosition::new(x / self.scale(), y / self.scale())
    }

    fn wants_frame(&self) -> bool {
        self.visible.get() && (self.needs_redraw.get() || self.window.has_active_animations())
    }

    /// Renders the scene and pushes it to DWM.
    fn render(&self) {
        self.needs_redraw.set(false);
        let size = self.size.get();
        if !self.visible.get() || size.width == 0 || size.height == 0 {
            return;
        }
        let mut slot = self.surface.borrow_mut();
        if slot.as_ref().is_none_or(|s| s.width != size.width || s.height != size.height) {
            *slot = Surface::new(size.width, size.height);
        }
        let Some(surface) = slot.as_mut() else { return };
        let pixels = surface.pixels();
        pixels.fill(Bgra::default());
        self.renderer.render(pixels, size.width as usize);

        // No destination point: the window keeps wherever it was placed
        // (set_position, AppBar reservation, or a native drag move).
        let src = POINT { x: 0, y: 0 };
        let sz = SIZE { cx: size.width as i32, cy: size.height as i32 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            let screen = GetDC(None);
            let _ = UpdateLayeredWindow(
                self.hwnd,
                Some(screen),
                None,
                Some(&sz),
                Some(surface.dc),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            ReleaseDC(None, screen);
        }
    }

    fn move_native(&self) {
        let (p, s) = (self.position.get(), self.size.get());
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                p.x,
                p.y,
                s.width.max(1) as i32,
                s.height.max(1) as i32,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    fn handle(&self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let dispatch = |e: WindowEvent| PENDING.with(|p| p.borrow_mut().push_back((self.me.clone(), e)));
        match msg {
            WM_MOUSEMOVE => {
                if !self.tracking_leave.replace(true) {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut tme);
                    }
                }
                dispatch(WindowEvent::PointerMoved { position: self.logical(lp) });
            }
            WM_MOUSELEAVE => {
                self.tracking_leave.set(false);
                dispatch(WindowEvent::PointerExited);
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
                log::trace!("pointer down {msg:#x} {:?} at {:?}", self.hwnd, self.logical(lp));
                unsafe { SetCapture(self.hwnd) };
                let button = match msg {
                    WM_RBUTTONDOWN => PointerEventButton::Right,
                    WM_MBUTTONDOWN => PointerEventButton::Middle,
                    _ => PointerEventButton::Left,
                };
                dispatch(WindowEvent::PointerPressed { position: self.logical(lp), button });
            }
            WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP => {
                log::trace!("pointer up {msg:#x} {:?} at {:?}", self.hwnd, self.logical(lp));
                unsafe {
                    let _ = ReleaseCapture();
                }
                let button = match msg {
                    WM_RBUTTONUP => PointerEventButton::Right,
                    WM_MBUTTONUP => PointerEventButton::Middle,
                    _ => PointerEventButton::Left,
                };
                dispatch(WindowEvent::PointerReleased { position: self.logical(lp), button });
            }
            WM_MOUSEWHEEL => {
                // Wheel coordinates are in screen space.
                let mut pt = POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 };
                unsafe {
                    let _ = ScreenToClient(self.hwnd, &mut pt);
                }
                let notches = ((wp.0 >> 16) & 0xFFFF) as i16 as f32 / 120.0;
                let position = LogicalPosition::new(pt.x as f32 / self.scale(), pt.y as f32 / self.scale());
                dispatch(WindowEvent::PointerScrolled { position, delta_x: 0.0, delta_y: notches * 40.0 });
            }
            WM_SETFOCUS => dispatch(WindowEvent::WindowActiveChanged(true)),
            WM_KILLFOCUS => dispatch(WindowEvent::WindowActiveChanged(false)),
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let repeat = lp.0 & (1 << 30) != 0;
                if let Some(text) = key_text(wp.0 as u16) {
                    dispatch(if repeat {
                        WindowEvent::KeyPressRepeated { text }
                    } else {
                        WindowEvent::KeyPressed { text }
                    });
                }
                if msg == WM_SYSKEYDOWN {
                    return None; // keep Alt+F4 and friends working
                }
            }
            WM_KEYUP | WM_SYSKEYUP => {
                if let Some(text) = key_text(wp.0 as u16) {
                    dispatch(WindowEvent::KeyReleased { text });
                }
                if msg == WM_SYSKEYUP {
                    return None;
                }
            }
            WM_CHAR => {
                let unit = wp.0 as u16;
                let text = if (0xD800..0xDC00).contains(&unit) {
                    self.high_surrogate.set(Some(unit));
                    return Some(LRESULT(0));
                } else if let Some(high) = self.high_surrogate.take() {
                    String::from_utf16_lossy(&[high, unit])
                } else if unit < 0x20 || unit == 0x7F || ctrl_down() {
                    // Control characters arrive as keys via WM_KEYDOWN.
                    return Some(LRESULT(0));
                } else {
                    String::from_utf16_lossy(&[unit])
                };
                dispatch(WindowEvent::KeyPressed { text: text.as_str().into() });
                dispatch(WindowEvent::KeyReleased { text: text.as_str().into() });
            }
            WM_DPICHANGED => {
                let dpi = (wp.0 & 0xFFFF) as f32;
                dispatch(WindowEvent::ScaleFactorChanged { scale_factor: dpi / 96.0 });
                self.needs_redraw.set(true);
            }
            WM_CLOSE => return Some(LRESULT(0)), // surfaces live as long as the shell
            _ => return None,
        }
        Some(LRESULT(0))
    }
}

fn ctrl_down() -> bool {
    (unsafe { GetKeyState(VK_CONTROL.0 as i32) }) < 0
}

/// Translates a virtual key into Slint's key text. Letters are only
/// reported here while Ctrl is held (Ctrl+A, Ctrl+C...); otherwise text
/// comes from WM_CHAR, which respects the keyboard layout.
fn key_text(vk: u16) -> Option<slint::SharedString> {
    let key = match vk {
        v if v == VK_BACK.0 => Key::Backspace,
        v if v == VK_TAB.0 => Key::Tab,
        v if v == VK_RETURN.0 => Key::Return,
        v if v == VK_ESCAPE.0 => Key::Escape,
        v if v == VK_DELETE.0 => Key::Delete,
        v if v == VK_LEFT.0 => Key::LeftArrow,
        v if v == VK_RIGHT.0 => Key::RightArrow,
        v if v == VK_UP.0 => Key::UpArrow,
        v if v == VK_DOWN.0 => Key::DownArrow,
        v if v == VK_HOME.0 => Key::Home,
        v if v == VK_END.0 => Key::End,
        v if v == VK_PRIOR.0 => Key::PageUp,
        v if v == VK_NEXT.0 => Key::PageDown,
        v if v == VK_SHIFT.0 => Key::Shift,
        v if v == VK_CONTROL.0 => Key::Control,
        v if v == VK_MENU.0 || v == VK_LMENU.0 => Key::Alt,
        v if (0x41..=0x5A).contains(&v) && ctrl_down() => {
            return Some(char::from(v as u8).to_ascii_lowercase().to_string().into());
        }
        _ => return None,
    };
    Some(key.into())
}

impl WindowAdapter for LayeredWindow {
    fn window(&self) -> &slint::Window {
        &self.window
    }

    fn renderer(&self) -> &dyn slint::platform::Renderer {
        &self.renderer
    }

    fn size(&self) -> PhysicalSize {
        self.size.get()
    }

    fn set_size(&self, size: slint::WindowSize) {
        let sf = self.scale();
        self.size.set(size.to_physical(sf));
        self.window.dispatch_event(WindowEvent::Resized { size: size.to_logical(sf) });
        self.move_native();
        self.needs_redraw.set(true);
        wake();
    }

    fn position(&self) -> Option<PhysicalPosition> {
        Some(self.position.get())
    }

    fn set_position(&self, position: slint::WindowPosition) {
        self.position.set(position.to_physical(self.scale()));
        self.move_native();
        self.needs_redraw.set(true);
        wake();
    }

    fn set_visible(&self, visible: bool) -> Result<(), PlatformError> {
        if self.size.get().width == 0 {
            // Nobody sized the window: fall back to its preferred layout size.
            let pref = self.window.size();
            if pref.width == 0 {
                self.set_size(slint::WindowSize::Logical(slint::LogicalSize::new(100.0, 100.0)));
            }
        }
        self.visible.set(visible);
        unsafe {
            let _ = ShowWindow(self.hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
        if visible {
            self.needs_redraw.set(true);
            wake();
        } else {
            // Release the pixel buffer while hidden.
            self.surface.borrow_mut().take();
        }
        Ok(())
    }

    fn request_redraw(&self) {
        self.needs_redraw.set(true);
        wake();
    }

    fn update_window_properties(&self, properties: slint::platform::WindowProperties<'_>) {
        let title = HSTRING::from(properties.title().as_str());
        unsafe {
            let _ = SetWindowTextW(self.hwnd, PCWSTR(title.as_ptr()));
        }
    }

    fn window_handle_06(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let raw = NonZeroIsize::new(self.hwnd.0 as isize).ok_or(raw_window_handle::HandleError::Unavailable)?;
        let handle = raw_window_handle::Win32WindowHandle::new(raw);
        // SAFETY: the HWND lives as long as this adapter.
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw_window_handle::RawWindowHandle::Win32(handle)) })
    }
}

impl Drop for LayeredWindow {
    fn drop(&mut self) {
        WINDOWS.with(|w| w.borrow_mut().remove(&(self.hwnd.0 as isize)));
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

// ---------------------------------------------------------------------------
// Platform + event loop.

const SURFACE_CLASS: PCWSTR = w!("FamiliarCoreSurface");
const WAKE_CLASS: PCWSTR = w!("FamiliarCoreWake");

unsafe extern "system" fn surface_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let handled = familiar_sys::safety::ffi_guard(None, || adapter_for(hwnd).and_then(|w| w.handle(msg, wp, lp)));
    match handled {
        Some(r) => r,
        None => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

unsafe extern "system" fn wake_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // WM_WAKE only needs to make MsgWaitForMultipleObjects return.
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

type Job = Box<dyn FnOnce() + Send>;

struct Shared {
    jobs: Mutex<VecDeque<Job>>,
    quit: AtomicBool,
    wake_hwnd: isize,
}

static SHARED: Mutex<Option<Arc<Shared>>> = Mutex::new(None);

fn wake() {
    if let Some(s) = SHARED.lock().ok().and_then(|g| g.clone()) {
        unsafe {
            let _ = PostMessageW(Some(HWND(s.wake_hwnd as *mut _)), WM_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

struct Proxy(Arc<Shared>);

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        self.0.quit.store(true, Ordering::SeqCst);
        wake();
        Ok(())
    }

    fn invoke_from_event_loop(&self, event: Box<dyn FnOnce() + Send>) -> Result<(), EventLoopError> {
        self.0.jobs.lock().map_err(|_| EventLoopError::EventLoopTerminated)?.push_back(event);
        wake();
        Ok(())
    }
}

pub struct LayeredPlatform {
    shared: Arc<Shared>,
    windows: RefCell<Vec<Weak<LayeredWindow>>>,
}

impl LayeredPlatform {
    pub fn new() -> Result<Self, PlatformError> {
        unsafe {
            let instance = GetModuleHandleW(None).unwrap_or_default().into();
            let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(surface_proc),
                hInstance: instance,
                hCursor: cursor,
                lpszClassName: SURFACE_CLASS,
                ..Default::default()
            });
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(wake_proc),
                hInstance: instance,
                lpszClassName: WAKE_CLASS,
                ..Default::default()
            });
            let wake_hwnd = CreateWindowExW(
                Default::default(),
                WAKE_CLASS,
                w!(""),
                Default::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance),
                None,
            )
            .map_err(|e| PlatformError::Other(format!("wake window: {e}")))?;
            let shared = Arc::new(Shared {
                jobs: Mutex::new(VecDeque::new()),
                quit: AtomicBool::new(false),
                wake_hwnd: wake_hwnd.0 as isize,
            });
            *SHARED.lock().map_err(|_| PlatformError::Other("poisoned".into()))? = Some(shared.clone());
            Ok(Self { shared, windows: RefCell::new(Vec::new()) })
        }
    }

    fn render_pending(&self) -> bool {
        let mut animating = false;
        self.windows.borrow_mut().retain(|w| w.strong_count() > 0);
        let live: Vec<Rc<LayeredWindow>> = self.windows.borrow().iter().filter_map(Weak::upgrade).collect();
        for w in live {
            if w.wants_frame() {
                w.render();
            }
            animating |= w.visible.get() && w.window.has_active_animations();
        }
        animating
    }
}

impl Platform for LayeredPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let w = LayeredWindow::new()?;
        self.windows.borrow_mut().push(Rc::downgrade(&w));
        Ok(w)
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.shared.clone())))
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        self.shared.quit.store(false, Ordering::SeqCst);
        loop {
            deliver_pending();
            slint::platform::update_timers_and_animations();
            let jobs: Vec<Job> = self.shared.jobs.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default();
            for job in jobs {
                job();
            }
            if self.shared.quit.load(Ordering::SeqCst) {
                return Ok(());
            }
            slint::platform::update_timers_and_animations();
            let animating = self.render_pending();

            let mut timeout = slint::platform::duration_until_next_timer_update();
            if animating {
                timeout = Some(timeout.map_or(FRAME, |t| t.min(FRAME)));
            }
            let ms = timeout.map_or(u32::MAX, |t| t.as_millis().min(u32::MAX as u128 - 1) as u32);
            unsafe { MsgWaitForMultipleObjects(None, false, ms, QS_ALLINPUT) };

            let mut msg = MSG::default();
            while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                if msg.message == WM_QUIT {
                    return Ok(());
                }
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                deliver_pending();
            }
        }
    }
}
