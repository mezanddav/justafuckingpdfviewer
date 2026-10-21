#![windows_subsystem = "windows"]

#[cfg(windows)]
mod assoc;
mod doc;
mod platform;

use std::collections::HashMap;
use std::path::PathBuf;

use doc::{DocInfo, Document, Job, Msg, TextChar};
use eframe::egui::{
    self, Align2, Color32, CursorIcon, Event, FontId, Key, PointerButton, Pos2, Rect, Sense,
    TextureHandle, TextureOptions, Vec2, ViewportCommand, pos2, vec2,
};

/// The classic PDF viewer gray (same as Chrome/Edge's viewer).
const BACKGROUND: Color32 = Color32::from_rgb(0x52, 0x56, 0x59);
const SELECTION: Color32 = Color32::from_rgba_premultiplied(0, 66, 120, 90);
/// Actual size: one PDF point is 1/72 inch and egui points are 1/96 inch (before display scaling).
const ACTUAL_SIZE: f32 = 96.0 / 72.0;
/// Space around and between pages, in screen points.
const GAP: f32 = 16.0;
/// Breathing room around the page in fit-to-screen mode.
const FIT_MARGIN: f32 = 24.0;
const LINE_SCROLL: f32 = 60.0;
/// Ctrl+wheel zoom range, relative to actual size.
const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 5.0;
/// After a quick drag the page keeps gliding and slows down exponentially (an ease-out
/// curve); this is how long, in seconds, the glide takes to lose ~63% of its speed.
const FLING_TIME_CONSTANT: f32 = 0.35;
const MIN_FLING_SPEED: f32 = 150.0;
const MAX_FLING_SPEED: f32 = 9000.0;
/// Keep the wheel going and scrolling speeds up: after this many seconds of continuous
/// scrolling it ramps up over `WHEEL_ACCEL_RAMP` seconds to `WHEEL_MAX_ACCEL` times normal.
const WHEEL_ACCEL_DELAY: f64 = 0.3;
const WHEEL_ACCEL_RAMP: f64 = 1.0;
const WHEEL_MAX_ACCEL: f32 = 8.0;
/// A pause longer than this between wheel notches ends the streak.
const WHEEL_STREAK_GAP: f64 = 0.25;

/// Joystick panning: no movement within this radius of the anchor, then the speed follows
/// a curve so small pushes crawl and big pushes fly.
const JOYSTICK_DEAD_ZONE: f32 = 6.0;
const JOYSTICK_GAIN: f32 = 4.0;
const JOYSTICK_CURVE: f32 = 1.5;
const JOYSTICK_MAX_SPEED: f32 = 15000.0;

/// The page preview strip on the left: previews are this wide (or this tall, for tall pages),
/// faint until the mouse is over the strip.
const THUMB_WIDTH: f32 = 96.0;
const THUMB_MAX_HEIGHT: f32 = 150.0;
const THUMB_PAD: f32 = 12.0;
const THUMB_LABEL: f32 = 16.0;
const THUMB_GAP: f32 = 8.0;
const STRIP_WIDTH: f32 = THUMB_WIDTH + 2.0 * THUMB_PAD;
const STRIP_FAINT: f32 = 0.35;
const ACCENT: Color32 = Color32::from_rgb(0x4d, 0xa3, 0xff);

/// Page-by-page mode: wheel travel (in points) that counts as one step, how much of the
/// screen one step moves inside a page taller than the screen, and how quickly the glide
/// to the target settles (seconds).
const PAGE_WHEEL_STEP: f32 = 50.0;
const PAGE_SCROLL_FRACTION: f32 = 0.8;
const PAGE_GLIDE_TIME: f32 = 0.07;
/// How long the mode label stays up, in seconds.
const TOAST_TIME: f64 = 1.2;

const HOTKEYS: &[(&str, &str)] = &[
    ("Hold right/middle", "Joystick: page flies toward the mouse"),
    ("Wheel / drag", "Move around"),
    ("Keep scrolling", "Scrolls faster and faster"),
    ("Flick and let go", "Page keeps gliding"),
    ("Shift + wheel", "Scroll sideways"),
    ("Ctrl + wheel", "Zoom at the mouse"),
    ("S", "Whole page / actual size"),
    ("Shift + S", "Scroll page by page / continuously"),
    ("Space", "Reset view: whole page, this page"),
    ("Arrows, PgUp/PgDn", "Scroll"),
    ("Home / End", "First / last page"),
    ("Drag over text", "Select it"),
    ("Ctrl + C", "Copy selected text"),
    ("F", "Fullscreen on / off"),
    ("A", "Page previews on / off"),
    ("Click a preview", "Go to that page"),
    ("I", "File info"),
    ("H", "This list"),
    ("X / Esc", "Quit"),
    ("Drop a PDF", "Open it"),
];

const APP_NAME: &str = "justafuckingpdfviewer";
/// Set when relaunching with the fallback renderer (see `main`).
const FALLBACK_RENDERER_ENV: &str = "JUSTAFUCKINGPDFVIEWER_FALLBACK_RENDERER";

fn main() -> eframe::Result {
    #[cfg(windows)]
    std::thread::spawn(assoc::register);

    // Start reading and parsing the PDF right away, while the window is still being created.
    let ctx = egui::Context::default();
    // Page 1 gets rendered at its fitted size before the window even exists (when the window
    // size is known up front).
    let first_page = platform::initial_window_size().map(|window| {
        let ppp = platform::scale_factor();
        Box::new(move |page| fit_zoom(page, window) * ppp) as doc::FirstPageScale
    });
    let doc = std::env::args_os()
        .nth(1)
        .map(|p| Document::open(PathBuf::from(p), ctx.clone(), first_page));
    let use_fallback_renderer = std::env::var_os(FALLBACK_RENDERER_ENV).is_some();
    let title = doc
        .as_ref()
        .map_or(APP_NAME.into(), |d| window_title(&d.path));

    let options = eframe::NativeOptions {
        viewport: platform::place_window(egui::ViewportBuilder::default())
            .with_title(title)
            .with_icon(egui::IconData {
                rgba: include_bytes!(concat!(env!("OUT_DIR"), "/icon_256.rgba")).to_vec(),
                width: 256,
                height: 256,
            })
            .with_decorations(false),
        renderer: if use_fallback_renderer {
            eframe::Renderer::Wgpu
        } else {
            eframe::Renderer::Glow
        },
        ..Default::default()
    };
    let result = eframe::run_native_ext(
        APP_NAME,
        options,
        Some(ctx),
        Box::new(move |cc| {
            cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
            Ok(Box::new(Viewer {
                doc,
                fit: true,
                page_mode: true,
                ..Default::default()
            }))
        }),
    );
    if result.is_err() && !use_fallback_renderer {
        // No usable OpenGL (VMs, remote desktop, missing drivers): start over with the
        // DirectX renderer, which falls back to software rendering. A fresh process is
        // needed because a second window event loop can't be created in this one.
        if let Ok(exe) = std::env::current_exe()
            && std::process::Command::new(exe)
                .args(std::env::args_os().skip(1))
                .env(FALLBACK_RENDERER_ENV, "1")
                .spawn()
                .is_ok()
        {
            return Ok(());
        }
    }
    result
}

