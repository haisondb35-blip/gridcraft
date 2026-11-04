use super::{Json, SheetApp, json};

impl SheetApp {
    /// Copy or cut the selected cells to the system clipboard. Engine-only commands keep
    /// their existing internal clipboard behavior and do not write to the host clipboard.
    pub fn copy_to_clipboard(&mut self, ctx: &egui::Context, command: &str) {
        // A selected picture goes out as a picture, as every program reads one.
        if self.copy_selected_picture(command) {
            return;
        }
        // Only ask the engine for HTML when the host can publish it.
        let result = match self.run_command(command, json!({"html": self.services.copy_html.is_some()})) {
            Ok(result) => result,
            Err(e) => return self.alert(e),
        };
        let Some(text) = result.get("text").and_then(Json::as_str) else { return };
        // The system clipboard gets this copy, so its text decides what Paste pastes.
        self.clipboard_text_at_copy = None;
        self.cut_text = (command == "edit.cut").then(|| text.replace("\r\n", "\n"));
        if let Some(html) = result.get("html").and_then(Json::as_str)
            && let Some(copy_html) = &mut self.services.copy_html
        {
            match copy_html(html, text) {
                Ok(()) => return,
                Err(error) => log::warn!("HTML clipboard unavailable; copying plain text: {error}"),
            }
        }
        ctx.copy_text(text.to_string());
    }

    /// Paste text that came from the system clipboard (Cmd/Ctrl+V, or the ribbon and menu Paste),
    /// with the clipboard's HTML flavour when the host can read it.
    ///
    /// Excel pastes a cut once: after GridCraft's cut has been pasted, the system clipboard text
    /// that came with it is not pasted again over the moved block; GridCraft's own (empty)
    /// clipboard is pasted instead, which reports "Nothing to paste.". Other text means the
    /// clipboard has moved on, so the cut's text is forgotten and pastes normally later.
    pub fn paste_external(&mut self, text: String) {
        if Self::same_clipboard_text(Some(&text), self.cut_text.as_deref()) {
            if self.session.clipboard.is_none() {
                return self.run_or_alert("edit.paste", json!({}));
            }
        } else {
            self.cut_text = None;
        }
        let html = self.services.paste_html.as_mut().and_then(|read| read());
        let params = match html {
            Some(html) => json!({"text": text, "html": html}),
            None => json!({"text": text}),
        };
        self.run_or_alert("edit.paste", params);
    }

    /// Ribbon and menu Paste: paste the text on the system clipboard, as Cmd/Ctrl+V does. With no
    /// text there but a picture, the picture is pasted. With neither, no host clipboard (web), or
    /// the text that was there when GridCraft last copied without publishing, GridCraft's own
    /// clipboard is pasted.
    pub fn paste_from_clipboard(&mut self) {
        match self.system_clipboard_text() {
            Some(text) if self.session.clipboard.is_none() || self.clipboard_text_at_copy.as_ref() != Some(&text) => self.paste_external(text),
            None if self.paste_picture() => {}
            _ => self.run_or_alert("edit.paste", json!({})),
        }
    }

    /// The system clipboard's plain text, with the line endings of egui's `Event::Paste` text.
    /// `None` when it holds no text or the host can't read it.
    pub(super) fn system_clipboard_text(&mut self) -> Option<String> {
        self.services.paste_text.as_mut().and_then(|read| read()).filter(|t| !t.is_empty()).map(|t| t.replace("\r\n", "\n"))
    }

