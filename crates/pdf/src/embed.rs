//! Embedded TrueType fonts for text the standard 14 fonts can't encode (Vietnamese, Greek,
//! Cyrillic, Polish…): WinAnsiEncoding has `à` and `ê` but not `ạ`, `ế`, `ơ` or `đ`, which used to
//! come out as `?`.
//!
//! A face is written as a Type0 font with a CIDFontType2 descendant, `Identity-H` encoding (two-byte
//! glyph ids in the content stream) and a ToUnicode CMap so the text can be copied and searched.
//! The embedded file is a subset that keeps glyph ids (unused glyphs are emptied), so the content
//! stream needs no remapping. Only TrueType outlines (`glyf`) are embedded; a CFF-flavoured font
//! is not used.
//!
//! The faces come from the operating system's fonts, preferring the metric-compatible families of
//! the standard fonts (Arial or Liberation Sans for Helvetica, Times New Roman or Liberation Serif
//! for Times, Courier New or Liberation Mono for Courier), so [`crate::text_width`]'s widths stay
//! right for the embedded text too. Callers can supply their own ([`crate::PdfDoc::set_unicode_font`]);
//! on the web there are no system fonts and such text keeps the old `?` fallback.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::OnceLock;

use skrifa::{FontRef, MetadataProvider, Tag};

use crate::Font;

/// Largest font file read (system CJK collections can be far larger and are not wanted here).
const MAX_FONT_BYTES: usize = 40 << 20;
/// Most glyphs a font may claim (the `maxp` limit).
const MAX_GLYPHS: usize = 65_535;
/// Deepest composite glyph nesting followed when collecting components.
const MAX_COMPOSITE_DEPTH: u32 = 8;

/// A parsed TrueType face, ready to map characters and to subset.
pub(crate) struct Face {
    data: Vec<u8>,
    index: u32,
    /// PostScript-safe base name (no spaces or delimiters).
    name: String,
    units_per_em: u16,
    ascent: i16,
    descent: i16,
    bbox: [i16; 4],
    italic_angle: f32,
    cap_height: i16,
    fixed_pitch: bool,
    /// Advance widths by glyph id, in font units.
    advances: Vec<u16>,
}

fn be16(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(at)?, *d.get(at + 1)?]))
}
fn bei16(d: &[u8], at: usize) -> Option<i16> {
    be16(d, at).map(|v| v as i16)
}
fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*d.get(at)?, *d.get(at + 1)?, *d.get(at + 2)?, *d.get(at + 3)?]))
}

impl Face {
    /// Parses face `index` of a TrueType/OpenType file (or collection). `None` for anything this
    /// writer can't embed: no `glyf` outlines, a licence that forbids embedding, malformed tables.
    pub(crate) fn parse(data: Vec<u8>, index: u32) -> Option<Face> {
        if data.len() > MAX_FONT_BYTES {
            return None;
        }
        let font = FontRef::from_index(&data, index).ok()?;
        let table = |t: &[u8; 4]| font.table_data(Tag::new(t)).map(|d| d.as_bytes());
        table(b"glyf")?;
        table(b"loca")?;
        let head = table(b"head")?;
        let hhea = table(b"hhea")?;
        let hmtx = table(b"hmtx")?;
        let maxp = table(b"maxp")?;
        let units_per_em = be16(head, 18).filter(|u| (16..=16_384).contains(u))?;
        let bbox = [bei16(head, 36)?, bei16(head, 38)?, bei16(head, 40)?, bei16(head, 42)?];
        let num_glyphs = usize::from(be16(maxp, 4)?).min(MAX_GLYPHS);
        let num_h = usize::from(be16(hhea, 34)?).min(num_glyphs).max(1);
        let mut advances = Vec::with_capacity(num_glyphs);
        let mut last = 0u16;
        for g in 0..num_glyphs {
            if g < num_h {
                last = be16(hmtx, g * 4)?;
            }
            advances.push(last);
        }
        let (mut ascent, mut descent) = (bei16(hhea, 4)?, bei16(hhea, 6)?);
        let mut cap_height = ascent;
        if let Some(os2) = table(b"OS/2") {
            // fsType: bit 1 alone (0x0002) is "restricted licence": the font must not be embedded.
            let fs_type = be16(os2, 8).unwrap_or(0);
            if fs_type & 0x000F == 0x0002 {
                return None;
            }
            if let (Some(a), Some(d)) = (bei16(os2, 68), bei16(os2, 70))
                && a > 0
            {
                ascent = a;
                descent = d;
            }
            if be16(os2, 0).unwrap_or(0) >= 2
                && let Some(c) = bei16(os2, 88).filter(|c| *c > 0)
            {
                cap_height = c;
            }
        }
        let (italic_angle, fixed_pitch) = match table(b"post") {
            Some(post) => (be32(post, 4).map_or(0.0, |v| v as i32 as f32 / 65536.0), be32(post, 12).unwrap_or(0) != 0),
            None => (0.0, false),
        };
        let name = postscript_name(&font).unwrap_or_else(|| "EmbeddedFont".to_string());
        Some(Face { data, index, name, units_per_em, ascent, descent, bbox, italic_angle, cap_height, fixed_pitch, advances })
    }