/// Zoom that fits `page` whole in a `screen`-sized window, with a margin.
fn fit_zoom(page: doc::PageSize, screen: Vec2) -> f32 {
    let avail = (screen - Vec2::splat(2.0 * FIT_MARGIN)).max(Vec2::splat(50.0));
    (avail.x / page.w).min(avail.y / page.h)
}

/// The scroll position that frames page rect `r`: whole and centered if it fits on screen,
/// otherwise starting at its top.
fn rest_scroll(r: Rect, screen: Vec2) -> f32 {
    if r.height() + 2.0 * GAP <= screen.y {
        r.center().y - screen.y / 2.0
    } else {
        r.top() - GAP
    }
}

fn window_title(path: &std::path::Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{name} - {APP_NAME}")
}

/// A character position: (page, index into that page's text).
type CharPos = (usize, usize);

#[derive(Default, PartialEq)]
enum Drag {
    #[default]
    None,
    Pan,
    Select,
    /// Right/middle button held: the page keeps moving toward where the mouse is pushed,
    /// relative to this anchor, even while the mouse stays still.
    Joystick(Pos2),
}

#[derive(Default)]
struct Viewer {
    doc: Option<Document>,
    info: Option<DocInfo>,
    error: Option<String>,
    textures: HashMap<usize, (TextureHandle, f32)>,
    text: HashMap<usize, Vec<TextChar>>,
    fit: bool,
    fit_zoom: f32,
    /// Zoom picked with Ctrl+wheel; `None` means actual size.
    custom_zoom: Option<f32>,
    fit_screen: Vec2,
    /// Document coordinate shown at the top-left of the screen.
    scroll: Vec2,
    show_info: bool,
    show_hotkeys: bool,
    /// The page preview strip is opt-in (A), so it costs nothing unless wanted.
    show_thumbs: bool,
    thumbs: HashMap<usize, TextureHandle>,
    /// How far the preview strip is scrolled, in points.
    thumb_scroll: f32,
    /// The page the strip last scrolled to follow.
    thumb_followed: Option<usize>,
    fullscreen: bool,
    /// Page-by-page scrolling (Shift+S) instead of continuous.
    page_mode: bool,
    /// Where a page-by-page glide is heading (scroll.y).
    page_target: Option<f32>,
    /// Wheel travel not yet turned into a page-by-page step.
    page_wheel: f32,
    /// A short message at the bottom of the screen, and when it appeared.
    toast: Option<(&'static str, f64)>,
    /// (when the current run of wheel scrolling started, last wheel event, direction)
    wheel_streak: Option<(f64, f64, f32)>,
    /// (anchor, cursor)
    selection: Option<(CharPos, CharPos)>,
    drag: Drag,
    /// Velocity (points per second) of the glide after a quick pan.
    fling: Option<Vec2>,
}

/// Where the pages sit in document coordinates at a given zoom.
struct Layout {
    pages: Vec<Rect>,
    size: Vec2,
    /// Offset that centers a document smaller than the screen.
    center: Vec2,
}

impl Viewer {
    fn open(&mut self, path: PathBuf, ctx: &egui::Context, screen: Vec2) {
        ctx.send_viewport_cmd(ViewportCommand::Title(window_title(&path)));
        *self = Viewer {
            fullscreen: self.fullscreen,
            page_mode: self.page_mode,
            fit: true,
            show_thumbs: self.show_thumbs,
            show_info: self.show_info,
            show_hotkeys: self.show_hotkeys,
            ..Default::default()
        };
        let ppp = ctx.pixels_per_point();
        let first_page: doc::FirstPageScale = Box::new(move |page| fit_zoom(page, screen) * ppp);
        self.doc = Some(Document::open(path, ctx.clone(), Some(first_page)));
    }

    fn zoom(&self) -> f32 {
        if self.fit {
            self.fit_zoom
        } else {
            self.custom_zoom.unwrap_or(ACTUAL_SIZE)
        }
    }

    fn layout(&self, screen: Vec2) -> Layout {
        let zoom = self.zoom();
        let info = self.info.as_ref().unwrap();
        let width = info.pages.iter().map(|p| p.w).fold(0.0, f32::max) * zoom + 2.0 * GAP;
        let mut y = GAP;
        let pages = info
            .pages
            .iter()
            .map(|p| {
                let size = vec2(p.w, p.h) * zoom;
                let rect = Rect::from_min_size(pos2((width - size.x) / 2.0, y), size);
                y += size.y + GAP;
                rect
            })
            .collect();
        let size = vec2(width, y);
        Layout {
            pages,
            size,
            center: ((screen - size) / 2.0).max(Vec2::ZERO),
        }
    }

    fn clamp_scroll(&mut self, layout: &Layout, screen: Vec2) {
        let max = (layout.size - screen).max(Vec2::ZERO);
        self.scroll = self.scroll.clamp(Vec2::ZERO, max);
    }

    /// The page under the middle of the screen.
    fn current_page(&self, layout: &Layout, screen: Vec2) -> usize {
        let y = self.scroll.y + screen.y / 2.0 - layout.center.y;
        layout
            .pages
            .iter()
            .position(|r| y < r.bottom() + GAP / 2.0)
            .unwrap_or(layout.pages.len().saturating_sub(1))
    }

    fn compute_fit_zoom(&self, page: usize, screen: Vec2) -> f32 {
        fit_zoom(self.info.as_ref().unwrap().pages[page], screen)
    }

    /// Switch between fit-to-screen and actual size (which also drops any Ctrl+wheel zoom),
    /// keeping the reader's place.
    fn set_fit(&mut self, fit: bool, screen: Vec2) {
        let old = self.layout(screen);
        let page = self.current_page(&old, screen);

        self.fit = fit;
        self.custom_zoom = None;
        if fit {
            self.fit_zoom = self.compute_fit_zoom(page, screen);
            self.fit_screen = screen;
        }
        // Frame the page the reader was on.
        let new = self.layout(screen);
        let rect = new.pages[page];
        self.scroll = vec2(rect.center().x - screen.x / 2.0, rest_scroll(rect, screen));
        self.page_target = None;
        self.fling = None;
        self.clamp_scroll(&new, screen);
    }

    /// Space: back to the default view (whole page, no zoom) on the current page.
    fn reset_view(&mut self, screen: Vec2) {
        self.set_fit(true, screen);
    }

