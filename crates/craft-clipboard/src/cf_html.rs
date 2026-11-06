//! Windows' "HTML Format": a header of byte offsets (`StartHTML`, `EndHTML`, `StartFragment`,
//! `EndFragment`, optionally `SourceURL`) before a UTF-8 HTML document whose copied part sits
//! between `<!--StartFragment-->` and `<!--EndFragment-->`. Browsers and office suites write it
//! whenever something is copied with formatting.

use crate::Html;

const MARK_START: &str = "<!--StartFragment-->";
const MARK_END: &str = "<!--EndFragment-->";

/// `fragment` (and the page it came from) as "HTML Format" bytes.
pub fn encode(fragment: &str, source_url: Option<&str>) -> Vec<u8> {
    // Every offset is 10 digits, so the header's length is known before the offsets are.
    let source = source_url.filter(|u| !u.contains(['\r', '\n'])).map(|u| format!("SourceURL:{u}\r\n")).unwrap_or_default();
    let header_len = "Version:0.9\r\nStartHTML:\r\nEndHTML:\r\nStartFragment:\r\nEndFragment:\r\n".len() + 4 * 10 + source.len();
    let before = format!("<html><head><meta charset=\"utf-8\"></head><body>{MARK_START}");
    let after = format!("{MARK_END}</body></html>");
    let start_html = header_len;
    let start_fragment = start_html + before.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + after.len();
    let header = format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n{source}"
    );
    debug_assert_eq!(header.len(), header_len);
    let mut out = Vec::with_capacity(end_html);
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(before.as_bytes());
    out.extend_from_slice(fragment.as_bytes());
    out.extend_from_slice(after.as_bytes());
    out
}

/// A header field's value: the text after `name:` on its own line within the header.
fn field<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.lines().find_map(|l| l.strip_prefix(name)?.strip_prefix(':')).map(str::trim)
}

/// An offset field, `None` when missing, negative (-1: "not given") or past `len`.
fn offset(header: &str, name: &str, len: usize) -> Option<usize> {
    field(header, name)?.parse::<usize>().ok().filter(|&o| o <= len)
}

/// "HTML Format" bytes as the document, its copied fragment and its source page. Offsets that are
/// missing or out of range fall back to the fragment markers, then to the whole text: other apps'
/// headers are untrusted.
pub fn decode(bytes: &[u8]) -> Option<Html> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let text = String::from_utf8_lossy(&bytes[..end]);
    let text = text.as_ref();
    // The header ends where the HTML starts; without a usable StartHTML, at the first '<'.
    let first_tag = text.find('<').unwrap_or(text.len());
    let header = &text[..first_tag];
    let at = |name| offset(header, name, text.len()).filter(|&o| text.is_char_boundary(o));
    let start_html = at("StartHTML").unwrap_or(first_tag);
    let end_html = at("EndHTML").filter(|&o| o >= start_html).unwrap_or(text.len());
    let document = text.get(start_html..end_html)?;
    let by_offsets = match (at("StartFragment"), at("EndFragment")) {
        (Some(s), Some(e)) if start_html <= s && s <= e && e <= end_html => text.get(s..e),
        _ => None,
    };
    let by_marks = || {
        let s = document.find(MARK_START)? + MARK_START.len();
        let e = document[s..].find(MARK_END)? + s;
        document.get(s..e)
    };
    let fragment = by_offsets.or_else(by_marks).unwrap_or(document);
    if fragment.trim().is_empty() && document.trim().is_empty() {
        return None;
    }
    let source_url = field(header, "SourceURL").filter(|u| !u.is_empty()).map(str::to_string);
    Some(Html { document: document.to_string(), fragment: fragment.to_string(), source_url })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_encoded_decodes_back_with_vietnamese_text() {
        let frag = "<p>Bảng lương <b>tháng 9</b></p><table><tr><td>Nguyễn Văn A</td><td>1.000</td></tr></table>";
        let bytes = encode(frag, Some("https://example.com/a"));
        let html = decode(&bytes).unwrap();
        assert_eq!(html.fragment, frag);
        assert_eq!(html.source_url.as_deref(), Some("https://example.com/a"));
        assert!(html.document.starts_with("<html>") && html.document.ends_with("</html>"));
        // The offsets point where they say.
        let text = std::str::from_utf8(&bytes).unwrap();
        let start: usize = field(text, "StartFragment").unwrap().parse().unwrap();
        assert!(text[start..].starts_with("<p>Bảng"));
        let end: usize = field(text, "EndHTML").unwrap().parse().unwrap();
        assert_eq!(end, bytes.len());
    }

    #[test]
    fn a_browsers_copy_keeps_the_table_around_its_rows() {
        let doc = "<html><body><table><tbody><!--StartFragment--><tr><td>1</td></tr><!--EndFragment--></tbody></table></body></html>";
        let header_len = 105;
        let s = header_len + doc.find("<tr>").unwrap();
        let e = header_len + doc.find("<!--EndFragment").unwrap();
        let mut text = format!(
            "Version:0.9\r\nStartHTML:{header_len:010}\r\nEndHTML:{:010}\r\nStartFragment:{s:010}\r\nEndFragment:{e:010}\r\n",
            header_len + doc.len()
        );
        assert_eq!(text.len(), header_len);
        text.push_str(doc);
        text.push('\0');
        let html = decode(text.as_bytes()).unwrap();
        assert_eq!(html.fragment, "<tr><td>1</td></tr>");
        assert!(html.document.contains("<table>"));
        assert_eq!(html.source_url, None);
    }

    #[test]
    fn broken_headers_fall_back_to_the_markers_then_the_text() {
        let lying =
            "Version:0.9\r\nStartHTML:-1\r\nEndHTML:-1\r\nStartFragment:99999\r\nEndFragment:3\r\n<p><!--StartFragment-->hi<!--EndFragment--></p>";
        assert_eq!(decode(lying.as_bytes()).unwrap().fragment, "hi");
        assert_eq!(decode(b"<b>bare</b>").unwrap().fragment, "<b>bare</b>");
        // Offsets inside a multi-byte character are refused, not sliced.
        let mid = "Version:0.9\r\nStartFragment:0000000066\r\nEndFragment:0000000068\r\n<p>ắắắắ</p>";
        assert_eq!(decode(mid.as_bytes()).unwrap().fragment, "<p>ắắắắ</p>");
        assert!(decode(b"").is_none());
        assert!(decode(b"Version:0.9\r\n").is_none());
    }
}