    fn font(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(&self.data, self.index).ok()
    }

    /// The glyph id for each character of `text` (`None` where the face has no glyph).
    pub(crate) fn glyphs(&self, text: &str) -> Vec<(char, Option<u16>)> {
        let Some(font) = self.font() else { return text.chars().map(|c| (c, None)).collect() };
        let cmap = font.charmap();
        text.chars()
            .map(|c| (c, cmap.map(c).and_then(|g| u16::try_from(g.to_u32()).ok()).filter(|g| usize::from(*g) < self.advances.len() && *g != 0)))
            .collect()
    }

    /// Advance of glyph `g` in 1/1000 em.
    fn width_1000(&self, g: u16) -> i64 {
        let adv = self.advances.get(usize::from(g)).copied().unwrap_or(0);
        (i64::from(adv) * 1000 + i64::from(self.units_per_em) / 2) / i64::from(self.units_per_em)
    }

    fn scale(&self, v: i16) -> i64 {
        i64::from(v) * 1000 / i64::from(self.units_per_em)
    }

    /// Bytes of glyph `g`'s outline (0 for an empty glyph), for tests.
    #[cfg(test)]
    pub(crate) fn glyph_len(&self, g: u16) -> Option<usize> {
        let font = self.font()?;
        let head = font.table_data(Tag::new(b"head"))?;
        let loca = font.table_data(Tag::new(b"loca"))?;
        let (head, loca) = (head.as_bytes(), loca.as_bytes());
        let g = usize::from(g);
        if be16(head, 50)? != 0 {
            Some((be32(loca, g * 4 + 4)? as usize).checked_sub(be32(loca, g * 4)? as usize)?)
        } else {
            Some((usize::from(be16(loca, g * 2 + 2)?) * 2).checked_sub(usize::from(be16(loca, g * 2)?) * 2)?)
        }
    }

