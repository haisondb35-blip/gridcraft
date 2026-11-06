//! Save chat with file: the assistant's conversation about a document, written into the document
//! itself when the person turns it on (off by default), so it travels with the file and comes
//! back when the file is opened on another computer.
//!
//! Office Open XML files are ZIP packages: the conversation is one more part,
//! `artcraft/assistant.json` (portable form, without this computer's conversation id, see
//! `Memory::to_json`), with a `json` content type. Excel, Word and PowerPoint ignore a part
//! nothing refers to. Saving with the option off takes the part out again.
//!
//! The same file lives in GridCraft, WordCraft and DeckCraft (`E:\Github\_shared\assistant_embed.rs`).

use std::io::{Cursor, Read, Write};

use serde_json::Value;

/// The part that holds the conversation.
pub const PART: &str = "artcraft/assistant.json";
/// The largest conversation part read back.
const MAX_PART: u64 = 8 << 20;

/// Files the conversation can be saved in.
pub fn can_embed(path: &str) -> bool {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    matches!(
        ext.as_str(),
        "xlsx" | "xlsm" | "xltx" | "xltm" | "docx" | "docm" | "dotx" | "dotm" | "pptx" | "pptm" | "potx" | "potm" | "ppsx" | "ppsm"
    )
}

/// The conversation saved in the file at `path`, if it has one.
pub fn read(path: &str) -> Option<Value> {
    if !can_embed(path) {
        return None;
    }
    let f = std::fs::File::open(path).ok()?;
    read_from(f)
}

fn read_from(r: impl Read + std::io::Seek) -> Option<Value> {
    let mut zip = zip::ZipArchive::new(r).ok()?;
    let part = zip.by_name(PART).ok()?;
    let mut text = String::new();
    part.take(MAX_PART).read_to_string(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}

/// The desktop app's save hook: a file saved with the conversation `shelf` has for it (or
/// without one, the option off). Other kinds of file are saved as they are.
pub fn hook(shelf: &std::sync::Mutex<super::memory::Shelf>, path: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    if !can_embed(path) {
        return None;
    }
    let chat = shelf.lock().ok().and_then(|s| s.chat_for(path));
    embed(bytes, chat.as_ref())
}

/// `[Content_Types].xml` with a default type for `.json` parts.
fn with_json_type(xml: &str) -> String {
    if xml.contains("Extension=\"json\"") {
        return xml.to_string();
    }
    let Some(at) = xml.find("<Types").and_then(|i| xml[i..].find('>').map(|j| i + j + 1)) else { return xml.to_string() };
    format!("{}<Default Extension=\"json\" ContentType=\"application/json\"/>{}", &xml[..at], &xml[at..])
}

/// The package `bytes` with `chat` as its conversation part (replacing one already there), or
/// without one when `chat` is None. None when nothing changes or `bytes` isn't a ZIP package
/// (an encrypted file, say): the file is saved as it was made.
pub fn embed(bytes: &[u8], chat: Option<&Value>) -> Option<Vec<u8>> {
    let mut zin = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let has = zin.index_for_name(PART).is_some();
    if chat.is_none() && !has {
        return None;
    }
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let deflate = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for i in 0..zin.len() {
        let name = zin.name_for_index(i)?.to_string();
        if name == PART {
            continue;
        }
        if name == "[Content_Types].xml" && chat.is_some() {
            let mut xml = String::new();
            zin.by_index(i).ok()?.read_to_string(&mut xml).ok()?;
            out.start_file(name, deflate).ok()?;
            out.write_all(with_json_type(&xml).as_bytes()).ok()?;
            continue;
        }
        out.raw_copy_file(zin.by_index_raw(i).ok()?).ok()?;
    }
    if let Some(c) = chat {
        out.start_file(PART, deflate).ok()?;
        out.write_all(c.to_string().as_bytes()).ok()?;
    }
    Some(out.finish().ok()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn package() -> Vec<u8> {
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        out.start_file("[Content_Types].xml", o).unwrap();
        out.write_all(br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#).unwrap();
        out.start_file("xl/workbook.xml", o).unwrap();
        out.write_all(b"<workbook/>").unwrap();
        out.finish().unwrap().into_inner()
    }

    #[test]
    fn a_chat_goes_into_the_package_and_comes_out_again() {
        let chat = json!({"version": 1, "messages": [{"role": "user", "text": "Tổng cột B"}]});
        let with = embed(&package(), Some(&chat)).unwrap();
        assert_eq!(read_from(Cursor::new(&with)), Some(chat.clone()));
        let mut zip = zip::ZipArchive::new(Cursor::new(&with)).unwrap();
        let mut types = String::new();
        zip.by_name("[Content_Types].xml").unwrap().read_to_string(&mut types).unwrap();
        assert_eq!(types.matches("Extension=\"json\"").count(), 1);
        let mut wb = String::new();
        zip.by_name("xl/workbook.xml").unwrap().read_to_string(&mut wb).unwrap();
        assert_eq!(wb, "<workbook/>", "the document's own parts are kept as they were");
        // Saved again: replaced, not doubled; with the option off: taken out.
        let again = embed(&with, Some(&json!({"messages": []}))).unwrap();
        assert_eq!(zip::ZipArchive::new(Cursor::new(&again)).unwrap().len(), 3);
        let without = embed(&with, None).unwrap();
        assert_eq!(read_from(Cursor::new(&without)), None);
        assert_eq!(embed(&package(), None), None, "nothing to do");
        assert_eq!(embed(b"not a zip", Some(&chat)), None);
        assert!(can_embed("C:/Docs/Báo cáo.XLSX") && can_embed("a.pptx") && !can_embed("a.xls") && !can_embed("a.pdf"));
    }
}
