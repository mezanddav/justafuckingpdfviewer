//! Everything that touches the PDF lives on one background thread: loading, rasterizing
//! pages with hayro and extracting text positions for selection. The UI talks to it
//! through a shared job queue and gets results back over a channel.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use eframe::egui::{self, ColorImage, Rect, pos2};
use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::hayro_syntax::object::DateTime;
use hayro::hayro_interpret::hayro_syntax::{Pdf, PdfData};
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, SoftMask, interpret_page,
};
use hayro::kurbo::{self, BezPath, Point, Shape};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

/// Size of one page in PDF points, after rotation.
#[derive(Clone, Copy)]
pub struct PageSize {
    pub w: f32,
    pub h: f32,
}

pub struct DocInfo {
    pub pages: Vec<PageSize>,
    pub file_size: u64,
    pub version: String,
    /// Document information dictionary entries that are present, in display order.
    pub meta: Vec<(&'static str, String)>,
}

/// One character on a page, with its box in page points (origin top-left).
pub struct TextChar {
    pub rect: Rect,
    pub text: String,
}

pub enum Msg {
    Loaded(DocInfo),
    Failed(String),
    Rendered { job: Job, image: ColorImage },
    Text { page: usize, chars: Vec<TextChar> },
}

/// Render `page` at `scale` physical pixels per PDF point, either for the page itself or
/// for its preview in the thumbnail strip.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Job {
    pub page: usize,
    pub scale: f32,
    pub thumb: bool,
}

/// Largest texture we ask the GPU for.
pub const MAX_TEXTURE_SIDE: f32 = 16384.0;

/// The rasterization scale (pixels per PDF point) for showing `page` at `px_per_pt`, limited
/// so the bitmap stays within texture limits and ~64 megapixels.
pub fn render_scale(page: PageSize, px_per_pt: f32, max_side: f32) -> f32 {
    let s = px_per_pt
        .min(max_side / page.w.max(page.h))
        .min((64.0e6 / (page.w * page.h)).sqrt());
    (s * 100.0).round() / 100.0
}

/// Pages render on this many threads at once, each with its own parsed copy of the file.
const WORKERS: usize = 3;

#[derive(Default)]
struct Queue {
    jobs: Vec<Job>,
    rendering: Vec<Job>,
    /// Pages whose text has been (or is being) extracted.
    text_claimed: HashSet<usize>,
    quit: bool,
}

type Shared = Arc<(Mutex<Queue>, Condvar)>;

pub struct Document {
    pub path: PathBuf,
    pub rx: Receiver<Msg>,
    shared: Shared,
}

/// Picks the resolution (pixels per PDF point) to render page 1 at, given its size.
pub type FirstPageScale = Box<dyn FnOnce(PageSize) -> f32 + Send>;

impl Document {
    /// Start loading `path` in the background. With `first_page_px_per_pt`, page 1 is
    /// rendered at that resolution as soon as the file is parsed, before the UI asks for it.
    pub fn open(
        path: PathBuf,
        ctx: egui::Context,
        first_page_px_per_pt: Option<FirstPageScale>,
    ) -> Self {
        let (tx, rx) = channel();
        let shared = Shared::default();
        let worker_shared = shared.clone();
        let worker_path = path.clone();
        std::thread::spawn(move || load(worker_path, worker_shared, tx, ctx, first_page_px_per_pt));
        Self { path, rx, shared }
    }

    /// Replace the pending work with `jobs` (highest priority first). Jobs that are
    /// already being rendered are not queued again.
    pub fn set_jobs(&self, mut jobs: Vec<Job>) {
        let (lock, cvar) = &*self.shared;
        let mut q = lock.lock().unwrap();
        jobs.retain(|j| !q.rendering.contains(j));
        if q.jobs != jobs {
            q.jobs = jobs;
            cvar.notify_all();
        }
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.shared;
        lock.lock().unwrap().quit = true;
        cvar.notify_all();
    }
}