    /// A TrueType file holding only the glyphs in `used` (plus `.notdef` and the components of
    /// composite glyphs), every glyph id kept.
    fn subset(&self, used: &BTreeSet<u16>) -> Option<Vec<u8>> {
        let font = self.font()?;
        let table = |t: &[u8; 4]| font.table_data(Tag::new(t)).map(|d| d.as_bytes());
        let head = table(b"head")?;
        let loca = table(b"loca")?;
        let glyf = table(b"glyf")?;
        let long = be16(head, 50)? != 0;
        let n = self.advances.len();
        let range = |g: usize| -> Option<(usize, usize)> {
            let (a, b) = if long {
                (be32(loca, g * 4)? as usize, be32(loca, g * 4 + 4)? as usize)
            } else {
                (usize::from(be16(loca, g * 2)?) * 2, usize::from(be16(loca, g * 2 + 2)?) * 2)
            };
            (a <= b && b <= glyf.len()).then_some((a, b))
        };
        // Close over composite components.
        let mut keep: BTreeSet<usize> = used.iter().map(|g| usize::from(*g)).filter(|g| *g < n).collect();
        keep.insert(0);
        let mut frontier: Vec<(usize, u32)> = keep.iter().map(|g| (*g, 0)).collect();
        while let Some((g, depth)) = frontier.pop() {
            if depth >= MAX_COMPOSITE_DEPTH {
                continue;
            }
            let Some((a, b)) = range(g) else { continue };
            let Some(bytes) = glyf.get(a..b) else { continue };
            for c in components(bytes) {
                if c < n && keep.insert(c) {
                    frontier.push((c, depth + 1));
                }
            }
        }
        let mut new_glyf: Vec<u8> = Vec::new();
        let mut new_loca: Vec<u8> = Vec::with_capacity((n + 1) * 4);
        for g in 0..n {
            new_loca.extend_from_slice(&u32::try_from(new_glyf.len()).ok()?.to_be_bytes());
            if keep.contains(&g)
                && let Some((a, b)) = range(g)
                && let Some(bytes) = glyf.get(a..b)
            {
                new_glyf.extend_from_slice(bytes);
                while !new_glyf.len().is_multiple_of(4) {
                    new_glyf.push(0);
                }
            }
        }
        new_loca.extend_from_slice(&u32::try_from(new_glyf.len()).ok()?.to_be_bytes());
        let mut new_head = head.to_vec();
        // checkSumAdjustment is left 0 (PDF readers ignore it); loca is always long.
        new_head.get_mut(8..12)?.copy_from_slice(&[0; 4]);
        new_head.get_mut(50..52)?.copy_from_slice(&1u16.to_be_bytes());
        let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![(*b"glyf", new_glyf), (*b"head", new_head), (*b"loca", new_loca)];
        for t in [b"cvt ", b"fpgm", b"hhea", b"hmtx", b"maxp", b"prep"] {
            if let Some(d) = table(t) {
                tables.push((*t, d.to_vec()));
            }
        }
        tables.sort_by_key(|(t, _)| *t);
        Some(sfnt(&tables))
    }
}

/// The glyph ids a composite glyph refers to (none for a simple glyph).
fn components(glyph: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    if bei16(glyph, 0).is_none_or(|c| c >= 0) {
        return out;
    }
    let mut at = 10usize;
    for _ in 0..1024 {
        let (Some(flags), Some(gid)) = (be16(glyph, at), be16(glyph, at + 2)) else { break };
        out.push(usize::from(gid));
        at += 4;
        at += if flags & 0x0001 != 0 { 4 } else { 2 };
        if flags & 0x0008 != 0 {
            at += 2;
        } else if flags & 0x0040 != 0 {
            at += 4;
        } else if flags & 0x0080 != 0 {
            at += 8;
        }
        if flags & 0x0020 == 0 {
            break;
        }
    }
    out
}

/// An sfnt file from `(tag, data)` tables sorted by tag.
fn sfnt(tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let num = tables.len() as u16;
    let mut pow = 1u16;
    let mut log = 0u16;
    while pow.saturating_mul(2) <= num {
        pow *= 2;
        log += 1;
    }
    let search = pow.saturating_mul(16);
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    for v in [num, search, log, num.saturating_mul(16).saturating_sub(search)] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in tables {
        let sum = data.chunks(4).fold(0u32, |s, c| {
            let mut w = [0u8; 4];
            for (d, b) in w.iter_mut().zip(c) {
                *d = *b;
            }
            s.wrapping_add(u32::from_be_bytes(w))
        });
        out.extend_from_slice(tag);
        out.extend_from_slice(&sum.to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().div_ceil(4) * 4;
    }
    for (_, data) in tables {
        out.extend_from_slice(data);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }
    out
}

/// The face's PostScript name (name id 6), reduced to characters a PDF name allows unescaped.
fn postscript_name(font: &FontRef<'_>) -> Option<String> {
    let s = font.localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME).english_or_first()?.chars().collect::<String>();
    let s: String = s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(60).collect();
    (!s.is_empty()).then_some(s)
}

/// Glyph usage of one embedded face across the document: glyph id → the character it draws (for
/// ToUnicode).
pub(crate) type Usage = BTreeMap<u16, char>;

