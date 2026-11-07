//! The system clipboard as every Craft app reads and writes it, so a copy in one pastes in the
//! others and in any other program, and what a web page or an office suite copied pastes in them.
//!
//! - **Standard formats** (tier 1), written together by every Copy and read by every Paste: plain
//!   text, HTML (Windows "HTML Format"), RTF, a picture as PNG and as the two device-independent
//!   bitmaps (`CF_DIBV5` with alpha, an opaque `CF_DIB` for programs that ignore alpha), SVG
//!   (`image/svg+xml`) and files copied in a file manager (`CF_HDROP`).
//! - **An app's own format** (tier 2), beside them, for what only the same kind of app understands
//!   whole (a slide, cells with their formulas): registered by name, read back by the same name.
//!
//! Copying anywhere else replaces everything at once, so a Paste never mixes two copies.
//! On macOS and Linux the clipboard is reached through `arboard`: text, HTML, a bitmap and files.
//! The web build has none of its own (`none.rs`).
//!
//! This crate is the same in every Craft app (`crates/craft-clipboard`); change it in one place and
//! copy it to the others.

mod cf_html;
pub mod dib;
pub mod media;
#[cfg(target_arch = "wasm32")]
mod none;
#[cfg(not(any(windows, target_arch = "wasm32")))]
mod portable;
#[cfg(windows)]
mod win;

use std::path::PathBuf;

#[cfg(target_arch = "wasm32")]
use none as sys;
#[cfg(not(any(windows, target_arch = "wasm32")))]
use portable as sys;
#[cfg(windows)]
use win as sys;

pub use cf_html::{decode as decode_html_format, encode as encode_html_format};

/// Largest clipboard item read (another program's data is untrusted).
pub const MAX_READ: usize = 256 << 20;

/// Everything one Copy offers. Empty fields are left out; a picture goes out as PNG and both
/// bitmaps.
#[derive(Clone, Debug, Default)]
pub struct Copy {
    pub text: Option<String>,
    /// An HTML fragment (no `<html>` or `<body>` needed).
    pub html: Option<String>,
    /// The page or file the HTML came from, for its relative links.
    pub source_url: Option<String>,
    pub rtf: Option<Vec<u8>>,
    /// A picture as PNG bytes.
    pub png: Option<Vec<u8>>,
    pub svg: Option<Vec<u8>>,
    /// Files, as a file manager copies them.
    pub files: Vec<PathBuf>,
    /// The app's own formats: registered name, bytes.
    pub own: Vec<(String, Vec<u8>)>,
}

impl Copy {
    pub fn text(text: impl Into<String>) -> Self {
        Self { text: Some(text.into()), ..Self::default() }
    }

    fn is_empty(&self) -> bool {
        self.text.is_none()
            && self.html.is_none()
            && self.rtf.is_none()
            && self.png.is_none()
            && self.svg.is_none()
            && self.files.is_empty()
            && self.own.is_empty()
    }
}

/// HTML another program copied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Html {
    /// The whole document: a browser puts a copied table row's `<table>` here, outside the
    /// fragment.
    pub document: String,
    /// What was selected.
    pub fragment: String,
    /// The page it was copied from.
    pub source_url: Option<String>,
}

impl Html {
    /// The pictures it carries that are here without the network (`data:` URIs, local files).
    pub fn local_pictures(&self) -> Vec<Picture> {
        media::local_pictures(&self.fragment).into_iter().map(|(mime, data)| Picture { mime, data }).collect()
    }

    /// The `<img>` sources of the fragment, resolved against the page.
    pub fn image_sources(&self) -> Vec<media::Source> {
        media::image_sources(&self.fragment).iter().filter_map(|s| media::source(s, self.source_url.as_deref())).collect()
    }
}

/// A picture: its bytes and their MIME type ([`media`]'s constants).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub mime: &'static str,
    pub data: Vec<u8>,
}

/// What the clipboard holds, cheaply (no data is read).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Holds {
    pub text: bool,
    pub html: bool,
    pub rtf: bool,
    /// PNG or a bitmap.
    pub bitmap: bool,
    pub svg: bool,
    pub files: bool,
}

impl Holds {
    /// Something a picture paste can use: a bitmap, SVG, or files (which may be pictures).
    pub fn picture(&self) -> bool {
        self.bitmap || self.svg || self.files
    }
}

/// Put `copy` on the clipboard, replacing what was there. Every format is tried; the first failure
/// is reported.
pub fn write(copy: &Copy) -> Result<(), String> {
    if copy.is_empty() {
        return sys::clear();
    }
    sys::write(copy)
}

/// The clipboard's plain text, when it has some.
pub fn text() -> Option<String> {
    sys::text().filter(|t| !t.is_empty())
}

/// The clipboard's HTML.
pub fn html() -> Option<Html> {
    sys::html()
}

/// The clipboard's RTF.
pub fn rtf() -> Option<Vec<u8>> {
    sys::rtf()
}

/// The clipboard's bitmap as PNG: its PNG as it is, else a device-independent bitmap converted.
pub fn bitmap() -> Option<Vec<u8>> {
    sys::bitmap()
}

/// The clipboard's SVG.
pub fn svg() -> Option<Vec<u8>> {
    sys::svg()
}

/// Files copied in a file manager.
pub fn files() -> Vec<PathBuf> {
    sys::files()
}

/// The clipboard's picture, from the best format it has: a bitmap (PNG first), SVG, the first copied
/// file that is a picture, else the one picture of its HTML (`data:` or a local file).
pub fn picture() -> Option<Picture> {
    if let Some(data) = bitmap() {
        return Some(Picture { mime: media::PNG, data });
    }
    if let Some(data) = svg() {
        return Some(Picture { mime: media::SVG, data });
    }
    if let Some((mime, data)) = files().iter().find_map(|p| media::picture_file(p)) {
        return Some(Picture { mime, data });
    }
    let mut pictures = html()?.local_pictures();
    (pictures.len() == 1).then(|| pictures.remove(0))
}

/// An app's own format, by its registered name.
pub fn own(name: &str) -> Option<Vec<u8>> {
    sys::own(name)
}

/// Whether the clipboard holds an app's own format.
pub fn holds_own(name: &str) -> bool {
    sys::holds_own(name)
}

/// What the clipboard holds.
pub fn holds() -> Holds {
    sys::holds()
}

/// A number that changes with every change of the clipboard (Windows), so an app knows whether its
/// own copy is still there. `None` where the platform has none.
pub fn sequence() -> Option<u32> {
    sys::sequence()
}

/// The names of the formats on the clipboard, for diagnostics.
pub fn format_names() -> Vec<String> {
    sys::format_names()
}