    /// Zoom by `factor`, keeping the document point under `mouse` where it is.
    fn zoom_at(&mut self, factor: f32, mouse: Pos2, screen: Rect) {
        let size = screen.size();
        let old = self.layout(size);
        let origin = screen.min + old.center - self.scroll;
        let Some((page, point)) = self.page_point(&old, origin, mouse) else {
            return;
        };

        let zoom = (self.zoom() * factor).clamp(MIN_ZOOM * ACTUAL_SIZE, MAX_ZOOM * ACTUAL_SIZE);
        self.fit = false;
        self.custom_zoom = Some(zoom);
        let new = self.layout(size);
        self.scroll =
            screen.min + new.center + new.pages[page].min.to_vec2() + point.to_vec2() * zoom
                - mouse;
        self.clamp_scroll(&new, size);
    }

    fn process_messages(&mut self, ctx: &egui::Context) {
        let Some(doc) = &self.doc else { return };
        while let Ok(msg) = doc.rx.try_recv() {
            match msg {
                Msg::Loaded(info) => {
                    if info.pages.is_empty() {
                        self.error = Some("This PDF has no pages.".into());
                    }
                    self.info = Some(info);
                }
                Msg::Failed(e) => self.error = Some(e),
                Msg::Rendered { job, image } if job.thumb => {
                    let tex = ctx.load_texture(
                        format!("thumb-{}", job.page),
                        image,
                        TextureOptions::LINEAR,
                    );
                    self.thumbs.insert(job.page, tex);
                }
                Msg::Rendered { job, image } => {
                    let tex = ctx.load_texture(
                        format!("page-{}", job.page),
                        image,
                        TextureOptions::LINEAR,
                    );
                    self.textures.insert(job.page, (tex, job.scale));
                }
                Msg::Text { page, chars } => {
                    self.text.insert(page, chars);
                }
            }
        }
    }

    /// Exact hit: the character under `p` (page points), if any.
    fn char_at(&self, page: usize, p: Pos2) -> Option<CharPos> {
        let chars = self.text.get(&page)?;
        chars
            .iter()
            .position(|c| c.rect.expand(1.0).contains(p))
            .map(|i| (page, i))
    }

    /// Closest character to `p`, preferring the same line.
    fn nearest_char(&self, page: usize, p: Pos2) -> Option<CharPos> {
        let chars = self.text.get(&page)?;
        let dist = |c: &TextChar| {
            let r = c.rect;
            let dy = (r.top() - p.y).max(p.y - r.bottom()).max(0.0);
            let dx = (r.left() - p.x).max(p.x - r.right()).max(0.0);
            dy * 1000.0 + dx
        };
        let (i, _) = chars
            .iter()
            .enumerate()
            .min_by(|a, b| dist(a.1).total_cmp(&dist(b.1)))?;
        Some((page, i))
    }

    /// Map a screen position to (page, point in page coordinates), snapping to the nearest page.
    fn page_point(&self, layout: &Layout, origin: Pos2, pos: Pos2) -> Option<(usize, Pos2)> {
        let doc = pos - origin;
        let page = layout
            .pages
            .iter()
            .position(|r| doc.y < r.bottom() + GAP / 2.0)
            .unwrap_or(layout.pages.len().checked_sub(1)?);
        let rect = layout.pages[page];
        Some((page, ((doc - rect.min.to_vec2()) / self.zoom()).to_pos2()))
    }

    fn selected_range(&self) -> Option<(CharPos, CharPos)> {
        let (a, b) = self.selection?;
        Some(if a <= b { (a, b) } else { (b, a) })
    }

    fn selected_text(&self) -> String {
        let Some((start, end)) = self.selected_range() else {
            return String::new();
        };
        let mut out = String::new();
        for page in start.0..=end.0 {
            let Some(chars) = self.text.get(&page) else {
                continue;
            };
            if chars.is_empty() {
                continue;
            }
            let from = if page == start.0 { start.1 } else { 0 };
            let to = if page == end.0 {
                end.1
            } else {
                chars.len() - 1
            };
            if page != start.0 && !out.is_empty() {
                out.push('\n');
            }
            let mut prev: Option<&TextChar> = None;
            for c in &chars[from..=to.min(chars.len() - 1)] {
                if let Some(p) = prev {
                    let mid = c.rect.center().y;
                    if mid < p.rect.top() || mid > p.rect.bottom() {
                        out.push('\n');
                    } else if c.rect.left() - p.rect.right() > p.rect.height() * 0.15
                        && !p.text.ends_with(char::is_whitespace)
                        && !c.text.starts_with(char::is_whitespace)
                    {
                        out.push(' ');
                    }
                }
                out.push_str(&c.text);
                prev = Some(c);
            }
        }
        out
    }

    /// Selection rectangles for one page, merged per line, in page points.
    fn selection_rects(&self, page: usize) -> Vec<Rect> {
        let Some((start, end)) = self.selected_range() else {
            return Vec::new();
        };
        let Some(chars) = self.text.get(&page) else {
            return Vec::new();
        };
        if page < start.0 || page > end.0 || chars.is_empty() {
            return Vec::new();
        }
        let from = if page == start.0 { start.1 } else { 0 };
        let to = if page == end.0 {
            end.1
        } else {
            chars.len() - 1
        };
        let mut rects: Vec<Rect> = Vec::new();
        for c in &chars[from..=to.min(chars.len() - 1)] {
            let r = c.rect;
            match rects.last_mut() {
                Some(last)
                    if r.left() >= last.left() - 1.0
                        && (last.bottom().min(r.bottom()) - last.top().max(r.top()))
                            > 0.5 * last.height().min(r.height()) =>
                {
                    *last = last.union(r)
                }
                _ => rects.push(r),
            }
        }
        rects
    }

    fn handle_keys(&mut self, ctx: &egui::Context, layout: &Layout, screen: Vec2) {
        let (pressed, copy) = ctx.input(|i| {
            let keys: Vec<(Key, bool)> = i
                .events
                .iter()
                .filter_map(|e| match e {
                    Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } if !modifiers.ctrl && !modifiers.alt => Some((*key, modifiers.shift)),
                    _ => None,
                })
                .collect();
            (keys, i.events.iter().any(|e| matches!(e, Event::Copy)))
        });

        if copy && self.selection.is_some() {
            ctx.copy_text(self.selected_text());
        }
        let page_step = (screen.y - 3.0 * LINE_SCROLL).max(LINE_SCROLL);
        for (key, shift) in pressed {
            match key {
                Key::S if !shift => self.set_fit(!self.fit, screen),
                Key::ArrowDown => self.scroll.y += LINE_SCROLL,
                Key::ArrowUp => self.scroll.y -= LINE_SCROLL,
                Key::ArrowRight => self.scroll.x += LINE_SCROLL,
                Key::ArrowLeft => self.scroll.x -= LINE_SCROLL,
                Key::PageDown if self.page_mode => self.page_step(true, layout, screen),
                Key::PageUp if self.page_mode => self.page_step(false, layout, screen),
                Key::PageDown => self.scroll.y += page_step,
                Key::PageUp => self.scroll.y -= page_step,
                Key::Space => self.reset_view(screen),
                Key::Home => self.scroll.y = 0.0,
                Key::End => self.scroll.y = layout.size.y,
                _ => {}
            }
        }
    }

