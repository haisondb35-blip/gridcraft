//! What a picture's bytes are, and the pictures an HTML copy carries: `<img>` sources that are
//! `data:` URIs or local files paste as they are; web addresses are only listed (fetching them is
//! the app's choice).

use std::io::Read;
use std::path::{Path, PathBuf};

/// Largest picture read from a file or a `data:` URI.
pub const MAX_PICTURE: u64 = 256 << 20;

pub const PNG: &str = "image/png";
pub const JPEG: &str = "image/jpeg";
pub const GIF: &str = "image/gif";
pub const BMP: &str = "image/bmp";
pub const WEBP: &str = "image/webp";
pub const TIFF: &str = "image/tiff";
pub const SVG: &str = "image/svg+xml";
pub const EMF: &str = "image/emf";
pub const WMF: &str = "image/wmf";

/// The picture type of `head` (a file's first bytes), using `name`'s extension for SVG, which has
/// no fixed signature. `None`: not a picture.
pub fn picture_type(head: &[u8], name: &str) -> Option<&'static str> {
    let ext = Path::new(name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let mime = match head {
        [0x89, b'P', b'N', b'G', ..] => PNG,
        [0xFF, 0xD8, 0xFF, ..] => JPEG,
        [b'G', b'I', b'F', b'8', ..] => GIF,
        [b'B', b'M', ..] => BMP,
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => WEBP,
        [b'I', b'I', 42, 0, ..] | [b'M', b'M', 0, 42, ..] => TIFF,
        [0xD7, 0xCD, 0xC6, 0x9A, ..] | [1 | 2, 0, 9, 0, ..] => WMF,
        [1, 0, 0, 0, ..] if head.get(40..44) == Some(b" EMF") => EMF,
        _ if matches!(ext.as_deref(), Some("svg")) => SVG,
        _ => return None,
    };
    Some(mime)
}

/// A file's bytes when it is a picture no larger than [`MAX_PICTURE`], with its type.
pub fn picture_file(path: &Path) -> Option<(&'static str, Vec<u8>)> {
    let mut file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut data = Vec::new();
    (&mut file).take(MAX_PICTURE + 1).read_to_end(&mut data).ok()?;
    if data.len() as u64 > MAX_PICTURE || data.is_empty() {
        return None;
    }
    let mime = picture_type(&data[..data.len().min(64)], &path.to_string_lossy())?;
    Some((mime, data))
}

/// Where an `<img>` points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Bytes carried in the HTML (`data:`), with their type.
    Data(&'static str, Vec<u8>),
    /// A file on this computer (`file:` URL or a bare path).
    File(PathBuf),
    /// A web address, as written.
    Web(String),
}

/// The `src` of every `<img>` in `html`, in order, with entities decoded.
pub fn image_sources(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut out = vec![];
    let mut at = 0;
    while let Some(i) = lower[at..].find("<img") {
        let start = at + i + 4;
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        if let Some(src) = attribute(&html[start..end], "src") {
            out.push(decode_entities(&src));
        }
        at = end;
    }
    out
}

/// The value of attribute `name` in a tag's attribute text.
fn attribute(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let b = lower.as_bytes();
    let mut i = 0;
    while let Some(p) = lower[i..].find(name) {
        let s = i + p;
        let e = s + name.len();
        i = e;
        let starts_word = s == 0 || b[s - 1].is_ascii_whitespace();
        let rest = lower[e..].trim_start();
        if !starts_word || !rest.starts_with('=') {
            continue;
        }
        let v = attrs[attrs.len() - rest.len() + 1..].trim_start();
        return Some(match v.chars().next() {
            Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or("").to_string(),
            _ => v.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("").to_string(),
        });
    }
    None
}

/// The common character references (`&amp;`, `&quot;`, numeric ones).
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "lt" => Some('<'),
            "gt" => Some('>'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .map(|h| u32::from_str_radix(h, 16))
                .or_else(|| name.strip_prefix('#').map(str::parse::<u32>))
                .and_then(Result::ok)
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `%XX` escapes decoded, as bytes.
fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Standard or URL-safe base64, whitespace ignored; `None` on any other character.
fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut n) = (0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((acc >> n) as u8);
        }
    }
    Some(out)
}

/// A `data:` URI's picture: its bytes and type (from its bytes, else its declared type).
fn data_uri(src: &str) -> Option<(&'static str, Vec<u8>)> {
    let rest = src.get(..5).filter(|p| p.eq_ignore_ascii_case("data:")).map(|_| &src[5..])?;
    let (meta, payload) = rest.split_once(',')?;
    if payload.len() as u64 > MAX_PICTURE * 4 / 3 + 4 {
        return None;
    }
    let meta = meta.to_ascii_lowercase();
    let data = if meta.ends_with(";base64") { base64(payload)? } else { percent_decode(payload) };
    let declared = meta.split(';').next().unwrap_or("");
    let name = if declared == SVG { "x.svg" } else { "" };
    let mime = picture_type(&data[..data.len().min(64)], name)?;
    Some((mime, data))
}

/// A `file:` URL's path (`file:///C:/a%20b.png`, `file://server/share/x.png`).
fn file_url(src: &str) -> Option<PathBuf> {
    let rest = src.get(..5).filter(|p| p.eq_ignore_ascii_case("file:")).map(|_| &src[5..])?;
    let path = String::from_utf8(percent_decode(rest)).ok()?;
    // Windows: `file:///C:/x`; elsewhere `file:///x`.
    let local = |p: &str| if cfg!(windows) { p.to_string() } else { format!("/{p}") };
    let path = if let Some(p) = path.strip_prefix("///") {
        local(p)
    } else if let Some(p) = path.strip_prefix("//localhost/") {
        local(p)
    } else if let Some(unc) = path.strip_prefix("//").filter(|u| !u.is_empty()) {
        if cfg!(windows) { format!(r"\\{}", unc.replace('/', "\\")) } else { format!("//{unc}") }
    } else {
        path.clone()
    };
    Some(PathBuf::from(path))
}