    /// Whether two clipboard texts are the same copy: trailing line breaks don't count, as in the
    /// engine's `edit.paste`, and CRLF matches LF (egui's `Event::Paste` text may keep the host's).
    fn same_clipboard_text(a: Option<&str>, b: Option<&str>) -> bool {
        fn chars(t: &str) -> impl Iterator<Item = char> + '_ {
            t.trim_end_matches(['\r', '\n']).chars().filter(|c| *c != '\r')
        }
        match (a, b) {
            (Some(a), Some(b)) => chars(a).eq(chars(b)),
            _ => false,
        }
    }

    /// The selected floating picture as PNG, and its id.
    fn selected_picture_png(&self) -> Option<(Vec<u8>, u32)> {
        let id = self.selected_image()?;
        let sh = self.session.active()?.wb.active()?;
        let im = sh.images.iter().find(|im| im.id == id)?;
        let png = picture_as_png(&im.data, &im.mime, im.anchor.width.max(im.anchor.height))?;
        Some((png, id))
    }

    /// Copy (or cut) the selected picture to the system clipboard. `false` when no picture is
    /// selected or the host has no picture clipboard: the cells are copied instead.
    fn copy_selected_picture(&mut self, command: &str) -> bool {
        if self.services.copy_picture.is_none() {
            return false;
        }
        let Some((png, id)) = self.selected_picture_png() else { return false };
        let Some(copy) = &mut self.services.copy_picture else { return false };
        if let Err(e) = copy(&png) {
            self.alert(format!("The picture couldn't be copied: {e}"));
            return true;
        }
        // The system clipboard now holds this picture, not GridCraft's cells.
        self.clipboard_text_at_copy = None;
        self.cut_text = None;
        if command == "edit.cut" {
            self.run_or_alert("object.delete", json!({"kind": "image", "id": id}));
            self.selected_object = None;
        }
        true
    }

    /// Paste the system clipboard's picture (a bitmap, a copied picture file, a web page's picture)
    /// over the active cell. `false` when it holds none.
    pub fn paste_picture(&mut self) -> bool {
        let Some(read) = &mut self.services.paste_picture else { return false };
        let Some((mime, data)) = read() else { return false };
        // Refuse a picture too large to paste before decoding its pixels (upstream's limits).
        if let Some((w, h)) = picture_size(&data)
            && let Err(e) = check_picture_size(w, h)
        {
            self.alert(e);
            return true;
        }
        let Some((mime, data)) = insertable_picture(&mime, data) else {
            self.alert("GridCraft can't show this kind of picture.".to_string());
            return true;
        };
        // A floating picture whose top-left corner is the active cell, as Excel does.
        let params = json!({"base64": gridcraft_engine::io::base64_encode(&data), "mime": mime});
        match self.run_command("insert.picture", params) {
            Ok(r) => self.select_object_id(r.get("image").and_then(Json::as_u64).and_then(|id| u32::try_from(id).ok())),
            Err(e) => self.alert(e),
        }
        true
    }
}

/// Longest side, in pixels, of a metafile picture drawn for the clipboard.
const METAFILE_COPY_SIDE: f32 = 2048.0;

/// A floating picture's bytes as PNG: PNG as it is, other bitmaps decoded, metafiles (EMF, WMF)
/// drawn at twice their size on the sheet (at most [`METAFILE_COPY_SIDE`]).
fn picture_as_png(data: &[u8], mime: &str, side: f32) -> Option<Vec<u8>> {
    if data.starts_with(b"\x89PNG") {
        return Some(data.to_vec());
    }
    let img = if matches!(mime, "image/x-emf" | "image/x-wmf" | "image/emf" | "image/wmf") {
        gridcraft_metafile::rasterize(data, (side * 2.0).clamp(64.0, METAFILE_COPY_SIDE) as u32)?
    } else {
        image::load_from_memory(data).ok()?.to_rgba8()
    };
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
    Some(out)
}

/// A pasted picture as `insert.picture` takes it: PNG, JPEG and metafiles as they are, other
/// bitmaps (GIF, BMP, WebP, TIFF) as PNG. `None`: not a picture GridCraft draws (SVG).
fn insertable_picture(mime: &str, data: Vec<u8>) -> Option<(&'static str, Vec<u8>)> {
    match mime {
        "image/png" => Some(("image/png", data)),
        "image/jpeg" => Some(("image/jpeg", data)),
        "image/emf" | "image/x-emf" => Some(("image/x-emf", data)),
        "image/wmf" | "image/x-wmf" => Some(("image/x-wmf", data)),
        "image/svg+xml" => None,
        _ => {
            let img = image::load_from_memory(&data).ok()?.to_rgba8();
            let mut out = vec![];
            img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
            Some(("image/png", out))
        }
    }
}

/// Largest clipboard picture pasted, per side and in total (an 8K screenshot fits).
const MAX_PICTURE_SIDE: u32 = 16_384;
const MAX_PICTURE_PIXELS: u64 = 64 * 1024 * 1024;

/// Refuse empty clipboard pictures and sizes that would exhaust memory.
fn check_picture_size(w: u32, h: u32) -> Result<(), String> {
    if w == 0 || h == 0 {
        return Err("The picture on the clipboard is empty.".into());
    }
    if w > MAX_PICTURE_SIDE || h > MAX_PICTURE_SIDE || u64::from(w) * u64::from(h) > MAX_PICTURE_PIXELS {
        return Err("The picture on the clipboard is too large to paste.".into());
    }
    Ok(())
}

/// A bitmap's width and height read from its header, without decoding it; `None` for data the
/// `image` crate can't size (metafiles, SVG).
fn picture_size(data: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().ok()?.into_dimensions().ok()
}