fn load(
    path: PathBuf,
    shared: Shared,
    tx: Sender<Msg>,
    ctx: egui::Context,
    first_page_px_per_pt: Option<FirstPageScale>,
) {
    let send = |msg: Msg| {
        let _ = tx.send(msg);
        ctx.request_repaint();
    };

    let data: PdfData = match std::fs::read(&path) {
        Ok(d) => d.into(),
        Err(e) => return send(Msg::Failed(format!("Can't read file: {e}"))),
    };
    let file_size = data.as_ref().len() as u64;
    let pdf = match Pdf::new(data.clone()) {
        Ok(p) => p,
        Err(hayro::hayro_syntax::LoadPdfError::Decryption(_)) => {
            return send(Msg::Failed(
                "Password-protected PDFs aren't supported.".into(),
            ));
        }
        Err(_) => return send(Msg::Failed("This file isn't a readable PDF.".into())),
    };
    let info = doc_info(&pdf, file_size);
    if let (Some(px_for), Some(&first)) = (first_page_px_per_pt, info.pages.first()) {
        let px = px_for(first);
        let mut q = shared.0.lock().unwrap();
        if q.jobs.is_empty() {
            q.jobs.push(Job {
                page: 0,
                scale: render_scale(first, px, MAX_TEXTURE_SIDE),
                thumb: false,
            });
        }
    }
    send(Msg::Loaded(info));

    // hayro's documents aren't shareable across threads, but the bytes are, and parsing is cheap.
    for _ in 1..WORKERS.min(pdf.pages().len()) {
        let (data, shared, tx, ctx) = (data.clone(), shared.clone(), tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            if let Ok(pdf) = Pdf::new(data) {
                work(&pdf, &shared, &tx, &ctx);
            }
        });
    }
    work(&pdf, &shared, &tx, &ctx);
}

fn work(pdf: &Pdf, shared: &Shared, tx: &Sender<Msg>, ctx: &egui::Context) {
    let send = |msg: Msg| {
        let _ = tx.send(msg);
        ctx.request_repaint();
    };
    let pages = pdf.pages();
    let settings = InterpreterSettings::default();
    let render_cache = RenderCache::new();
    let text_cache = InterpreterCache::new();

    loop {
        let job = {
            let (lock, cvar) = &**shared;
            let mut q = lock.lock().unwrap();
            loop {
                if q.quit {
                    return;
                }
                if !q.jobs.is_empty() {
                    break;
                }
                q = cvar.wait(q).unwrap();
            }
            let job = q.jobs.remove(0);
            q.rendering.push(job);
            job
        };

        let page = &pages[job.page];
        let (w, h) = page.render_dimensions();
        // hayro is young; a malformed page must not take the whole viewer down.
        let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            hayro::render(
                page,
                &render_cache,
                &settings,
                &RenderSettings::default(),
                &PixmapSettings {
                    x_scale: job.scale,
                    y_scale: job.scale,
                    bg_color: WHITE,
                },
            )
        }));
        let image = match rendered {
            Ok(pix) if pix.width() > 0 && pix.height() > 0 => ColorImage::from_rgba_premultiplied(
                [pix.width() as usize, pix.height() as usize],
                pix.data_as_u8_slice(),
            ),
            _ => {
                let size = [
                    ((w * job.scale) as usize).max(1),
                    ((h * job.scale) as usize).max(1),
                ];
                ColorImage::new(size, vec![egui::Color32::WHITE; size[0] * size[1]])
            }
        };
        let extract_text = {
            let mut q = shared.0.lock().unwrap();
            q.rendering.retain(|j| *j != job);
            q.text_claimed.insert(job.page)
        };
        send(Msg::Rendered { job, image });

        if extract_text {
            let mut device = TextDevice { chars: Vec::new() };
            let extracted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut context = Context::new(
                    page.initial_transform(true).to_kurbo(),
                    kurbo::Rect::new(0.0, 0.0, w as f64, h as f64),
                    &text_cache,
                    page.xref(),
                    settings.clone(),
                );
                interpret_page(page, &mut context, &mut device);
            }));
            if extracted.is_ok() {
                send(Msg::Text {
                    page: job.page,
                    chars: device.chars,
                });
            }
        }
    }
}

