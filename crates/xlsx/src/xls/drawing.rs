//! Pictures in `.xls` files, from the Office Drawing records ([MS-ODRAW]): the workbook's
//! MsoDrawingGroup holds the pictures (a BStore of FBSE entries, each with its BLIP), and each
//! sheet's MsoDrawing records hold its shapes; a shape whose properties name a BLIP (`pib`,
//! 0x0104) and that has a client anchor (cells and fractions of them) is a picture.

/// Pictures read from one workbook at most.
const MAX_BLIPS: usize = 4096;
/// Nesting of drawing containers followed at most.
const MAX_DEPTH: usize = 16;

/// One stored picture: its bytes and MIME type, or `None` for a format GridCraft doesn't show
/// (metafiles, DIB, TIFF), so that `pib` indexes still line up.
pub(super) type Blip = Option<(Vec<u8>, &'static str)>;

/// An Office Drawing record header: version/instance, type, length.
fn header(b: &[u8], at: usize) -> Option<(u16, u16, usize)> {
    let h = b.get(at..at.checked_add(8)?)?;
    let vi = u16::from_le_bytes([*h.first()?, *h.get(1)?]);
    let ty = u16::from_le_bytes([*h.get(2)?, *h.get(3)?]);
    let len = u32::from_le_bytes([*h.get(4)?, *h.get(5)?, *h.get(6)?, *h.get(7)?]);
    Some((vi, ty, usize::try_from(len).ok()?))
}

/// The records directly inside `b` (a container's body): (version/instance, type, body).
fn children(b: &[u8]) -> Vec<(u16, u16, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some((vi, ty, len)) = header(b, at) {
        let start = at + 8;
        // A record claiming more than is there keeps what is there: the last one in a buffer
        // is often cut where its sheet's drawing records end.
        let end = start.saturating_add(len).min(b.len());
        out.push((vi, ty, b.get(start..end).unwrap_or(&[])));
        at = end;
    }
    out
}

/// The pictures of the workbook's MsoDrawingGroup, in BStore order (`pib` 1 is the first).
pub(super) fn blip_store(dgg: &[u8]) -> Vec<Blip> {
    let mut out = Vec::new();
    for (_, ty, body) in children(dgg) {
        if ty != 0xF000 {
            continue;
        }
        for (_, ty, store) in children(body) {
            if ty != 0xF001 {
                continue;
            }
            for (_, ty, fbse) in children(store) {
                if ty == 0xF007 && out.len() < MAX_BLIPS {
                    out.push(fbse_blip(fbse));
                }
            }
        }
    }
    out
}

/// The picture of one FBSE: 36 bytes of description, its name, then the BLIP record.
fn fbse_blip(fbse: &[u8]) -> Blip {
    let name_len = usize::from(*fbse.get(33)?);
    let rest = fbse.get(36usize.checked_add(name_len)?..)?;
    let (vi, ty, len) = header(rest, 0)?;
    let data = rest.get(8..8usize.checked_add(len)?.min(rest.len()))?;
    let instance = vi >> 4;
    // PNG and JPEG keep one or two 16-byte identifiers and a tag byte before the bytes.
    let (uids, mime) = match (ty, instance) {
        (0xF01E, 0x6E0) => (1, "image/png"),
        (0xF01E, 0x6E1) => (2, "image/png"),
        (0xF01D | 0xF02A, 0x46A | 0x6E2) => (1, "image/jpeg"),
        (0xF01D | 0xF02A, 0x46B | 0x6E3) => (2, "image/jpeg"),
        _ => return None,
    };
    let bytes = data.get(16 * uids + 1..)?;
    (!bytes.is_empty()).then(|| (bytes.to_vec(), mime))
}

/// A picture shape: its BLIP (1-based) and client anchor: left column, its 1/1024ths, top row,
/// its 1/256ths, then the same for the bottom-right corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PictureShape {
    pub pib: u32,
    pub anchor: [u16; 8],
}