    fn viewer_ui(&mut self, ui: &mut egui::Ui, screen: Rect) {
        let ctx = ui.ctx().clone();
        let size = screen.size();

        if self.fit && self.fit_zoom == 0.0 {
            // Freshly opened in fit mode: size the first page before anything asks which
            // page is current (a zero zoom would collapse every page onto the first row).
            self.fit_zoom = self.compute_fit_zoom(0, size);
            self.fit_screen = Vec2::ZERO;
        }
        if self.fit && size != self.fit_screen {
            // The window changed size (e.g. while entering fullscreen); refit.
            self.set_fit(true, size);
        }

        let layout = self.layout(size);
        self.handle_keys(&ctx, &layout, size);
        // Ctrl+wheel (and pinch) arrive as a zoom factor instead of a scroll.
        let (zoom_delta, mouse) = ctx.input(|i| (i.zoom_delta(), i.pointer.hover_pos()));
        if zoom_delta != 1.0 {
            self.zoom_at(zoom_delta, mouse.unwrap_or(screen.center()), screen);
        }
        let layout = self.layout(size);

        let over_strip =
            self.strip_shown() && mouse.is_some_and(|p| p.x < screen.left() + STRIP_WIDTH);
        let wheel = ctx.input(|i| i.smooth_scroll_delta) * self.wheel_acceleration(&ctx);
        if over_strip {
            self.thumb_scroll -= wheel.y;
        } else if self.page_mode {
            self.scroll.x -= wheel.x;
            self.page_wheel(&ctx, &layout, size);
        } else {
            self.scroll -= wheel;
        }
        if ctx.input(|i| i.pointer.any_pressed()) {
            self.page_target = None;
        }
        if let Some(target) = self.page_target {
            // Ease out toward the target page.
            let dt = ctx.input(|i| i.stable_dt).min(0.05);
            self.scroll.y += (target - self.scroll.y) * (1.0 - (-dt / PAGE_GLIDE_TIME).exp());
            if (target - self.scroll.y).abs() < 0.5 {
                self.scroll.y = target;
                self.page_target = None;
            }
            ctx.request_repaint();
        }

        // Any new input takes over from a glide.
        if wheel != Vec2::ZERO
            || zoom_delta != 1.0
            || ctx.input(|i| {
                i.pointer.any_pressed()
                    || i.events
                        .iter()
                        .any(|e| matches!(e, Event::Key { pressed: true, .. }))
            })
        {
            self.fling = None;
        }
        if let Some(velocity) = self.fling {
            let dt = ctx.input(|i| i.stable_dt).min(0.05);
            let decay = (-dt / FLING_TIME_CONSTANT).exp();
            let wanted = self.scroll - velocity * FLING_TIME_CONSTANT * (1.0 - decay);
            self.scroll = wanted;
            self.clamp_scroll(&layout, size);
            // Stop along an axis once the glide runs into the edge of the document.
            let mut velocity = velocity * decay;
            if self.scroll.x != wanted.x {
                velocity.x = 0.0;
            }
            if self.scroll.y != wanted.y {
                velocity.y = 0.0;
            }
            self.fling = (velocity.length() > 10.0).then_some(velocity);
            ctx.request_repaint();
        }

        // Pointer: drag on text selects it, drag anywhere else (or with any other button) pans.
        let response = ui.interact(
            screen,
            ui.scope_id().with("canvas"),
            Sense::click_and_drag(),
        );
        self.clamp_scroll(&layout, size);
        let origin = screen.min + layout.center - self.scroll;

        if response.drag_started() {
            let press = ctx
                .input(|i| i.pointer.press_origin())
                .or(response.interact_pointer_pos());
            let hit = press
                .filter(|_| response.drag_started_by(PointerButton::Primary))
                .and_then(|p| self.page_point(&layout, origin, p))
                .and_then(|(page, p)| self.char_at(page, p));
            let joystick = response.drag_started_by(PointerButton::Secondary)
                || response.drag_started_by(PointerButton::Middle);
            match (hit, press) {
                (_, Some(anchor)) if joystick => self.drag = Drag::Joystick(anchor),
                (Some(hit), _) => {
                    self.drag = Drag::Select;
                    self.selection = Some((hit, hit));
                }
                _ => self.drag = Drag::Pan,
            }
        }
        if response.dragged() {
            match self.drag {
                Drag::Pan => self.scroll -= response.drag_delta(),
                Drag::Joystick(anchor) => {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let offset = pos - anchor;
                        let push = offset.length() - JOYSTICK_DEAD_ZONE;
                        if push > 0.0 {
                            let speed =
                                (JOYSTICK_GAIN * push.powf(JOYSTICK_CURVE)).min(JOYSTICK_MAX_SPEED);
                            let dt = ctx.input(|i| i.stable_dt).min(0.05);
                            self.scroll += offset.normalized() * speed * dt;
                        }
                    }
                    // Keep moving even when the mouse doesn't.
                    ctx.request_repaint();
                }
                Drag::Select => {
                    if let Some(pos) = response.interact_pointer_pos() {
                        // Scroll when dragging a selection past the screen edge.
                        let edge = 40.0;
                        let push = if pos.y < screen.top() + edge {
                            pos.y - (screen.top() + edge)
                        } else if pos.y > screen.bottom() - edge {
                            pos.y - (screen.bottom() - edge)
                        } else {
                            0.0
                        };
                        if push != 0.0 {
                            self.scroll.y += push * 0.5;
                            ctx.request_repaint();
                        }
                        if let Some(end) = self
                            .page_point(&layout, origin, pos)
                            .and_then(|(page, p)| self.nearest_char(page, p))
                            && let Some(sel) = &mut self.selection
                        {
                            sel.1 = end;
                        }
                    }
                }
                Drag::None => {}
            }
        }
        if response.drag_stopped() {
            if self.drag == Drag::Pan {
                let velocity = ctx.input(|i| i.pointer.velocity());
                if velocity.length() > MIN_FLING_SPEED {
                    self.fling = Some(velocity * (MAX_FLING_SPEED / velocity.length()).min(1.0));
                    ctx.request_repaint();
                }
            }
            self.drag = Drag::None;
        }
        if response.clicked() {
            self.selection = None;
        }
        self.clamp_scroll(&layout, size);
        let origin = screen.min + layout.center - self.scroll;

        let hover_text = self.drag == Drag::None
            && response
                .hover_pos()
                .and_then(|p| self.page_point(&layout, origin, p))
                .and_then(|(page, p)| self.char_at(page, p))
                .is_some();
        ctx.set_cursor_icon(match self.drag {
            Drag::Pan => CursorIcon::Grabbing,
            Drag::Joystick(_) => CursorIcon::AllScroll,
            Drag::Select => CursorIcon::Text,
            Drag::None if hover_text => CursorIcon::Text,
            Drag::None => CursorIcon::Default,
        });