fn doc_info(pdf: &Pdf, file_size: u64) -> DocInfo {
    let pages = pdf
        .pages()
        .iter()
        .map(|p| {
            let (w, h) = p.render_dimensions();
            PageSize { w, h }
        })
        .collect();

    let version = format!("{:?}", pdf.version());
    let version = match version.strip_prefix("Pdf") {
        Some(digits) if digits.len() == 2 => format!("{}.{}", &digits[..1], &digits[1..]),
        _ => version,
    };

    let m = pdf.metadata();
    let mut meta = Vec::new();
    for (label, value) in [
        ("Title", &m.title),
        ("Author", &m.author),
        ("Subject", &m.subject),
        ("Keywords", &m.keywords),
        ("Creator", &m.creator),
        ("Producer", &m.producer),
    ] {
        if let Some(s) = value
            .as_deref()
            .map(decode_pdf_string)
            .filter(|s| !s.trim().is_empty())
        {
            meta.push((label, s));
        }
    }
    for (label, date) in [
        ("Created", &m.creation_date),
        ("Modified", &m.modification_date),
    ] {
        if let Some(d) = date {
            meta.push((label, format_date(d)));
        }
    }

    DocInfo {
        pages,
        file_size,
        version,
        meta,
    }
}

/// Text strings in the info dictionary are either UTF-16BE with a BOM or PDFDocEncoding,
/// which is close enough to Latin-1 for display.
fn decode_pdf_string(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        String::from_utf16_lossy(&units)
    } else if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else {
        bytes.iter().map(|&b| b as char).collect()
    }
}

fn format_date(d: &DateTime) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        d.year, d.month, d.day, d.hour, d.minute
    )
}

/// A device that draws nothing and just records where each character ends up.
struct TextDevice {
    chars: Vec<TextChar>,
}

impl<'a> Device<'a> for TextDevice {
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        let glyphs = run.glyphs();
        for (i, glyph) in glyphs.iter().enumerate() {
            let text = match glyph.as_unicode() {
                Some(BfString::Char(c)) => c.to_string(),
                Some(BfString::String(s)) => s,
                None => continue,
            };
            if text.chars().all(char::is_control) {
                continue;
            }

            // Glyph space is 1000 units per em.
            let m = props.transform * glyph.transform();
            let origin = m * Point::ZERO;
            let em_x = m * Point::new(1000.0, 0.0) - origin;
            let em_y = m * Point::new(0.0, 1000.0) - origin;
            if em_x.hypot() < 1e-3 || em_y.hypot() < 1e-3 {
                continue;
            }

            // The advance isn't exposed, so measure it from the next glyph's origin when it
            // sits further along the same baseline, else fall back to the outline's extent.
            let advance_to_next = glyphs.get(i + 1).map(|next| {
                let next_origin = (props.transform * next.transform()) * Point::ZERO;
                (next_origin - origin).dot(em_x) / em_x.hypot2()
            });
            let width = match advance_to_next {
                Some(a) if a > 0.05 && a < 3.0 => a,
                _ => match &**glyph {
                    Glyph::Outline(o) => {
                        let path = o.outline();
                        if path.elements().is_empty() {
                            0.3
                        } else {
                            path.bounding_box().x1 / 1000.0
                        }
                    }
                    Glyph::Type3(_) => 0.5,
                },
            }
            .clamp(0.1, 3.0);

            let corners = [
                origin + em_y * -0.22,
                origin + em_y * 0.88,
                origin + em_x * width + em_y * -0.22,
                origin + em_x * width + em_y * 0.88,
            ];
            let mut rect = kurbo::Rect::from_points(corners[0], corners[1]);
            for c in &corners[2..] {
                rect = rect.union_pt(*c);
            }
            self.chars.push(TextChar {
                rect: Rect::from_min_max(
                    pos2(rect.x0 as f32, rect.y0 as f32),
                    pos2(rect.x1 as f32, rect.y1 as f32),
                ),
                text,
            });
        }
    }

    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}
