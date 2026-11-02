//! Drawing a metafile into pixels (ported from DeckCraft's renderer): paths, bitmaps and pattern
//! fills with vello_cpu; text with the installed family the file names, through the font source
//! the app registers ([`set_font_source`]), Arial or Segoe UI or Tahoma for a character it lacks,
//! one glyph per character from the face's character map, the file's own advances (the dx array)
//! placing each one. Without a font source the text is left out.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::{HAlign, Item, PlacedItem, Text, Tile, VAlign};
use kurbo::{Affine, BezPath, Rect, Shape as _};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{LocationRef, Size};
use skrifa::{FontRef, GlyphId, MetadataProvider as _};
use vello_cpu::{Pixmap, RenderContext, Resources, peniko};

/// Largest side drawn.
const MAX_SIDE: u32 = 4096;
/// Pattern cells smaller than this (output pixels) are drawn as the tile's average colour: a
/// sub-pixel pattern would only alias.
const MIN_CELL_PX: f64 = 1.5;
/// Size of text whose font gives no height (LOGFONT height 0), in text units.
const DEFAULT_EM: f64 = 12.0;

/// Draw the metafile `bytes` into straight RGBA with its longer side `side` px (transparent where
/// it draws nothing). `None` when it doesn't parse or has no size.
pub fn rasterize(bytes: &[u8], side: u32) -> Option<image::RgbaImage> {
    let mut pic = crate::parse(bytes).ok()?;
    if !pic.has_size() {
        return None;
    }
    let side = f64::from(side.clamp(16, MAX_SIDE));
    let (pw, ph) = (f64::from(pic.width), f64::from(pic.height));
    let k = side / pw.max(ph);
    let w = (pw * k).round().clamp(1.0, f64::from(MAX_SIDE)) as u16;
    let h = (ph * k).round().clamp(1.0, f64::from(MAX_SIDE)) as u16;
    let pixmaps: Vec<Option<Arc<Pixmap>>> = pic.items.iter_mut().map(|it| it.take_pixels().and_then(|(w, h, px)| premul(w, h, &px))).collect();
    // Pattern tiles shared by several fills are converted once.
    let mut tiles: HashMap<*const Tile, (Option<Arc<Pixmap>>, peniko::Color)> = HashMap::new();
    let mut ctx = RenderContext::new_with(w, h, vello_cpu::RenderSettings { num_threads: 0, ..Default::default() });
    let r = Rect::new(0.0, 0.0, f64::from(w), f64::from(h));
    ctx.push_clip_layer(&r.to_path(0.1));
    for it in pic.place(r, [0.0; 4]) {
        match it {
            PlacedItem::Path { path, fill, stroke, even_odd, region } => clipped(&mut ctx, region.as_deref(), |ctx| {
                if let Some(f) = fill {
                    ctx.set_paint(rgba(f));
                    ctx.set_fill_rule(if even_odd { peniko::Fill::EvenOdd } else { peniko::Fill::NonZero });
                    ctx.fill_path(&path);
                }
                // GDI pens: round caps and joins; a cosmetic (zero-width) pen is one pixel.
                if let Some((s, sw)) = stroke {
                    ctx.set_paint(rgba(s));
                    ctx.set_stroke(kurbo::Stroke::new(sw.max(1.0)).with_caps(kurbo::Cap::Round).with_join(kurbo::Join::Round));
                    ctx.stroke_path(&path);
                }
            }),
            PlacedItem::Bitmap { index, rect, region } => clipped(&mut ctx, region.as_deref(), |ctx| {
                let Some(Some(pm)) = pixmaps.get(index) else { return };
                let (bw, bh) = (f64::from(pm.width().max(1)), f64::from(pm.height().max(1)));
                ctx.set_paint(vello_cpu::Image {
                    image: vello_cpu::ImageSource::Pixmap(pm.clone()),
                    sampler: peniko::ImageSampler::default().with_quality(peniko::ImageQuality::Medium),
                });
                ctx.set_paint_transform(Affine::translate((rect.x0, rect.y0)) * Affine::scale_non_uniform(rect.width() / bw, rect.height() / bh));
                ctx.fill_rect(&rect);
                ctx.reset_paint_transform();
            }),
            PlacedItem::Pattern { index, path, even_odd, tile, region } => clipped(&mut ctx, region.as_deref(), |ctx| {
                let Some(Item::Pattern { tile: t, .. }) = pic.items.get(index) else { return };
                let key = Arc::as_ptr(t);
                let (pm, avg) = tiles.entry(key).or_insert_with(|| (premul(t.width, t.height, &t.rgba), average(&t.rgba))).clone();
                ctx.set_fill_rule(if even_odd { peniko::Fill::EvenOdd } else { peniko::Fill::NonZero });
                let cell = tile.determinant().abs().sqrt();
                match pm {
                    Some(pm) if cell >= MIN_CELL_PX => {
                        ctx.set_paint(vello_cpu::Image {
                            image: vello_cpu::ImageSource::Pixmap(pm),
                            sampler: peniko::ImageSampler {
                                x_extend: peniko::Extend::Repeat,
                                y_extend: peniko::Extend::Repeat,
                                quality: peniko::ImageQuality::Low,
                                alpha: 1.0,
                            },
                        });
                        ctx.set_paint_transform(tile);
                        ctx.fill_path(&path);
                        ctx.reset_paint_transform();
                    }
                    _ => {
                        ctx.set_paint(avg);
                        ctx.fill_path(&path);
                    }
                }
            }),
            PlacedItem::Text { index, transform, clip, region } => clipped(&mut ctx, region.as_deref(), |ctx| {
                let Some(Item::Text(t)) = pic.items.get(index) else { return };
                if let Some(c) = clip {
                    ctx.push_clip_layer(&c.to_path(0.1));
                }
                draw_text(ctx, t, transform);
                if clip.is_some() {
                    ctx.pop_layer();
                }
            }),
        }
    }
    ctx.set_fill_rule(peniko::Fill::NonZero);
    ctx.pop_layer();
    ctx.flush();
    let mut px = vec![0u8; usize::from(w) * usize::from(h) * 4];
    if let Some(pm) = vello_cpu::PixmapMut::new(w, h, &mut px) {
        ctx.render(pm, &mut Resources::new());
    }
    // Premultiplied to straight.
    for p in px.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        if a != 0 && a != 255 {
            for c in p.iter_mut().take(3) {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    image::RgbaImage::from_raw(u32::from(w), u32::from(h), px)
}

/// Draws `draw` inside the device-context clip. No region is the default clip.
fn clipped(ctx: &mut RenderContext, region: Option<&BezPath>, draw: impl FnOnce(&mut RenderContext)) {
    if let Some(region) = region {
        ctx.push_clip_layer(region);
        draw(ctx);
        ctx.pop_layer();
    } else {
        draw(ctx);
    }
}

/// One installed face for metafile text: the font file's bytes, the face's index in it, its
/// weight (400 regular) and whether it is italic.
#[derive(Clone)]
pub struct FaceData {
    pub bytes: Arc<Vec<u8>>,
    pub index: u32,
    pub weight: f32,
    pub italic: bool,
}

impl FaceData {
    fn font(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(&self.bytes, self.index).ok()
    }
}

/// Finds the installed face of a family closest to bold/italic: `(family, bold, italic)`.
pub type FontSource = fn(&str, bool, bool) -> Option<FaceData>;

static FONT_SOURCE: OnceLock<FontSource> = OnceLock::new();

/// Register where metafile text finds its fonts (once, by the app; later calls are ignored).
pub fn set_font_source(source: FontSource) {
    let _ = FONT_SOURCE.set(source);
}

fn installed_face(family: &str, bold: bool, italic: bool) -> Option<FaceData> {
    FONT_SOURCE.get().and_then(|f| f(family, bold, italic))
}

fn glyph_of(face: &FaceData, c: char) -> Option<GlyphId> {
    face.font()?.charmap().map(c).filter(|g| g.to_u32() != 0)
}

/// One glyph placed in text space: its face, id, and pen position along the baseline.
struct Glyph {
    face: usize,
    gid: GlyphId,
    x: f64,
}

/// Draws a text item; `m` maps its text space to the output.
fn draw_text(ctx: &mut RenderContext, t: &Text, m: Affine) {
    let font = &t.font;
    let family = if font.family.trim().is_empty() { "Arial" } else { font.family.trim() };
    let (bold, italic) = (font.weight >= 600, font.italic);
    // No installed face (the font scan hasn't finished, or a bare system): the text is left out.
    let Some(main) = installed_face(family, bold, italic).or_else(|| installed_face("Arial", bold, italic)) else { return };
    let Some(main_font) = main.font() else { return };
    let metrics = main_font.metrics(Size::unscaled(), LocationRef::default());
    let upem = f64::from(metrics.units_per_em.max(1));
    let (asc, desc) = win_metrics(&main_font, &metrics);
    // LOGFONT height: negative is the em, positive the cell (ascent plus descent).
    let h = f64::from(font.height);
    let em = if h < 0.0 {
        -h
    } else if h > 0.0 {
        h * upem / (asc + desc).max(1.0)
    } else {
        DEFAULT_EM
    };
    let k = em / upem;
    if !k.is_finite() || k <= 0.0 {
        return;
    }
    // Faces: the main one, then fallbacks for characters it lacks.
    let mut faces = vec![main.clone()];
    let mut glyphs: Vec<Glyph> = Vec::new();
    let mut starts: Vec<f64> = Vec::new();
    let mut pen = 0.0;
    for c in t.text.chars() {
        starts.push(pen);
        if c.is_control() {
            continue;
        }
        let (fi, gid) = match glyph_of(&main, c) {
            Some(g) => (0, g),
            None => {
                let fallback = ["Arial", "Segoe UI", "Tahoma"].iter().find_map(|f| installed_face(f, bold, italic).filter(|d| glyph_of(d, c).is_some()));
                match fallback.and_then(|d| glyph_of(&d, c).map(|g| (d, g))) {
                    Some((d, g)) => {
                        faces.push(d);
                        (faces.len() - 1, g)
                    }
                    None => (0, GlyphId::new(0)),
                }
            }
        };
        let Some(f) = faces.get(fi).and_then(FaceData::font) else { continue };
        let fk = em / f64::from(f.metrics(Size::unscaled(), LocationRef::default()).units_per_em.max(1));
        let advance = f.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(gid).map_or(em / 2.0, |a| f64::from(a) * fk);
        glyphs.push(Glyph { face: fi, gid, x: pen });
        pen += advance;
    }
    // The file's advances place each character.
    let count = t.text.chars().count();
    let mut width = pen;
    let mut x_scale = 1.0;
    if !t.dx.is_empty() && t.dx.len() == count {
        let mut placed = Vec::with_capacity(count);
        let mut x = 0.0;
        for d in &t.dx {
            placed.push(x);
            x += f64::from(*d);
        }
        width = x;
        for g in &mut glyphs {
            if let Some(i) = starts.iter().position(|s| (*s - g.x).abs() < 1e-9) {
                g.x = placed.get(i).copied().unwrap_or(g.x);
            }
        }
    } else if font.width > 0.0 {
        // A LOGFONT width narrows or widens the font against its average character width.
        let avg = avg_char_width(&main_font, upem) * k;
        if avg > 0.0 {
            x_scale = (f64::from(font.width) / avg).clamp(0.05, 20.0);
            width *= x_scale;
            for g in &mut glyphs {
                g.x *= x_scale;
            }
        }
    }
    let x0 = match t.h_align {
        HAlign::Left => 0.0,
        HAlign::Center => -width / 2.0,
        HAlign::Right => -width,
    };
    let (a, d) = (asc * k, desc * k);
    let base = match t.v_align {
        VAlign::Baseline => 0.0,
        VAlign::Top => a,
        VAlign::Bottom => -d,
    };
    let at = m * Affine::translate((x0, base));
    ctx.set_fill_rule(peniko::Fill::NonZero);
    if let Some(bg) = t.background
        && width > 0.0
    {
        ctx.set_paint(rgba(bg));
        ctx.fill_path(&(at * Rect::new(0.0, -a, width, d).to_path(0.1)));
    }
    ctx.set_paint(rgba(t.color));
    for g in &glyphs {
        let Some(data) = faces.get(g.face) else { continue };
        let Some(f) = data.font() else { continue };
        let Some(outline) = f.outline_glyphs().get(g.gid) else { continue };
        let mut path = FlipPen(BezPath::new());
        if outline.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut path).is_err() || path.0.elements().is_empty() {
            continue;
        }
        let s = em / f64::from(f.metrics(Size::unscaled(), LocationRef::default()).units_per_em.max(1));
        let skew = if italic && !data.italic { Affine::new([1.0, 0.0, -0.2, 1.0, 0.0, 0.0]) } else { Affine::IDENTITY };
        let gm = at * Affine::translate((g.x, 0.0)) * Affine::scale_non_uniform(x_scale, 1.0) * skew * Affine::scale(s);
        let sc = gm.determinant().abs().sqrt();
        if !sc.is_finite() || sc > 1e5 {
            continue;
        }
        let p: BezPath = gm * path.0;
        if p.elements().len() > 10_000 || p.bounding_box().width() > 1e6 || p.bounding_box().height() > 1e6 {
            continue;
        }
        ctx.fill_path(&p);
        if bold && data.weight < 600.0 {
            ctx.set_stroke(kurbo::Stroke::new(em * 0.025 * at.determinant().abs().sqrt()).with_join(kurbo::Join::Round));
            ctx.stroke_path(&p);
        }
    }
    // Underline and strike-out: the font's own positions and thickness.
    let deco = |on: bool, d: Option<skrifa::metrics::Decoration>, fallback: (f64, f64)| -> Option<Rect> {
        if !on || width <= 0.0 {
            return None;
        }
        let (off, th) = d.map_or(fallback, |d| (f64::from(d.offset) * k, f64::from(d.thickness) * k));
        let th = if th > 0.0 { th } else { em / 20.0 };
        // Offsets are y-up from the baseline; text space is y-down.
        Some(Rect::new(0.0, -off - th / 2.0, width, -off + th / 2.0))
    };
    let lines = [deco(font.underline, metrics.underline, (-em * 0.1, em / 20.0)), deco(font.strike, metrics.strikeout, (em * 0.26, em / 20.0))];
    for r in lines.into_iter().flatten() {
        ctx.fill_path(&(at * r.to_path(0.1)));
    }
}

/// Glyph outlines in font units, y-down.
struct FlipPen(BezPath);

impl OutlinePen for FlipPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), -f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), -f64::from(y)));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to((f64::from(cx0), -f64::from(cy0)), (f64::from(x), -f64::from(y)));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to((f64::from(cx0), -f64::from(cy0)), (f64::from(cx1), -f64::from(cy1)), (f64::from(x), -f64::from(y)));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

