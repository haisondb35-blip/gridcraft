//! Cell formatting in `.xls` files: the Font, XF and Palette records of the workbook globals
//! ([MS-XLS] 2.4.122 Font, 2.4.353 XF with 2.5.37 CellXF, 2.4.188 Palette, 2.5.161 Icv), turned
//! into GridCraft styles: font, fill, borders, alignment, number format and protection.

use gridcraft_model::style::Protection;
use gridcraft_model::{
    Alignment, BorderLine, BorderStyle, Borders, Color, Fill, Font, HAlign, NumFmt, PatternType, Style, Underline, VAlign, VertAlign,
};

use super::records::Cursor;
use crate::IoError;
use crate::tables::INDEXED;

/// Fonts kept at most (the format allows a few thousand; more is a hostile file).
pub(super) const MAX_FONTS: usize = 4096;

/// One Font record.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct XlsFont {
    pub name: String,
    /// Twentieths of a point.
    pub height: u16,
    pub italic: bool,
    pub strike: bool,
    pub color: u16,
    pub weight: u16,
    pub script: u16,
    pub underline: u8,
}

/// Read a Font record.
pub(super) fn font(c: &mut Cursor<'_, '_>) -> Result<XlsFont, IoError> {
    let height = c.u16()?;
    let flags = c.u16()?;
    let color = c.u16()?;
    let weight = c.u16()?;
    let script = c.u16()?;
    let underline = c.u8()?;
    // Family, character set and a reserved byte.
    c.skip(3)?;
    let name = c.short_string()?;
    Ok(XlsFont { name, height, italic: flags & 0x0002 != 0, strike: flags & 0x0008 != 0, color, weight, script, underline })
}

/// One cell or style XF record: what a cell's format points at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Xf {
    pub font: u16,
    pub format: u16,
    pub locked: bool,
    pub hidden: bool,
    pub h_align: u8,
    pub wrap: bool,
    pub v_align: u8,
    pub rotation: u8,
    pub indent: u8,
    pub shrink: bool,
    /// Border styles: left, right, top, bottom, diagonal.
    pub border: [u8; 5],
    /// Border colours, in the same order.
    pub border_color: [u16; 5],
    /// Bit 0: diagonal down, bit 1: diagonal up.
    pub diagonal: u8,
    pub pattern: u8,
    pub fore: u16,
    pub back: u16,
}

/// Read an XF record. A record shorter than BIFF8's 20 bytes keeps what it has and defaults the
/// rest (an XF is never worth refusing the whole workbook over).
pub(super) fn xf(c: &mut Cursor<'_, '_>) -> Result<Xf, IoError> {
    let mut x = Xf { font: c.u16()?, format: c.u16()?, ..Xf::default() };
    if c.remaining() < 16 {
        x.locked = true;
        return Ok(x);
    }
    let flags = c.u16()?;
    x.locked = flags & 0x0001 != 0;
    x.hidden = flags & 0x0002 != 0;
    let align = c.u8()?;
    x.h_align = align & 0x07;
    x.wrap = align & 0x08 != 0;
    x.v_align = (align >> 4) & 0x07;
    x.rotation = c.u8()?;
    let indent = c.u8()?;
    x.indent = indent & 0x0F;
    x.shrink = indent & 0x10 != 0;
    // Which attributes differ from the parent style: cell XFs hold them all anyway.
    c.u8()?;
    let a = c.u32()?;
    let b = c.u32()?;
    let fill = c.u16()?;
    x.border = [(a & 0x0F) as u8, ((a >> 4) & 0x0F) as u8, ((a >> 8) & 0x0F) as u8, ((a >> 12) & 0x0F) as u8, ((b >> 21) & 0x0F) as u8];
    x.border_color = [((a >> 16) & 0x7F) as u16, ((a >> 23) & 0x7F) as u16, (b & 0x7F) as u16, ((b >> 7) & 0x7F) as u16, ((b >> 14) & 0x7F) as u16];
    x.diagonal = ((a >> 30) & 0x03) as u8;
    x.pattern = ((b >> 26) & 0x3F) as u8;
    x.fore = fill & 0x7F;
    x.back = (fill >> 7) & 0x7F;
    Ok(x)
}

/// The 64-entry colour table: the standard one, with a Palette record's colours in 8..64.
pub(super) fn palette(custom: &[u32]) -> [u32; 64] {
    let mut p = INDEXED;
    for (slot, rgb) in p.iter_mut().skip(8).zip(custom) {
        *slot = *rgb;
    }
    p
}

/// Read a Palette record: its colours as 0xRRGGBB.
pub(super) fn palette_record(c: &mut Cursor<'_, '_>) -> Result<Vec<u32>, IoError> {
    let n = usize::from(c.u16()?).min(56).min(c.remaining() / 4);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let (r, g, b) = (c.u8()?, c.u8()?, c.u8()?);
        c.u8()?;
        out.push((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b));
    }
    Ok(out)
}