        let mut jobs = self.paint_pages(ui, &layout, origin, screen);
        if self.strip_shown() {
            let current = self.current_page(&layout, size);
            jobs.extend(self.thumbnail_strip(&ctx, screen, current, over_strip));
        }
        if let Some(doc) = &self.doc {
            doc.set_jobs(jobs);
        }

        if let Drag::Joystick(anchor) = self.drag {
            // Show where the joystick is anchored and which way it's pushing.
            let painter = ui.painter();
            if let Some(pos) = response.interact_pointer_pos() {
                painter.line_segment([anchor, pos], (2.0, Color32::from_black_alpha(140)));
            }
            painter.circle(
                anchor,
                6.0,
                Color32::from_black_alpha(140),
                (1.5, Color32::WHITE),
            );
        }

        if self.show_info {
            let current = self.current_page(&layout, size);
            self.info_panel(&ctx, screen, current);
        }
    }

    /// Draw the visible pages; returns the render jobs they need, most urgent first.
    fn paint_pages(
        &mut self,
        ui: &egui::Ui,
        layout: &Layout,
        origin: Pos2,
        screen: Rect,
    ) -> Vec<Job> {
        let painter = ui.painter();
        let ppp = ui.ctx().pixels_per_point();
        let max_side = (ui.ctx().input(|i| i.max_texture_side) as f32).min(doc::MAX_TEXTURE_SIDE);
        let zoom = self.zoom();
        let prefetch = screen.expand2(vec2(0.0, screen.height()));

        let mut visible = Vec::new();
        let mut nearby = Vec::new();
        for (i, rect) in layout.pages.iter().enumerate() {
            let rect = rect.translate(origin.to_vec2());
            if rect.intersects(screen) {
                visible.push(i);
            } else if rect.intersects(prefetch) {
                nearby.push(i);
            }
        }

        let info = self.info.as_ref().unwrap();
        let scale_for = |i: usize| doc::render_scale(info.pages[i], zoom * ppp, max_side);

        let center_y = screen.center().y;
        visible.sort_by_key(|&i| {
            let r = layout.pages[i].translate(origin.to_vec2());
            (r.center().y - center_y).abs() as i64
        });
        let jobs: Vec<Job> = visible
            .iter()
            .chain(&nearby)
            .map(|&i| Job {
                page: i,
                scale: scale_for(i),
                thumb: false,
            })
            .filter(|job| {
                self.textures
                    .get(&job.page)
                    .is_none_or(|(_, s)| *s != job.scale)
            })
            .collect();

        // Drop textures for pages far away to keep memory in check.
        let keep_lo = visible
            .iter()
            .chain(&nearby)
            .min()
            .copied()
            .unwrap_or(0)
            .saturating_sub(2);
        let keep_hi = visible.iter().chain(&nearby).max().copied().unwrap_or(0) + 2;
        self.textures.retain(|&i, _| i >= keep_lo && i <= keep_hi);

        for &i in &visible {
            let rect = layout.pages[i].translate(origin.to_vec2());
            painter.rect_filled(
                rect.translate(vec2(0.0, 2.0)).expand(2.0),
                2.0,
                Color32::from_black_alpha(60),
            );
            match self.textures.get(&i) {
                Some((tex, _)) => {
                    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
                    painter.image(tex.id(), rect, uv, Color32::WHITE);
                }
                None => {
                    painter.rect_filled(rect, 0.0, Color32::WHITE);
                }
            }
            for r in self.selection_rects(i) {
                let r = Rect::from_min_max(
                    rect.min + r.min.to_vec2() * zoom,
                    rect.min + r.max.to_vec2() * zoom,
                );
                painter.rect_filled(r, 0.0, SELECTION);
            }
        }
        jobs
    }

    /// How much faster than normal the wheel should scroll right now.
    fn wheel_acceleration(&mut self, ctx: &egui::Context) -> f32 {
        let (now, raw) = ctx.input(|i| {
            let raw = i.events.iter().fold(Vec2::ZERO, |sum, e| match e {
                Event::MouseWheel {
                    delta, modifiers, ..
                } if !modifiers.ctrl => sum + *delta,
                _ => sum,
            });
            (i.time, raw)
        });
        let direction = if raw.y != 0.0 {
            raw.y.signum()
        } else {
            raw.x.signum()
        };
        if raw != Vec2::ZERO {
            self.wheel_streak = match self.wheel_streak {
                Some((start, last, dir)) if now - last < WHEEL_STREAK_GAP && dir == direction => {
                    Some((start, now, dir))
                }
                _ => Some((now, now, direction)),
            };
        }
        match self.wheel_streak {
            Some((start, last, _)) if now - last < WHEEL_STREAK_GAP => {
                let t =
                    ((now - start - WHEEL_ACCEL_DELAY) / WHEEL_ACCEL_RAMP).clamp(0.0, 1.0) as f32;
                // Ease in, so the speed-up feels smooth rather than a sudden jump.
                1.0 + (WHEEL_MAX_ACCEL - 1.0) * t * t
            }
            _ => 1.0,
        }
    }

    /// Page-by-page mode: turn wheel notches into steps.
    fn page_wheel(&mut self, ctx: &egui::Context, layout: &Layout, screen: Vec2) {
        // Positive y means the content moves down, i.e. scrolling up.
        let travel: f32 = ctx.input(|i| {
            i.events
                .iter()
                .map(|e| match e {
                    Event::MouseWheel {
                        unit,
                        delta,
                        modifiers,
                        ..
                    } if !modifiers.ctrl && !modifiers.shift => match unit {
                        egui::MouseWheelUnit::Point => delta.y,
                        egui::MouseWheelUnit::Line => delta.y * PAGE_WHEEL_STEP,
                        egui::MouseWheelUnit::Page => delta.y * screen.y,
                    },
                    _ => 0.0,
                })
                .sum()
        });
        self.page_wheel += travel;
        while self.page_wheel.abs() >= PAGE_WHEEL_STEP {
            let down = self.page_wheel < 0.0;
            self.page_wheel -= PAGE_WHEEL_STEP.copysign(self.page_wheel);
            self.page_step(down, layout, screen);
        }
    }

    /// One page-by-page step. Pages that fit on screen are shown whole and centered; a taller
    /// page is scrolled through first, and the step after reaching its edge moves on.
    fn page_step(&mut self, down: bool, layout: &Layout, screen: Vec2) {
        let y = self.page_target.unwrap_or(self.scroll.y);
        let rest = |r: Rect| rest_scroll(r, screen);
        let current = layout
            .pages
            .iter()
            .position(|r| y + screen.y / 2.0 < r.bottom() + GAP / 2.0)
            .unwrap_or(layout.pages.len() - 1);
        let r = layout.pages[current];
        let target = if down {
            if r.bottom() + GAP > y + screen.y + 0.5 {
                (y + screen.y * PAGE_SCROLL_FRACTION).min(r.bottom() + GAP - screen.y)
            } else if let Some(&next) = layout.pages.get(current + 1) {
                rest(next)
            } else {
                y
            }
        } else if rest(r) < y - 0.5 {
            (y - screen.y * PAGE_SCROLL_FRACTION).max(rest(r))
        } else if current > 0 {
            let prev = layout.pages[current - 1];
            if prev.height() + 2.0 * GAP <= screen.y {
                rest(prev)
            } else {
                // Coming back up into a tall page: show its end first.
                prev.bottom() + GAP - screen.y
            }
        } else {
            y
        };
        let max = (layout.size.y - screen.y).max(0.0);
        self.page_target = Some(target.clamp(0.0, max));
        self.fling = None;
    }

    fn strip_shown(&self) -> bool {
        self.show_thumbs && self.info.as_ref().is_some_and(|i| i.pages.len() > 1)
    }

    /// Bring `page` to the top of the screen.
    fn go_to_page(&mut self, page: usize, screen: Vec2) {
        let layout = self.layout(screen);
        self.scroll.y = layout.pages[page].top() - GAP;
        self.fling = None;
        self.clamp_scroll(&layout, screen);
    }

    /// Where each preview sits in the strip, in strip coordinates (before scrolling).
    fn thumb_rects(&self) -> (Vec<Rect>, f32) {
        let mut y = THUMB_PAD;
        let rects = self
            .info
            .as_ref()
            .unwrap()
            .pages
            .iter()
            .map(|p| {
                let size = vec2(p.w, p.h) * (THUMB_WIDTH / p.w).min(THUMB_MAX_HEIGHT / p.h);
                let rect = Rect::from_min_size(pos2((STRIP_WIDTH - size.x) / 2.0, y), size);
                y += size.y + THUMB_LABEL + THUMB_GAP;
                rect
            })
            .collect();
        (rects, y + THUMB_PAD - THUMB_GAP)
    }

    /// The page preview strip down the left edge. Returns the previews that need rendering.
    fn thumbnail_strip(
        &mut self,
        ctx: &egui::Context,
        screen: Rect,
        current: usize,
        hovered: bool,
    ) -> Vec<Job> {
        let strip = Rect::from_min_size(screen.min, vec2(STRIP_WIDTH, screen.height()));
        let (rects, total) = self.thumb_rects();

        // Follow along as the reader moves through the document, unless they're browsing the strip.
        if self.thumb_followed != Some(current) && !hovered {
            let r = rects[current];
            if r.top() < self.thumb_scroll
                || r.bottom() + THUMB_LABEL > self.thumb_scroll + strip.height()
            {
                self.thumb_scroll = r.center().y - strip.height() / 2.0;
            }
        }
        self.thumb_followed = Some(current);
        self.thumb_scroll = self
            .thumb_scroll
            .clamp(0.0, (total - strip.height()).max(0.0));

        let fade = ctx.animate_bool_with_time(egui::Id::unique("thumb-strip-fade"), hovered, 0.15);
        let ppp = ctx.pixels_per_point();
        let offset = strip.min.to_vec2() - vec2(0.0, self.thumb_scroll);
        let visible: Vec<usize> = (0..rects.len())
            .filter(|&i| {
                rects[i]
                    .translate(offset)
                    .expand2(vec2(0.0, THUMB_LABEL))
                    .intersects(strip)
            })
            .collect();

        let mut clicked = None;
        let mut dragged = 0.0;
        egui::Area::new(egui::Id::unique("thumb-strip"))
            .fixed_pos(strip.min)
            .movable(false)
            .show(ctx, |ui| {
                let (_, response) = ui.allocate_exact_size(strip.size(), Sense::click_and_drag());
                ui.multiply_opacity(STRIP_FAINT + (1.0 - STRIP_FAINT) * fade);
                let painter = ui.painter_at(strip);
                painter.rect_filled(strip, 0.0, Color32::from_black_alpha(110));

                let pointer = response.hover_pos();
                for &i in &visible {
                    let rect = rects[i].translate(offset);
                    painter.rect_filled(
                        rect.translate(vec2(0.0, 1.0)).expand(1.0),
                        1.0,
                        Color32::from_black_alpha(80),
                    );
                    match self.thumbs.get(&i) {
                        Some(tex) => {
                            let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
                            painter.image(tex.id(), rect, uv, Color32::WHITE);
                        }
                        None => {
                            painter.rect_filled(rect, 0.0, Color32::WHITE);
                        }
                    }
                    let hot = pointer.is_some_and(|p| rect.expand(THUMB_GAP / 2.0).contains(p));
                    if i == current {
                        painter.rect_stroke(
                            rect.expand(2.0),
                            2.0,
                            (2.5, ACCENT),
                            egui::StrokeKind::Outside,
                        );
                    } else if hot {
                        painter.rect_stroke(
                            rect.expand(2.0),
                            2.0,
                            (1.5, Color32::from_gray(200)),
                            egui::StrokeKind::Outside,
                        );
                    }
                    painter.text(
                        pos2(strip.center().x, rect.bottom() + 3.0),
                        Align2::CENTER_TOP,
                        (i + 1).to_string(),
                        FontId::proportional(11.0),
                        if i == current {
                            ACCENT
                        } else {
                            Color32::from_gray(200)
                        },
                    );
                    if hot && response.clicked() {
                        clicked = Some(i);
                    }
                }
                if response.dragged() {
                    dragged = response.drag_delta().y;
                }
            });
        self.thumb_scroll -= dragged;
        if let Some(page) = clicked {
            self.go_to_page(page, screen.size());
        }

        // Keep previews near the visible part of the strip; drop the rest.
        let lo = visible.first().copied().unwrap_or(0).saturating_sub(20);
        let hi = visible.last().copied().unwrap_or(0) + 20;
        self.thumbs.retain(|&i, _| i >= lo && i <= hi);

        let info = self.info.as_ref().unwrap();
        visible
            .into_iter()
            .filter(|i| !self.thumbs.contains_key(i))
            .map(|i| Job {
                page: i,
                scale: doc::render_scale(
                    info.pages[i],
                    rects[i].width() * ppp / info.pages[i].w,
                    doc::MAX_TEXTURE_SIDE,
                ),
                thumb: true,
            })
            .collect()
    }

    fn info_panel(&self, ctx: &egui::Context, screen: Rect, current: usize) {
        let info = self.info.as_ref().unwrap();
        let path = &self.doc.as_ref().unwrap().path;
        let page = info.pages[current];

        let mut rows: Vec<(&str, String)> = vec![
            (
                "File",
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
            (
                "Location",
                path.parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            ),
            ("File size", format_bytes(info.file_size)),
            (
                "Pages",
                format!("{} (viewing {})", info.pages.len(), current + 1),
            ),
            (
                "Page size",
                format!(
                    "{:.0} × {:.0} pt\n{:.0} × {:.0} mm · {:.2} × {:.2} in{}",
                    page.w,
                    page.h,
                    page.w / 72.0 * 25.4,
                    page.h / 72.0 * 25.4,
                    page.w / 72.0,
                    page.h / 72.0,
                    paper_name(page.w, page.h)
                        .map(|n| format!(" · {n}"))
                        .unwrap_or_default()
                ),
            ),
            (
                "Zoom",
                match (self.fit, self.custom_zoom) {
                    (true, _) => format!(
                        "Fit to screen ({:.0}%)",
                        self.fit_zoom / ACTUAL_SIZE * 100.0
                    ),
                    (false, Some(z)) => format!("{:.0}%", z / ACTUAL_SIZE * 100.0),
                    (false, None) => "Actual size (100%)".into(),
                },
            ),
            (
                "Scrolling",
                if self.page_mode {
                    "Page by page"
                } else {
                    "Continuous"
                }
                .into(),
            ),
            ("PDF version", info.version.clone()),
        ];
        rows.extend(info.meta.iter().map(|(k, v)| (*k, v.clone())));

        egui::Area::new(egui::Id::unique("info-panel"))
            .default_pos(pos2(screen.right() - 470.0, screen.top() + 20.0))
            .movable(true)
            .show(ctx, |ui| {
                panel_frame().show(ui, |ui| {
                    ui.set_max_width(430.0);
                    egui::Grid::new("info-grid")
                        .num_columns(2)
                        .spacing(vec2(16.0, 6.0))
                        .show(ui, |ui| {
                            for (label, value) in rows {
                                ui.label(egui::RichText::new(label).color(Color32::from_gray(150)));
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(value).color(Color32::from_gray(235)),
                                    )
                                    .wrap(),
                                );
                                ui.end_row();
                            }
                        });
                });
            });
    }
}