/// A subset tag (`ABCDEF+`) derived from the glyphs used, as PDF requires for subset fonts.
fn subset_tag(used: &Usage) -> String {
    let mut h: u32 = 2_166_136_261;
    for (g, c) in used {
        for b in g.to_be_bytes().into_iter().chain((*c as u32).to_be_bytes()) {
            h = (h ^ u32::from(b)).wrapping_mul(16_777_619);
        }
    }
    (0..6)
        .map(|i| {
            let v = (h >> (i * 5)) % 26;
            char::from(b'A' + v as u8)
        })
        .collect()
}

/// A PDF object of an embedded face: a dictionary, or a stream's extra dictionary entries (the
/// writer adds `/Length`) and its data.
pub(crate) enum Body {
    Dict(String),
    Stream(String, Vec<u8>),
}

/// The PDF objects for face `face` used as `usage`: returns `(objects, type0_id)` with ids taken
/// from `next`.
pub(crate) fn write_objects(face: &Face, usage: &Usage, next: &mut u32) -> (Vec<(u32, Body)>, u32) {
    let type0 = *next;
    let cid = type0 + 1;
    let desc = type0 + 2;
    let file = type0 + 3;
    let tounicode = type0 + 4;
    *next += 5;
    let used: BTreeSet<u16> = usage.keys().copied().collect();
    let base = format!("{}+{}", subset_tag(usage), face.name);
    let mut objects = Vec::new();
    objects.push((
        type0,
        Body::Dict(format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /{base} /Encoding /Identity-H /DescendantFonts [{cid} 0 R] /ToUnicode {tounicode} 0 R >>"
        )),
    ));
    let mut w = String::new();
    for g in &used {
        let _ = write!(w, "{g} [{}] ", face.width_1000(*g));
    }
    objects.push((
        cid,
        Body::Dict(format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{base} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {desc} 0 R /DW {} /W [{w}] /CIDToGIDMap /Identity >>",
            face.width_1000(0)
        )),
    ));
    // Flags: 1 fixed pitch, 4 symbolic (required with Identity-H), 64 italic.
    let flags = 4 | if face.fixed_pitch { 1 } else { 0 } | if face.italic_angle != 0.0 { 64 } else { 0 };
    let [x0, y0, x1, y1] = face.bbox;
    objects.push((
        desc,
        Body::Dict(format!(
            "<< /Type /FontDescriptor /FontName /{base} /Flags {flags} /FontBBox [{} {} {} {}] /ItalicAngle {} /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 {file} 0 R >>",
            face.scale(x0),
            face.scale(y0),
            face.scale(x1),
            face.scale(y1),
            crate::n(face.italic_angle),
            face.scale(face.ascent),
            face.scale(face.descent),
            face.scale(face.cap_height),
        )),
    ));
    let font_file = face.subset(&used).unwrap_or_default();
    objects.push((file, Body::Stream(format!("/Length1 {}", font_file.len()), font_file)));
    objects.push((tounicode, Body::Stream(String::new(), to_unicode(usage).into_bytes())));
    (objects, type0)
}