/// The face's Windows ascent and descent (OS/2 `usWinAscent`/`usWinDescent`, as GDI measures the
/// cell), in font units; the typographic ones when the font has no OS/2 table.
fn win_metrics(font: &FontRef<'_>, metrics: &skrifa::metrics::Metrics) -> (f64, f64) {
    use skrifa::raw::TableProvider as _;
    match font.os2().ok().map(|o| (f64::from(o.us_win_ascent()), f64::from(o.us_win_descent()))) {
        Some((a, d)) if a + d > 0.0 => (a, d),
        _ => (f64::from(metrics.ascent).max(0.0), f64::from(-metrics.descent).max(0.0)),
    }
}

/// OS/2 `xAvgCharWidth` in font units (half an em without it).
fn avg_char_width(font: &FontRef<'_>, upem: f64) -> f64 {
    use skrifa::raw::TableProvider as _;
    font.os2().ok().map(|o| f64::from(o.x_avg_char_width())).filter(|w| *w > 0.0).unwrap_or(upem * 0.5)
}

fn rgba(c: [u8; 4]) -> peniko::Color {
    peniko::Color::from_rgba8(c[0], c[1], c[2], c[3])
}

/// The average colour of straight RGBA pixels (alpha-weighted).
fn average(px: &[u8]) -> peniko::Color {
    let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
    for p in px.as_chunks::<4>().0 {
        let w = u64::from(p[3]);
        r += u64::from(p[0]) * w;
        g += u64::from(p[1]) * w;
        b += u64::from(p[2]) * w;
        a += w;
        n += 1;
    }
    if a == 0 || n == 0 {
        return peniko::Color::from_rgba8(0, 0, 0, 0);
    }
    peniko::Color::from_rgba8((r / a) as u8, (g / a) as u8, (b / a) as u8, (a / n) as u8)
}