impl eframe::App for Viewer {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let screen = ui.max_rect();
        ui.painter().rect_filled(screen, 0.0, BACKGROUND);

        if ctx.input(|i| {
            i.key_pressed(Key::Escape) || (i.key_pressed(Key::X) && !i.modifiers.command)
        }) {
            // Nothing to save, so skip the orderly teardown and vanish immediately.
            std::process::exit(0);
        }
        if ctx.input(|i| i.key_pressed(Key::I) && !i.modifiers.ctrl) {
            self.show_info = !self.show_info;
        }
        if ctx.input(|i| i.key_pressed(Key::H) && !i.modifiers.ctrl) {
            self.show_hotkeys = !self.show_hotkeys;
        }
        // Check the key event's own modifiers: a quick Shift+S can release Shift within
        // the same frame.
        if ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(e, Event::Key { key: Key::S, pressed: true, modifiers, .. }
                    if modifiers.shift && !modifiers.ctrl)
            })
        }) {
            self.page_mode = !self.page_mode;
            self.page_target = None;
            self.page_wheel = 0.0;
            let label = if self.page_mode {
                "Page by page"
            } else {
                "Continuous scrolling"
            };
            self.toast = Some((label, ctx.input(|i| i.time)));
        }
        if ctx.input(|i| i.key_pressed(Key::A) && !i.modifiers.ctrl) {
            self.show_thumbs = !self.show_thumbs;
        }
        if ctx.input(|i| i.key_pressed(Key::F) && !i.modifiers.ctrl) {
            self.fullscreen = !self.fullscreen;
            platform::set_fullscreen(&ctx, self.fullscreen);
        }
        if let Some(path) =
            ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()))
        {
            self.open(path, &ctx, screen.size());
        }

        self.process_messages(&ctx);

        let message = if let Some(e) = &self.error {
            Some(e.clone())
        } else if self.doc.is_none() {
            Some("Drop a PDF here.\nH shows the hotkeys.".into())
        } else if self.info.is_none() {
            Some("Opening…".into())
        } else {
            None
        };
        match message {
            Some(text) => {
                ui.painter().text(
                    screen.center(),
                    Align2::CENTER_CENTER,
                    text,
                    FontId::proportional(20.0),
                    Color32::from_gray(220),
                );
            }
            None => self.viewer_ui(ui, screen),
        }

        if self.show_hotkeys {
            hotkey_panel(&ctx, screen);
        }
        if let Some((text, since)) = self.toast {
            let age = ctx.input(|i| i.time) - since;
            if age > TOAST_TIME {
                self.toast = None;
            } else {
                let alpha = ((TOAST_TIME - age) / 0.3).min(1.0) as f32;
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::unique("toast"),
                ));
                let galley = painter.layout_no_wrap(
                    text.into(),
                    FontId::proportional(18.0),
                    Color32::from_gray(235).gamma_multiply(alpha),
                );
                let rect = Rect::from_center_size(
                    pos2(screen.center().x, screen.bottom() - 60.0),
                    galley.size() + vec2(32.0, 18.0),
                );
                painter.rect_filled(
                    rect,
                    8.0,
                    Color32::from_rgba_premultiplied(24, 26, 28, 250).gamma_multiply(alpha),
                );
                painter.galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
                ctx.request_repaint();
            }
        }

        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            ui.painter()
                .rect_filled(screen, 0.0, Color32::from_black_alpha(120));
            ui.painter().text(
                screen.center(),
                Align2::CENTER_CENTER,
                "Drop to open",
                FontId::proportional(28.0),
                Color32::WHITE,
            );
        }
    }

    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        BACKGROUND.to_normalized_gamma_f32()
    }
}