/// The picture shapes in a sheet's drawing (its MsoDrawing records joined).
pub(super) fn picture_shapes(dg: &[u8]) -> Vec<PictureShape> {
    let mut out = Vec::new();
    walk(dg, 0, &mut out);
    out
}

fn walk(b: &[u8], depth: usize, out: &mut Vec<PictureShape>) {
    if depth > MAX_DEPTH {
        return;
    }
    for (vi, ty, body) in children(b) {
        if ty == 0xF004 {
            if let Some(p) = shape(body) {
                out.push(p);
            }
        } else if vi & 0x0F == 0x0F {
            walk(body, depth + 1, out);
        }
    }
}

/// A shape container's BLIP and anchor, when it has both.
fn shape(sp: &[u8]) -> Option<PictureShape> {
    let (mut pib, mut anchor) = (None, None);
    for (vi, ty, body) in children(sp) {
        match ty {
            // Properties: `instance` of them, six bytes each (id with flags, value).
            0xF00B => {
                for p in body.chunks(6).take(usize::from(vi >> 4)) {
                    if let [a, b, c, d, e, f] = *p
                        && u16::from_le_bytes([a, b]) & 0x3FFF == 0x0104
                    {
                        pib = Some(u32::from_le_bytes([c, d, e, f]));
                    }
                }
            }
            // Client anchor: flags, then eight 16-bit values.
            0xF010 => {
                let mut v = [0u16; 8];
                for (i, slot) in v.iter_mut().enumerate() {
                    let at = 2 + i * 2;
                    *slot = u16::from_le_bytes([*body.get(at)?, *body.get(at + 1)?]);
                }
                anchor = Some(v);
            }
            _ => {}
        }
    }
    Some(PictureShape { pib: pib.filter(|p| *p > 0)?, anchor: anchor? })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(vi: u16, ty: u16, body: &[u8]) -> Vec<u8> {
        [&vi.to_le_bytes()[..], &ty.to_le_bytes(), &(body.len() as u32).to_le_bytes(), body].concat()
    }

    #[test]
    fn pictures_and_their_anchors_are_found_and_hostile_drawings_are_harmless() {
        let png = [0x89, b'P', b'N', b'G'];
        let blip = rec(0x6E00, 0xF01E, &[&[0u8; 17][..], &png].concat());
        let fbse = rec(0x0062, 0xF007, &[&[0u8; 36][..], &blip].concat());
        let wmf = rec(0x0062, 0xF007, &[&[0u8; 36][..], &rec(0x2160, 0xF01B, &[0; 60])].concat());
        let dgg = rec(0x000F, 0xF000, &rec(0x002F, 0xF001, &[fbse, wmf].concat()));
        let store = blip_store(&dgg);
        assert_eq!(store.len(), 2);
        assert_eq!(store[0], Some((png.to_vec(), "image/png")));
        assert_eq!(store[1], None, "a metafile keeps its slot");
        // A shape with pib 1 and an anchor from B2 (half way across) to D5.
        let opt = rec(0x0013, 0xF00B, &[&0x4104u16.to_le_bytes()[..], &1u32.to_le_bytes()].concat());
        let anchor_body: Vec<u8> = [0u16, 1, 512, 1, 0, 3, 0, 4, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
        let sp = rec(0x000F, 0xF004, &[opt, rec(0x0000, 0xF010, &anchor_body)].concat());
        let dg = rec(0x000F, 0xF002, &rec(0x000F, 0xF003, &sp));
        assert_eq!(picture_shapes(&dg), vec![PictureShape { pib: 1, anchor: [1, 512, 1, 0, 3, 0, 4, 0] }]);
        // Truncated or nonsense drawings give nothing, never a panic.
        for bad in [&dg[..dg.len() - 5], &[0xFF; 40][..], &[][..], &dgg[..20]] {
            let _ = picture_shapes(bad);
            let _ = blip_store(bad);
        }
    }
}
