//! The Windows clipboard: every format of a copy set while it is open once.

use std::num::NonZeroU32;
use std::path::PathBuf;

use clipboard_win::{formats, options::NoClear, raw};

use crate::{Copy, Holds, Html, MAX_READ, cf_html, dib};

const HTML: &str = "HTML Format";
const RTF: &str = "Rich Text Format";
/// Registered PNG names: the one office suites and browsers use, then the MIME type some write.
const PNGS: [&str; 2] = ["PNG", "image/png"];
const SVG: &str = "image/svg+xml";
/// The bitmaps, the one that carries alpha first. The system converts between them and
/// `CF_BITMAP`, so each is there whenever one of them is.
const DIBS: [u32; 2] = [formats::CF_DIBV5, formats::CF_DIB];

fn id(name: &str) -> Option<u32> {
    raw::register_format(name).map(NonZeroU32::get)
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The clipboard opened, retried for a moment while another program has it.
fn open() -> Option<clipboard_win::Clipboard> {
    clipboard_win::Clipboard::new_attempts(10).ok()
}

pub fn clear() -> Result<(), String> {
    let _open = clipboard_win::Clipboard::new_attempts(10).map_err(err)?;
    raw::empty().map_err(err)
}

pub fn write(copy: &Copy) -> Result<(), String> {
    // Converted before the clipboard is opened, not keeping other programs waiting.
    let bitmaps = copy.png.as_deref().and_then(|png| {
        let img = dib::decode_png(png)?;
        Some((dib::with_alpha(&img), dib::opaque(&img)))
    });
    let html = copy.html.as_deref().map(|h| cf_html::encode(h, copy.source_url.as_deref()));
    let files: Vec<String> = copy.files.iter().map(|p| p.to_string_lossy().into_owned()).collect();

    let _open = clipboard_win::Clipboard::new_attempts(10).map_err(err)?;
    raw::empty().map_err(err)?;
    let mut result = Ok(());
    let mut set = |r: Result<(), String>| {
        if result.is_ok() {
            result = r;
        }
    };
    let mut put = |name: &str, data: &[u8]| {
        set(id(name).ok_or_else(|| format!("can't register {name}")).and_then(|f| raw::set_without_clear(f, data).map_err(err)))
    };
    if let Some(h) = &html {
        put(HTML, h);
    }
    if let Some(r) = &copy.rtf {
        put(RTF, r);
    }
    if let Some(p) = &copy.png {
        put(PNGS[0], p);
    }
    if let Some(s) = &copy.svg {
        put(SVG, s);
    }
    for (name, data) in &copy.own {
        put(name, data);
    }
    if let Some((alpha, opaque)) = &bitmaps {
        set(raw::set_without_clear(formats::CF_DIBV5, alpha).map_err(err));
        set(raw::set_without_clear(formats::CF_DIB, opaque).map_err(err));
    }
    if !files.is_empty() {
        set(raw::set_file_list_with(&files[..], NoClear).map_err(err));
    }
    if let Some(t) = &copy.text {
        set(raw::set_string_with(t, NoClear).map_err(err));
    }
    result
}

/// Format `f`'s bytes, when it is there and no larger than [`MAX_READ`]. The clipboard must be
/// open.
fn read_format(f: u32) -> Option<Vec<u8>> {
    if !raw::is_format_avail(f) || raw::size(f)?.get() > MAX_READ {
        return None;
    }
    let mut data = vec![];
    raw::get_vec(f, &mut data).ok()?;
    (!data.is_empty()).then_some(data)
}

fn read_named(name: &str) -> Option<Vec<u8>> {
    let _open = open()?;
    read_format(id(name)?)
}

/// Text formats end at their first NUL: a memory block can be longer than its data.
fn until_nul(mut data: Vec<u8>) -> Vec<u8> {
    if let Some(end) = data.iter().position(|b| *b == 0) {
        data.truncate(end);
    }
    data
}

pub fn text() -> Option<String> {
    let _open = open()?;
    if !raw::is_format_avail(formats::CF_UNICODETEXT) || raw::size(formats::CF_UNICODETEXT)?.get() > MAX_READ {
        return None;
    }
    let mut data = vec![];
    raw::get_string(&mut data).ok()?;
    String::from_utf8(data).ok()
}

pub fn html() -> Option<Html> {
    cf_html::decode(&read_named(HTML)?)
}

pub fn rtf() -> Option<Vec<u8>> {
    read_named(RTF).map(until_nul).filter(|d| !d.is_empty())
}

pub fn svg() -> Option<Vec<u8>> {
    read_named(SVG).map(until_nul).filter(|d| !d.is_empty())
}

pub fn bitmap() -> Option<Vec<u8>> {
    let dibs: Vec<Vec<u8>> = {
        let _open = open()?;
        if let Some(png) = PNGS.iter().filter_map(|n| id(n)).find_map(read_format) {
            return Some(png);
        }
        DIBS.into_iter().filter_map(read_format).collect()
    };
    // Decoded with the clipboard closed again.
    dibs.iter().find_map(|d| dib::png(&dib::decode(d)?))
}

pub fn files() -> Vec<PathBuf> {
    let mut out = vec![];
    if let Some(_open) = open()
        && raw::is_format_avail(formats::CF_HDROP)
    {
        let _ = raw::get_file_list_path(&mut out);
    }
    out
}

pub fn own(name: &str) -> Option<Vec<u8>> {
    read_named(name)
}

pub fn holds_own(name: &str) -> bool {
    id(name).is_some_and(raw::is_format_avail)
}

pub fn holds() -> Holds {
    let named = |n: &str| id(n).is_some_and(raw::is_format_avail);
    Holds {
        text: raw::is_format_avail(formats::CF_UNICODETEXT),
        html: named(HTML),
        rtf: named(RTF),
        bitmap: PNGS.iter().any(|n| named(n)) || DIBS.into_iter().chain([formats::CF_BITMAP]).any(raw::is_format_avail),
        svg: named(SVG),
        files: raw::is_format_avail(formats::CF_HDROP),
    }
}

pub fn sequence() -> Option<u32> {
    raw::seq_num().map(NonZeroU32::get)
}

pub fn format_names() -> Vec<String> {
    let Some(_open) = open() else { return vec![] };
    clipboard_win::EnumFormats::new().map(|f| raw::format_name_big(f).unwrap_or_else(|| format!("#{f}"))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media;

    /// The real clipboard: one test, so nothing else runs between its writes and reads. Ignored by
    /// default (it replaces what the user copied): `cargo test -p craft-clipboard -- --ignored`.
    #[test]
    #[ignore = "replaces the user's clipboard"]
    fn a_copy_reads_back_in_every_format() {
        let mut img = image::RgbaImage::new(2, 2);
        img.put_pixel(1, 0, image::Rgba([255, 0, 0, 255]));
        let png = dib::png(&img).unwrap();
        let copy = Copy {
            text: Some("Xin chào".into()),
            html: Some("<b>Xin chào</b>".into()),
            rtf: Some(b"{\\rtf1 x}".to_vec()),
            png: Some(png.clone()),
            svg: Some(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec()),
            own: vec![("Craft Clipboard Test".into(), vec![1, 2, 3])],
            ..Copy::default()
        };
        if crate::write(&copy).is_err() {
            // No interactive desktop (a service account): nothing to check.
            return;
        }
        let seq = crate::sequence();
        assert_eq!(crate::text().as_deref(), Some("Xin chào"));
        assert_eq!(crate::html().unwrap().fragment, "<b>Xin chào</b>");
        assert_eq!(crate::rtf().as_deref(), Some(&b"{\\rtf1 x}"[..]));
        assert_eq!(crate::bitmap(), Some(png));
        assert_eq!(crate::picture().unwrap().mime, media::PNG);
        assert_eq!(crate::own("Craft Clipboard Test"), Some(vec![1, 2, 3]));
        let holds = crate::holds();
        assert!(holds.text && holds.html && holds.rtf && holds.bitmap && holds.svg && !holds.files);
        // Both bitmaps are there for programs that don't read PNG.
        let alpha = {
            let _open = open().unwrap();
            read_format(formats::CF_DIBV5).unwrap()
        };
        assert_eq!(dib::decode(&alpha).unwrap().get_pixel(1, 0).0, [255, 0, 0, 255]);
        assert_eq!(crate::sequence(), seq, "reading doesn't change the clipboard");

        // Files, as Explorer copies them.
        let dir = std::env::temp_dir().join(format!("craft-clipboard-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ảnh.png");
        std::fs::write(&file, dib::png(&img).unwrap()).unwrap();
        crate::write(&Copy { files: vec![file.clone()], ..Copy::default() }).unwrap();
        assert_eq!(crate::files(), [file]);
        assert!(crate::text().is_none() && crate::bitmap().is_none());
        assert_eq!(crate::picture().unwrap().mime, media::PNG);
        crate::write(&Copy::default()).unwrap();
        assert!(crate::files().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