fn hotkey_panel(ctx: &egui::Context, screen: Rect) {
    egui::Area::new(egui::Id::unique("hotkey-panel"))
        .default_pos(pos2(screen.center().x - 200.0, screen.top() + 80.0))
        .movable(true)
        .show(ctx, |ui| {
            panel_frame().show(ui, |ui| {
                ui.label(
                    egui::RichText::new("Hotkeys")
                        .size(18.0)
                        .color(Color32::from_gray(235)),
                );
                ui.add_space(8.0);
                egui::Grid::new("hotkey-grid")
                    .num_columns(2)
                    .spacing(vec2(24.0, 6.0))
                    .show(ui, |ui| {
                        for (keys, action) in HOTKEYS {
                            ui.label(
                                egui::RichText::new(*keys)
                                    .monospace()
                                    .color(Color32::from_gray(235)),
                            );
                            ui.label(egui::RichText::new(*action).color(Color32::from_gray(170)));
                            ui.end_row();
                        }
                    });
            });
        });
}

/// The dark floating card used by the info and hotkey panels.
fn panel_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgba_premultiplied(24, 26, 28, 250))
        .corner_radius(8.0)
        .inner_margin(16.0)
}

fn format_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    let human = if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", b / (1024.0 * 1024.0 * 1024.0))
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.2} MB", b / (1024.0 * 1024.0))
    } else if b >= 1024.0 {
        format!("{:.1} KB", b / 1024.0)
    } else {
        return format!("{bytes} bytes");
    };
    format!("{human} ({bytes} bytes)")
}