/// A ToUnicode CMap mapping each used glyph id to its character.
fn to_unicode(usage: &Usage) -> String {
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<(&u16, &char)> = usage.iter().collect();
    for chunk in entries.chunks(100) {
        let _ = writeln!(s, "{} beginbfchar", chunk.len());
        for (g, c) in chunk {
            let mut u = String::new();
            let mut buf = [0u16; 2];
            for unit in c.encode_utf16(&mut buf) {
                let _ = write!(u, "{unit:04X}");
            }
            let _ = writeln!(s, "<{g:04X}> <{u}>");
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    s
}

/// The Unicode faces standing in for each of the 12 standard fonts, loaded on first use.
#[derive(Default)]
pub(crate) struct Faces {
    slots: [OnceLock<Option<Face>>; 12],
}

impl Faces {
    /// The face for standard font `font`: a caller-supplied one, else the system's.
    pub(crate) fn get(&self, font: Font) -> Option<&Face> {
        self.slots.get(font.index())?.get_or_init(|| system_face(font)).as_ref()
    }

    /// Supplies the face for `font`; false when it is unusable or that slot was already used.
    pub(crate) fn set(&self, font: Font, face: Face) -> bool {
        self.slots.get(font.index()).is_some_and(|s| s.set(Some(face)).is_ok())
    }
}

/// Candidate system font files for a standard font, best first: the metric-compatible family in
/// the right style, then its regular face, then a broad-coverage fallback.
fn candidates(font: Font) -> Vec<String> {
    use Font::*;
    let (bold, italic) = match font {
        HelveticaBold | TimesBold | CourierBold => (true, false),
        HelveticaOblique | TimesItalic | CourierOblique => (false, true),
        HelveticaBoldOblique | TimesBoldItalic | CourierBoldOblique => (true, true),
        _ => (false, false),
    };
    let family = match font {
        Helvetica | HelveticaBold | HelveticaOblique | HelveticaBoldOblique => 0,
        TimesRoman | TimesBold | TimesItalic | TimesBoldItalic => 1,
        _ => 2,
    };
    let style = usize::from(bold) + 2 * usize::from(italic);
    // Windows file names, macOS names (Supplemental), Liberation and DejaVu names, by style
    // (regular, bold, italic, bold italic).
    let win: [[&str; 4]; 3] =
        [["arial", "arialbd", "ariali", "arialbi"], ["times", "timesbd", "timesi", "timesbi"], ["cour", "courbd", "couri", "courbi"]];
    let mac_family = ["Arial", "Times New Roman", "Courier New"][family];
    let mac_style = ["", " Bold", " Italic", " Bold Italic"];
    let lib_family = ["LiberationSans", "LiberationSerif", "LiberationMono"][family];
    let lib_style = ["Regular", "Bold", "Italic", "BoldItalic"];
    let dejavu_family = ["DejaVuSans", "DejaVuSerif", "DejaVuSansMono"][family];
    let dejavu_style = ["", "-Bold", if family == 1 { "-Italic" } else { "-Oblique" }, if family == 1 { "-BoldItalic" } else { "-BoldOblique" }];
    let mut out = Vec::new();
    let windir = std::env::var("WINDIR").or_else(|_| std::env::var("SystemRoot")).unwrap_or_else(|_| "C:\\Windows".into());
    for s in [style, 0] {
        if let Some(name) = win.get(family).and_then(|w| w.get(s)) {
            out.push(format!("{windir}\\Fonts\\{name}.ttf"));
        }
        let mac = format!("{mac_family}{}.ttf", mac_style.get(s).unwrap_or(&""));
        out.push(format!("/System/Library/Fonts/Supplemental/{mac}"));
        out.push(format!("/Library/Fonts/{mac}"));
        let lib = format!("{lib_family}-{}.ttf", lib_style.get(s).unwrap_or(&"Regular"));
        for dir in [
            "/usr/share/fonts/truetype/liberation",
            "/usr/share/fonts/truetype/liberation2",
            "/usr/share/fonts/liberation-sans",
            "/usr/share/fonts/liberation-serif",
            "/usr/share/fonts/liberation-mono",
            "/usr/share/fonts/liberation",
            "/usr/share/fonts/TTF",
            "/usr/local/share/fonts/liberation-fonts-ttf",
            "/usr/local/share/fonts/TTF",
        ] {
            out.push(format!("{dir}/{lib}"));
        }
        let dv = format!("{dejavu_family}{}.ttf", dejavu_style.get(s).unwrap_or(&""));
        for dir in ["/usr/share/fonts/truetype/dejavu", "/usr/share/fonts/dejavu", "/usr/share/fonts/TTF", "/usr/local/share/fonts/dejavu"] {
            out.push(format!("{dir}/{dv}"));
        }
    }
    out
}

/// The first usable system face for `font`, if any (never on the web).
fn system_face(font: Font) -> Option<Face> {
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    candidates(font).into_iter().find_map(|path| {
        let len = std::fs::metadata(&path).ok()?.len();
        if len > MAX_FONT_BYTES as u64 {
            return None;
        }
        Face::parse(std::fs::read(&path).ok()?, 0)
    })
}
