//! macOS and Linux through `arboard`: a copy sets HTML with its text, else a bitmap, else the
//! text; apps' own formats stay inside the app there.

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::PathBuf;

use crate::{Copy, Holds, Html, cf_html, dib};

thread_local! {
    /// Kept open: on Linux copied data lives as long as its handle.
    static CLIPBOARD: RefCell<Option<arboard::Clipboard>> = const { RefCell::new(None) };
}

fn with<T>(f: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>) -> Result<T, String> {
    CLIPBOARD.with(|cell| {
        let mut cell = cell.try_borrow_mut().map_err(|e| e.to_string())?;
        if cell.is_none() {
            *cell = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
        }
        let cb = cell.as_mut().ok_or("no clipboard")?;
        f(cb).map_err(|e| e.to_string())
    })
}

pub fn clear() -> Result<(), String> {
    with(arboard::Clipboard::clear)
}

pub fn write(copy: &Copy) -> Result<(), String> {
    if let Some(html) = &copy.html {
        return with(|cb| cb.set_html(html.as_str(), copy.text.as_deref()));
    }
    if let Some(img) = copy.png.as_deref().and_then(dib::decode_png) {
        let (width, height) = (img.width() as usize, img.height() as usize);
        return with(|cb| cb.set_image(arboard::ImageData { width, height, bytes: Cow::Owned(img.into_raw()) }));
    }
    match &copy.text {
        Some(t) => with(|cb| cb.set_text(t.as_str())),
        None => clear(),
    }
}

pub fn text() -> Option<String> {
    with(arboard::Clipboard::get_text).ok()
}

pub fn html() -> Option<Html> {
    let html = with(|cb| cb.get().html()).ok()?;
    // Some programs hand over the Windows-style header even here.
    cf_html::decode(html.as_bytes())
}

pub fn rtf() -> Option<Vec<u8>> {
    None
}

pub fn svg() -> Option<Vec<u8>> {
    None
}

pub fn bitmap() -> Option<Vec<u8>> {
    let img = with(arboard::Clipboard::get_image).ok()?;
    let (w, h) = (u32::try_from(img.width).ok()?, u32::try_from(img.height).ok()?);
    if w > dib::MAX_SIDE || h > dib::MAX_SIDE {
        return None;
    }
    dib::png(&image::RgbaImage::from_raw(w, h, img.bytes.into_owned())?)
}

pub fn files() -> Vec<PathBuf> {
    with(|cb| cb.get().file_list()).unwrap_or_default()
}

pub fn own(_name: &str) -> Option<Vec<u8>> {
    None
}

pub fn holds_own(_name: &str) -> bool {
    false
}

/// Text only: asking for a bitmap here costs as much as reading it.
pub fn holds() -> Holds {
    Holds { text: text().is_some_and(|t| !t.is_empty()), ..Holds::default() }
}

pub fn sequence() -> Option<u32> {
    None
}

pub fn format_names() -> Vec<String> {
    vec![]
}