fn paper_name(w: f32, h: f32) -> Option<String> {
    const PAPERS: [(&str, f32, f32); 8] = [
        ("A3", 842.0, 1191.0),
        ("A4", 595.0, 842.0),
        ("A5", 420.0, 595.0),
        ("B5", 499.0, 709.0),
        ("Letter", 612.0, 792.0),
        ("Legal", 612.0, 1008.0),
        ("Tabloid", 792.0, 1224.0),
        ("Executive", 522.0, 756.0),
    ];
    let (short, long) = (w.min(h), w.max(h));
    PAPERS
        .iter()
        .find(|(_, a, b)| (short - a).abs() < 3.0 && (long - b).abs() < 3.0)
        .map(|(name, ..)| {
            let orientation = if w > h { "landscape" } else { "portrait" };
            format!("{name} {orientation}")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn extracts_selects_and_copies_text() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/sample.pdf");
        let ctx = egui::Context::default();
        let mut viewer = Viewer {
            doc: Some(Document::open(path, ctx.clone(), None)),
            ..Default::default()
        };
        viewer.doc.as_ref().unwrap().set_jobs(vec![Job {
            page: 0,
            scale: 1.0,
            thumb: false,
        }]);

        for _ in 0..100 {
            viewer.process_messages(&ctx);
            if viewer.text.contains_key(&0) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let info = viewer.info.as_ref().expect("loaded");
        assert_eq!(info.pages.len(), 3);
        assert_eq!((info.pages[1].w, info.pages[1].h), (842.0, 595.0));
        assert!(
            info.meta
                .iter()
                .any(|(k, v)| *k == "Title" && v == "Viewer test")
        );
        let (_, scale) = viewer.textures[&0];
        assert_eq!(scale, 1.0);

        // "Page one..." has its baseline at y = 842 - 140 = 702 from the bottom, i.e. 140 from the top.
        let first = viewer.char_at(0, pos2(63.0, 136.0)).expect("hit on 'P'");
        assert_eq!(viewer.text[&0][first.1].text, "P");
        assert!(viewer.char_at(0, pos2(500.0, 600.0)).is_none());

        // Drag from "Page one" to the end of the second line.
        let end = viewer.nearest_char(0, pos2(590.0, 160.0)).unwrap();
        viewer.selection = Some((end, first));
        assert_eq!(
            viewer.selected_text(),
            "Page one. The quick brown fox jumps over the lazy dog.
Second line of text, to try selecting across lines."
        );
        assert!(!viewer.selection_rects(0).is_empty());
        assert_eq!(viewer.selection_rects(0).len(), 2);
    }

    #[test]
    fn ctrl_zoom_keeps_point_under_mouse_and_s_resets_it() {
        let pages = vec![doc::PageSize { w: 595.0, h: 842.0 }; 5];
        let mut viewer = Viewer {
            info: Some(DocInfo {
                pages,
                file_size: 0,
                version: "1.7".into(),
                meta: Vec::new(),
            }),
            ..Default::default()
        };
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1700.0, 1000.0));
        viewer.scroll = vec2(0.0, 1500.0);
        let mouse = pos2(900.0, 400.0);
        let under_mouse = |v: &Viewer| {
            let layout = v.layout(screen.size());
            let origin = screen.min + layout.center - v.scroll;
            v.page_point(&layout, origin, mouse).unwrap()
        };

        let before = under_mouse(&viewer);
        viewer.zoom_at(3.0, mouse, screen); // wide enough that the pages overflow the screen
        let after = under_mouse(&viewer);
        assert_eq!(before.0, after.0);
        assert!(
            (before.1 - after.1).length() < 0.01,
            "{before:?} vs {after:?}"
        );
        assert!((viewer.zoom() - ACTUAL_SIZE * 3.0).abs() < 1e-4);

        viewer.set_fit(true, screen.size());
        assert!(viewer.fit && viewer.custom_zoom.is_none());
        viewer.set_fit(false, screen.size());
        assert_eq!(viewer.zoom(), ACTUAL_SIZE);

        viewer.zoom_at(3.0, mouse, screen);
        let page = viewer.current_page(&viewer.layout(screen.size()), screen.size());
        viewer.reset_view(screen.size());
        assert!(viewer.fit && viewer.custom_zoom.is_none());
        // Back on the same page, framed whole rather than between two pages.
        let layout = viewer.layout(screen.size());
        assert_eq!(viewer.current_page(&layout, screen.size()), page);
        assert_eq!(
            viewer.scroll.y,
            rest_scroll(layout.pages[page], screen.size()).max(0.0)
        );
    }

    #[test]
    fn page_mode_steps_through_tall_pages_then_snaps() {
        // Two A4 pages at actual size are taller than this 1000-point screen; then a
        // landscape page that fits.
        let pages = vec![
            doc::PageSize { w: 595.0, h: 842.0 },
            doc::PageSize { w: 595.0, h: 842.0 },
            doc::PageSize { w: 842.0, h: 595.0 },
        ];
        let mut viewer = Viewer {
            info: Some(DocInfo {
                pages,
                file_size: 0,
                version: "1.7".into(),
                meta: Vec::new(),
            }),
            ..Default::default()
        };
        let screen = vec2(1700.0, 1000.0);
        let layout = viewer.layout(screen);
        let step = |v: &mut Viewer, down: bool| {
            v.page_step(down, &layout, screen);
            v.scroll.y = v.page_target.take().unwrap();
            v.scroll.y
        };
        let p = &layout.pages;
        // Down: finish page 1, then snap to page 2's top, finish it, then page 3 (centered,
        // but the document ends first, so it stops at the end).
        assert_eq!(step(&mut viewer, true), p[0].bottom() + GAP - 1000.0);
        assert_eq!(step(&mut viewer, true), p[1].top() - GAP);
        assert_eq!(step(&mut viewer, true), p[1].bottom() + GAP - 1000.0);
        assert_eq!(step(&mut viewer, true), layout.size.y - 1000.0);
        assert_eq!(step(&mut viewer, true), layout.size.y - 1000.0);
        // Back up: into the end of page 2, then its top.
        assert_eq!(step(&mut viewer, false), p[1].bottom() + GAP - 1000.0);
        assert_eq!(step(&mut viewer, false), p[1].top() - GAP);
    }

    #[test]
    fn names_paper_sizes() {
        assert_eq!(paper_name(595.0, 842.0).as_deref(), Some("A4 portrait"));
        assert_eq!(
            paper_name(792.0, 612.0).as_deref(),
            Some("Letter landscape")
        );
        assert_eq!(paper_name(100.0, 100.0), None);
    }
}