/// Straight RGBA rows to a premultiplied pixmap.
fn premul(w: u32, h: u32, px: &[u8]) -> Option<Arc<Pixmap>> {
    let (w16, h16) = (u16::try_from(w).ok()?, u16::try_from(h).ok()?);
    if w == 0 || h == 0 || px.len() < w as usize * h as usize * 4 {
        return None;
    }
    let data = px
        .as_chunks::<4>()
        .0
        .iter()
        .take(w as usize * h as usize)
        .map(|p| {
            let a = u16::from(p[3]);
            let m = |c: u8| ((u16::from(c) * a + 127) / 255) as u8;
            vello_cpu::color::PremulRgba8 { r: m(p[0]), g: m(p[1]), b: m(p[2]), a: p[3] }
        })
        .collect();
    Some(Arc::new(Pixmap::from_parts(data, w16, h16)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal EMF: header, a red filled rectangle, EOF.
    fn red_box_emf() -> Vec<u8> {
        let mut r: Vec<u8> = Vec::new();
        let u32s = |v: &mut Vec<u8>, xs: &[u32]| xs.iter().for_each(|x| v.extend_from_slice(&x.to_le_bytes()));
        // EMR_HEADER (88 bytes): bounds and frame (0.01 mm), signature, version, sizes, device.
        u32s(&mut r, &[1, 88, 0, 0, 99, 49, 0, 0, 2540, 1270, 0x464D_4520, 0x10000, 0, 5]);
        r.extend_from_slice(&2u16.to_le_bytes()); // handles
        r.extend_from_slice(&0u16.to_le_bytes());
        u32s(&mut r, &[0, 0, 0, 1024, 768, 320, 240]);
        assert_eq!(r.len(), 88);
        // EMR_CREATEBRUSHINDIRECT (ihBrush 1, solid red), EMR_SELECTOBJECT 1.
        u32s(&mut r, &[39, 24, 1, 0, 0x0000_00FF, 0]);
        u32s(&mut r, &[37, 12, 1]);
        // EMR_RECTANGLE.
        u32s(&mut r, &[43, 24, 10, 10, 90, 40]);
        // EMR_EOF.
        u32s(&mut r, &[14, 20, 0, 0, 20]);
        let total = r.len() as u32;
        r[48..52].copy_from_slice(&total.to_le_bytes());
        r
    }

    #[test]
    fn an_emf_draws_into_pixels() {
        let emf = red_box_emf();
        assert!(crate::is_metafile(&emf));
        let img = rasterize(&emf, 200).expect("drawn");
        assert!(img.width() > 0 && img.height() > 0);
        let red = img.pixels().filter(|p| p[0] > 200 && p[1] < 60 && p[2] < 60 && p[3] > 200).count();
        assert!(red > 100, "the red box shows ({red} red pixels)");
    }

    #[test]
    fn junk_is_not_drawn() {
        assert!(rasterize(b"not a metafile", 100).is_none());
        assert!(rasterize(&red_box_emf()[..40], 100).is_none());
    }
}
