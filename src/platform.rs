//! Window placement and the few OS calls the viewer needs that winit/eframe don't offer.
//! The careful work is on Windows; elsewhere the window simply maximizes and fullscreens.

use eframe::egui::{self, ViewportBuilder, ViewportCommand};
#[cfg(windows)]
use eframe::egui::{Pos2, Rect, pos2, vec2};

/// The display scaling Windows will give the window (1.5 at 150%).
#[cfg(windows)]
pub fn scale_factor() -> f32 {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetProcessDpiAwarenessContext(context: isize) -> i32;
        fn GetDpiForSystem() -> u32;
    }
    const PER_MONITOR_AWARE_V2: isize = -4;
    // winit makes the process DPI aware too, but only once the window is created; without
    // this the query below would always report 96 DPI.
    // SAFETY: plain Win32 calls with no pointers involved.
    unsafe {
        SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2);
        GetDpiForSystem() as f32 / 96.0
    }
}

/// The primary screen, and the part of it not covered by the taskbar, in egui points.
#[cfg(windows)]
fn screen_and_work_area() -> (Rect, Rect) {
    #[repr(C)]
    #[derive(Default)]
    struct WinRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetSystemMetrics(index: i32) -> i32;
        fn SystemParametersInfoW(action: u32, param: u32, out: *mut WinRect, ini: u32) -> i32;
    }
    const SM_CXSCREEN: i32 = 0;
    const SM_CYSCREEN: i32 = 1;
    const SPI_GETWORKAREA: u32 = 0x30;

    let scale = scale_factor(); // also makes the queries below report real pixels
    // SAFETY: plain Win32 calls; `work` is a valid RECT to write into.
    let (screen, work) = unsafe {
        let screen = vec2(
            GetSystemMetrics(SM_CXSCREEN) as f32,
            GetSystemMetrics(SM_CYSCREEN) as f32,
        );
        let mut work = WinRect::default();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work, 0);
        (screen, work)
    };
    let screen = Rect::from_min_size(Pos2::ZERO, screen / scale);
    let work = Rect::from_min_max(
        pos2(work.left as f32, work.top as f32) / scale,
        pos2(work.right as f32, work.bottom as f32) / scale,
    );
    (screen, if work.is_positive() { work } else { screen })
}

/// Tell the taskbar to stay out of the way of the (active) viewer window while fullscreen.
#[cfg(windows)]
fn mark_fullscreen(on: bool) {
    use std::ffi::c_void;

    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);
    const CLSID_TASKBAR_LIST: Guid = Guid(
        0x56FDF344,
        0xFD6D,
        0x11d0,
        [0x95, 0x8A, 0x00, 0x60, 0x97, 0xC9, 0xA0, 0x90],
    );
    const IID_ITASKBAR_LIST2: Guid = Guid(
        0x602D4995,
        0xB13A,
        0x429b,
        [0xA6, 0x6E, 0x19, 0x35, 0xE4, 0x4F, 0x43, 0x17],
    );
    const CLSCTX_INPROC_SERVER: u32 = 1;

    /// ITaskbarList2's vtable: IUnknown, then ITaskbarList, then ITaskbarList2.
    #[repr(C)]
    struct TaskbarListVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        hr_init: unsafe extern "system" fn(*mut c_void) -> i32,
        add_tab: usize,
        delete_tab: usize,
        activate_tab: usize,
        set_active_alt: usize,
        mark_fullscreen_window: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    }

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoCreateInstance(
            clsid: *const Guid,
            outer: *mut c_void,
            ctx: u32,
            iid: *const Guid,
            out: *mut *mut c_void,
        ) -> i32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetActiveWindow() -> *mut c_void;
    }

    // SAFETY: COM is already initialized on the UI thread (winit does it for drag and drop);
    // the object comes from CoCreateInstance with the interface we call through, and is
    // released before returning.
    unsafe {
        let hwnd = GetActiveWindow();
        let mut list: *mut c_void = std::ptr::null_mut();
        let hr = CoCreateInstance(
            &CLSID_TASKBAR_LIST,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_ITASKBAR_LIST2,
            &mut list,
        );
        if hr < 0 || list.is_null() || hwnd.is_null() {
            return;
        }
        let vtbl = &**(list as *mut *const TaskbarListVtbl);
        if (vtbl.hr_init)(list) >= 0 {
            (vtbl.mark_fullscreen_window)(list, hwnd, on as i32);
        }
        (vtbl.release)(list);
    }
}

/// Where the window goes on Windows: the desktop area above the taskbar, like a maximized
/// app (but without the invisible border a real maximize adds to borderless windows), or the
/// whole screen. Not real fullscreen: a window covering exactly the screen gets switched into
/// the driver's exclusive fullscreen mode, which blacks out the display for a moment on the
/// way in and out, so it overhangs the screen by a point instead.
#[cfg(windows)]
fn window_rect(fullscreen: bool) -> Rect {
    let (screen, work) = screen_and_work_area();
    if fullscreen { screen.expand(1.0) } else { work }
}

/// The window's size in points before it exists, if known.
#[cfg(windows)]
pub fn initial_window_size() -> Option<egui::Vec2> {
    Some(window_rect(false).size())
}

/// Open like a maximized app.
#[cfg(windows)]
pub fn place_window(builder: ViewportBuilder) -> ViewportBuilder {
    let rect = window_rect(false);
    builder.with_position(rect.min).with_inner_size(rect.size())
}

#[cfg(windows)]
pub fn set_fullscreen(ctx: &egui::Context, on: bool) {
    let rect = window_rect(on);
    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(rect.min));
    ctx.send_viewport_cmd(ViewportCommand::InnerSize(rect.size()));
    mark_fullscreen(on);
}

#[cfg(not(windows))]
pub fn scale_factor() -> f32 {
    1.0
}

#[cfg(not(windows))]
pub fn initial_window_size() -> Option<egui::Vec2> {
    None
}

#[cfg(not(windows))]
pub fn place_window(builder: ViewportBuilder) -> ViewportBuilder {
    builder.with_maximized(true)
}

#[cfg(not(windows))]
pub fn set_fullscreen(ctx: &egui::Context, on: bool) {
    ctx.send_viewport_cmd(ViewportCommand::Fullscreen(on));
}