/// Where `src` points, relative web addresses resolved against `base` (the copy's page).
pub fn source(src: &str, base: Option<&str>) -> Option<Source> {
    let src = src.trim();
    if src.is_empty() {
        return None;
    }
    if let Some((mime, data)) = data_uri(src) {
        return Some(Source::Data(mime, data));
    }
    if let Some(path) = file_url(src) {
        return Some(Source::File(path));
    }
    let lower = src.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(Source::Web(src.to_string()));
    }
    if lower.starts_with("data:") || lower.starts_with("blob:") || lower.starts_with("cid:") {
        return None;
    }
    // A Windows path an office suite wrote (`C:\Users\…\clip_image001.png`).
    if src.get(1..3) == Some(":\\") || src.starts_with(r"\\") {
        return Some(Source::File(PathBuf::from(src)));
    }
    let base = base?;
    let scheme_end = base.find("://")? + 3;
    let origin_end = base[scheme_end..].find('/').map_or(base.len(), |i| scheme_end + i);
    let url = if let Some(rest) = src.strip_prefix("//") {
        format!("{}{rest}", &base[..scheme_end])
    } else if src.starts_with('/') {
        format!("{}{src}", &base[..origin_end])
    } else {
        let dir_end = base[origin_end..].rfind('/').map_or(base.len(), |i| origin_end + i + 1);
        let dir = &base[..dir_end.max(origin_end)];
        if dir.ends_with('/') { format!("{dir}{src}") } else { format!("{dir}/{src}") }
    };
    Some(Source::Web(url))
}

/// The pictures of `html` that are here without the network: `data:` URIs and local files.
pub fn local_pictures(html: &str) -> Vec<(&'static str, Vec<u8>)> {
    image_sources(html)
        .iter()
        .filter_map(|s| match source(s, None)? {
            Source::Data(mime, data) => Some((mime, data)),
            Source::File(path) => picture_file(&path),
            Source::Web(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_known_by_their_bytes() {
        assert_eq!(picture_type(b"\x89PNG\r\n", ""), Some(PNG));
        assert_eq!(picture_type(&[0xFF, 0xD8, 0xFF, 0xE0], "a.png"), Some(JPEG));
        assert_eq!(picture_type(b"RIFF\0\0\0\0WEBPVP8 ", ""), Some(WEBP));
        assert_eq!(picture_type(b"<svg/>", "Logo.SVG"), Some(SVG));
        assert_eq!(picture_type(b"hello", "notes.txt"), None);
        let mut emf = vec![1, 0, 0, 0];
        emf.resize(40, 0);
        emf.extend(b" EMF");
        assert_eq!(picture_type(&emf, ""), Some(EMF));
    }

    #[test]
    fn img_sources_are_found_with_any_quoting() {
        let html = r#"<p>x</p><IMG alt="a" SRC="a.png?x=1&amp;y=2"><img data-src="no" src='b.gif' /><img src=c.jpg width=3><img alt=none>"#;
        assert_eq!(image_sources(html), ["a.png?x=1&y=2", "b.gif", "c.jpg"]);
    }

    #[test]
    fn data_uris_and_files_paste_without_the_network() {
        // A 1×1 PNG.
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";
        let html = format!(r#"<img src="data:image/png;base64,{png}"><img src="https://example.com/x.png">"#);
        let found = local_pictures(&html);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, PNG);
        assert!(crate::dib::decode_png(&found[0].1).is_some());
        let svg = source("data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%2F%3E", None);
        assert!(matches!(svg, Some(Source::Data(SVG, ref d)) if d.starts_with(b"<svg")));
        // Not a picture, not base64.
        assert_eq!(source("data:text/plain,hi", None), None);
        assert_eq!(source("data:image/png;base64,***", None), None);

        let dir = std::env::temp_dir().join(format!("craft-clipboard-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ảnh 1.png");
        std::fs::write(&file, base64(png).unwrap()).unwrap();
        let url = format!("file:///{}", file.to_string_lossy().replace('\\', "/").trim_start_matches('/')).replace(' ', "%20");
        let html = format!(r#"<img src="{url}">"#);
        assert_eq!(local_pictures(&html).len(), 1, "{url}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn web_addresses_resolve_against_the_page() {
        let base = Some("https://vi.wikipedia.org/wiki/H%C3%A0_N%E1%BB%99i");
        let web = |s| match source(s, base) {
            Some(Source::Web(u)) => u,
            other => panic!("{other:?}"),
        };
        assert_eq!(web("//upload.example.org/a.jpg"), "https://upload.example.org/a.jpg");
        assert_eq!(web("/static/b.png"), "https://vi.wikipedia.org/static/b.png");
        assert_eq!(web("c.png"), "https://vi.wikipedia.org/wiki/c.png");
        assert_eq!(source("c.png", None), None);
        assert_eq!(source("blob:https://x/1", base), None);
        assert_eq!(source(r"C:\Users\a\clip_image001.png", None), Some(Source::File(PathBuf::from(r"C:\Users\a\clip_image001.png"))));
    }

    #[test]
    fn entities_and_escapes_decode() {
        assert_eq!(decode_entities("a&amp;b&#39;c&#x41;&bogus;&"), "a&b'cA&bogus;&");
        assert_eq!(percent_decode("a%20b%zz%4"), b"a b%zz%4");
        assert_eq!(base64("aGk=").unwrap(), b"hi");
        assert_eq!(base64("aG\nk").unwrap(), b"hi");
    }
}