/// A colour index: 0..64 in the table, anything else (64 and 65 are the system's window text
/// and background, 0x7FFF the font's automatic colour) is automatic.
fn color(icv: u16, palette: &[u32; 64]) -> Color {
    palette.get(usize::from(icv)).map_or(Color::Auto, |rgb| Color::Rgb(*rgb))
}

fn border_style(dg: u8) -> BorderStyle {
    match dg {
        1 => BorderStyle::Thin,
        2 => BorderStyle::Medium,
        3 => BorderStyle::Dashed,
        4 => BorderStyle::Dotted,
        5 => BorderStyle::Thick,
        6 => BorderStyle::Double,
        7 => BorderStyle::Hair,
        8 => BorderStyle::MediumDashed,
        9 => BorderStyle::DashDot,
        10 => BorderStyle::MediumDashDot,
        11 => BorderStyle::DashDotDot,
        12 => BorderStyle::MediumDashDotDot,
        13 => BorderStyle::SlantDashDot,
        _ => BorderStyle::None,
    }
}

fn pattern(fls: u8) -> PatternType {
    match fls {
        1 => PatternType::Solid,
        2 => PatternType::Gray50,
        3 => PatternType::Gray75,
        4 => PatternType::Gray25,
        5 => PatternType::DarkHorizontal,
        6 => PatternType::DarkVertical,
        7 => PatternType::DarkDown,
        8 => PatternType::DarkUp,
        9 => PatternType::DarkGrid,
        10 => PatternType::DarkTrellis,
        11 => PatternType::LightHorizontal,
        12 => PatternType::LightVertical,
        13 => PatternType::LightDown,
        14 => PatternType::LightUp,
        15 => PatternType::LightGrid,
        16 => PatternType::LightTrellis,
        17 => PatternType::Gray125,
        18 => PatternType::Gray0625,
        _ => PatternType::None,
    }
}

/// The font an XF names: index 4 doesn't exist in the file (a BIFF quirk), so indexes past it
/// count one less. A missing font is the file's first (its Normal font).
fn font_of(fonts: &[XlsFont], index: u16) -> Option<&XlsFont> {
    let i = usize::from(index);
    fonts.get(if i >= 4 { i - 1 } else { i }).or_else(|| fonts.first())
}

/// The GridCraft style for `x`, with `num_fmt` its number format code.
pub(super) fn style(x: &Xf, fonts: &[XlsFont], palette: &[u32; 64], num_fmt: &str) -> Style {
    let font = font_of(fonts, x.font).map_or_else(Font::default, |f| Font {
        name: if f.name.trim().is_empty() { Font::default().name } else { f.name.clone() },
        size: if (20..=8191).contains(&f.height) { f32::from(f.height) / 20.0 } else { Font::default().size },
        bold: f.weight >= 600,
        italic: f.italic,
        underline: match f.underline {
            1 => Underline::Single,
            2 => Underline::Double,
            0x21 => Underline::SingleAccounting,
            0x22 => Underline::DoubleAccounting,
            _ => Underline::None,
        },
        strike: f.strike,
        color: color(f.color, palette),
        vert: match f.script {
            1 => VertAlign::Superscript,
            2 => VertAlign::Subscript,
            _ => VertAlign::Baseline,
        },
    });
    let pattern = pattern(x.pattern);
    let fill = if pattern == PatternType::None { Fill::default() } else { Fill { pattern, fg: color(x.fore, palette), bg: color(x.back, palette) } };
    let line = |i: usize| {
        let style = border_style(x.border.get(i).copied().unwrap_or(0));
        if style == BorderStyle::None {
            BorderLine::default()
        } else {
            BorderLine { style, color: color(x.border_color.get(i).copied().unwrap_or(64), palette) }
        }
    };
    let diag = line(4);
    let borders = Borders {
        left: line(0),
        right: line(1),
        top: line(2),
        bottom: line(3),
        diag_down: if x.diagonal & 1 != 0 { diag } else { BorderLine::default() },
        diag_up: if x.diagonal & 2 != 0 { diag } else { BorderLine::default() },
    };
    let align = Alignment {
        h: match x.h_align {
            1 => HAlign::Left,
            2 => HAlign::Center,
            3 => HAlign::Right,
            4 => HAlign::Fill,
            5 => HAlign::Justify,
            6 => HAlign::CenterAcross,
            7 => HAlign::Distributed,
            _ => HAlign::General,
        },
        v: match x.v_align {
            0 => VAlign::Top,
            1 => VAlign::Center,
            3 => VAlign::Justify,
            4 => VAlign::Distributed,
            _ => VAlign::Bottom,
        },
        wrap: x.wrap,
        shrink: x.shrink,
        indent: x.indent,
        // 0..=90 counter-clockwise, 91..=180 clockwise (90 less), 255 stacked; anything else flat.
        rotation: match x.rotation {
            r @ 0..=90 => i16::from(r),
            r @ 91..=180 => 90 - i16::from(r),
            255 => 255,
            _ => 0,
        },
    };
    Style { font, fill, borders, align, num_fmt: NumFmt::new(num_fmt), protection: Protection { locked: x.locked, hidden: x.hidden } }
}
