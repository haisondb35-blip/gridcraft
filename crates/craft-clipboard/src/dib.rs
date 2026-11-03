//! Windows device-independent bitmaps (`CF_DIB`, `CF_DIBV5`): a BITMAPINFO header, the colour masks
//! of a 40-byte header, the colour table, then the pixels. Screenshots and most programs copy
//! pictures this way; some read nothing else.

use std::io::Cursor;

/// Most pixels a side of a bitmap another program put on the clipboard (untrusted data).
pub const MAX_SIDE: u32 = 32_768;

/// A DIB as a BMP file, whose header says where the pixels start. `None` when the header is cut
/// short or out of range.
fn bmp_file(dib: &[u8]) -> Option<Vec<u8>> {
    const BI_BITFIELDS: u32 = 3;
    const BI_ALPHABITFIELDS: u32 = 6;
    let u16_at = |i: usize| Some(u16::from_le_bytes(dib.get(i..i + 2)?.try_into().ok()?));
    let u32_at = |i: usize| Some(u32::from_le_bytes(dib.get(i..i + 4)?.try_into().ok()?));
    let header = u32_at(0)?;
    // BITMAPCOREHEADER has 16-bit fields and 3-byte colours; the others 32-bit fields, 4-byte
    // colours and a count of them (0: all a palette of the depth has), the 40-byte one its masks
    // after it.
    let (bits, used, entry, masks) = match header {
        12 => (u16_at(10)?, 0, 3, 0),
        _ => {
            let masks = match (header, u32_at(16)?) {
                (40, BI_BITFIELDS) => 12,
                (40, BI_ALPHABITFIELDS) => 16,
                _ => 0,
            };
            (u16_at(14)?, u32_at(32)?, 4, masks)
        }
    };
    let colours = match used {
        0 if bits <= 8 => 1 << bits,
        n if n <= 256 => n,
        _ => return None,
    };
    let offset = [header, masks, colours * entry].into_iter().try_fold(14u32, u32::checked_add)?;
    let size = u32::try_from(dib.len()).ok()?.checked_add(14)?;
    if offset > size {
        return None;
    }
    let mut bmp = Vec::with_capacity(dib.len() + 14);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&size.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&offset.to_le_bytes());
    bmp.extend_from_slice(dib);
    Some(bmp)
}

/// A DIB as an image, at most [`MAX_SIDE`] pixels a side. It is opaque when its alpha is zero
/// throughout: screenshots and most programs leave the fourth byte of a 32-bit pixel at zero.
pub fn decode(dib: &[u8]) -> Option<image::RgbaImage> {
    let mut reader = image::ImageReader::with_format(Cursor::new(bmp_file(dib)?), image::ImageFormat::Bmp);
    let mut limits = image::Limits::default();
    (limits.max_image_width, limits.max_image_height) = (Some(MAX_SIDE), Some(MAX_SIDE));
    reader.limits(limits);
    let mut img = reader.decode().ok()?.into_rgba8();
    if img.pixels().all(|p| p.0[3] == 0) {
        img.pixels_mut().for_each(|p| p.0[3] = u8::MAX);
    }
    Some(img)
}

/// `img` as PNG bytes.
pub fn png(img: &image::RgbaImage) -> Option<Vec<u8>> {
    let mut out = vec![];
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
    Some(out)
}

/// PNG (or any format `image` reads here) bytes as an image, at most [`MAX_SIDE`] a side.
pub fn decode_png(bytes: &[u8]) -> Option<image::RgbaImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    (limits.max_image_width, limits.max_image_height) = (Some(MAX_SIDE), Some(MAX_SIDE));
    reader.limits(limits);
    Some(reader.decode().ok()?.into_rgba8())
}

/// A BITMAPINFOHEADER (40 bytes) or the start of a BITMAPV5HEADER.
fn info_header(size: u32, img: &image::RgbaImage, bits: u16, compression: u32, image_size: usize) -> Vec<u8> {
    let mut h = Vec::with_capacity(size as usize);
    h.extend(size.to_le_bytes());
    h.extend(i32::try_from(img.width()).unwrap_or(i32::MAX).to_le_bytes());
    // Positive: bottom row first, which every reader takes.
    h.extend(i32::try_from(img.height()).unwrap_or(i32::MAX).to_le_bytes());
    h.extend(1u16.to_le_bytes());
    h.extend(bits.to_le_bytes());
    h.extend(compression.to_le_bytes());
    h.extend(u32::try_from(image_size).unwrap_or(0).to_le_bytes());
    // 96 pixels an inch, no colour table.
    for v in [3780u32, 3780, 0, 0] {
        h.extend(v.to_le_bytes());
    }
    h
}

/// `img` over white as a 24-bit `CF_DIB`, for programs that ignore alpha.
pub fn opaque(img: &image::RgbaImage) -> Vec<u8> {
    let over = |c: u8, a: u8| ((u32::from(c) * u32::from(a) + 255 * (255 - u32::from(a)) + 127) / 255) as u8;
    let row = (img.width() as usize * 3).div_ceil(4) * 4;
    let mut pixels = Vec::with_capacity(row * img.height() as usize);
    for y in (0..img.height()).rev() {
        let start = pixels.len();
        for x in 0..img.width() {
            let [r, g, b, a] = img.get_pixel(x, y).0;
            pixels.extend([over(b, a), over(g, a), over(r, a)]);
        }
        pixels.resize(start + row, 0);
    }
    let mut dib = info_header(40, img, 24, 0, pixels.len());
    dib.extend(pixels);
    dib
}

/// `img` with its alpha as a 32-bit `CF_DIBV5` (BGRA masks, sRGB, straight alpha).
pub fn with_alpha(img: &image::RgbaImage) -> Vec<u8> {
    const BI_BITFIELDS: u32 = 3;
    const LCS_SRGB: u32 = 0x7352_4742;
    const LCS_GM_IMAGES: u32 = 4;
    let mut pixels = Vec::with_capacity(img.as_raw().len());
    for y in (0..img.height()).rev() {
        for x in 0..img.width() {
            let [r, g, b, a] = img.get_pixel(x, y).0;
            pixels.extend([b, g, r, a]);
        }
    }
    let mut dib = info_header(124, img, 32, BI_BITFIELDS, pixels.len());
    for v in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000, LCS_SRGB] {
        dib.extend(v.to_le_bytes());
    }
    // Endpoints and gamma (unused for sRGB), then the intent, profile offset and size, reserved.
    dib.resize(108, 0);
    for v in [LCS_GM_IMAGES, 0, 0, 0] {
        dib.extend(v.to_le_bytes());
    }
    debug_assert_eq!(dib.len(), 124);
    dib.extend(pixels);
    dib
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DIB of `w`×`h` pixels (top row first when `h` is negative): a `header`-byte header (40,
    /// or 124 for a BITMAPV5HEADER with BGRA masks), `extra` (the masks after a 40-byte header, a
    /// colour table), then `rows`, each padded to 4 bytes.
    fn dib(header: u32, w: i32, h: i32, bits: u16, compression: u32, extra: &[u8], rows: &[u8]) -> Vec<u8> {
        let mut d = vec![];
        d.extend(header.to_le_bytes());
        d.extend(w.to_le_bytes());
        d.extend(h.to_le_bytes());
        d.extend(1u16.to_le_bytes());
        d.extend(bits.to_le_bytes());
        d.extend(compression.to_le_bytes());
        d.extend((rows.len() as u32).to_le_bytes());
        d.extend([3780u32, 3780, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
        if header == 124 {
            d.extend([0x00FF_0000u32, 0xFF00, 0xFF, 0xFF00_0000, 0x7352_4742].iter().flat_map(|v| v.to_le_bytes()));
            d.resize(124, 0);
        }
        d.extend_from_slice(extra);
        d.extend_from_slice(rows);
        d
    }

    fn bgra(px: &[[u8; 4]]) -> Vec<u8> {
        px.iter().flat_map(|[r, g, b, a]| [*b, *g, *r, *a]).collect()
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn pixels(img: &image::RgbaImage) -> Vec<[u8; 4]> {
        img.pixels().map(|p| p.0).collect()
    }

    #[test]
    fn a_screenshot_dib_is_opaque_and_upright() {
        let rows = bgra(&[[0, 0, 255, 0], [0, 0, 255, 0], [255, 0, 0, 0], [255, 0, 0, 0]]);
        assert_eq!(pixels(&decode(&dib(40, 2, 2, 32, 0, &[], &rows)).unwrap()), [RED, RED, BLUE, BLUE]);
        let img = decode(&dib(124, 2, -2, 32, 3, &[], &bgra(&[[255, 0, 0, 0]; 4]))).unwrap();
        assert_eq!(pixels(&img), [RED; 4]);
    }

    #[test]
    fn masks_and_colour_tables_come_before_the_pixels() {
        let masks: Vec<u8> = [0x00FF_0000u32, 0xFF00, 0xFF].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(pixels(&decode(&dib(40, 1, 1, 32, 3, &masks, &bgra(&[[0, 0, 255, 0]]))).unwrap()), [BLUE]);
        let mut table = bgra(&[RED, BLUE]);
        table[3] = 0;
        let mut d = dib(40, 3, 2, 8, 0, &table, &[1, 1, 1, 0, 0, 1, 0, 0]);
        d[32..36].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(pixels(&decode(&d).unwrap()), [RED, BLUE, RED, BLUE, BLUE, BLUE]);
    }

    #[test]
    fn what_is_written_reads_back() {
        let mut img = image::RgbaImage::new(3, 2);
        img.put_pixel(0, 0, image::Rgba(RED));
        img.put_pixel(2, 1, image::Rgba([0, 0, 255, 128]));
        // Alpha kept in the V5 bitmap.
        assert_eq!(pixels(&decode(&with_alpha(&img)).unwrap()), pixels(&img));
        // Over white in the opaque one; rows padded (3 × 3 bytes → 12).
        let flat = decode(&opaque(&img)).unwrap();
        assert_eq!(flat.get_pixel(0, 0).0, RED);
        assert_eq!(flat.get_pixel(1, 0).0, [255, 255, 255, 255]);
        assert_eq!(flat.get_pixel(2, 1).0, [127, 127, 255, 255]);
        assert_eq!(opaque(&img).len(), 40 + 12 * 2);
    }

    #[test]
    fn untrusted_dibs_are_refused() {
        let one = bgra(&[RED]);
        assert!(decode(&dib(40, MAX_SIDE as i32 + 1, 1, 32, 0, &[], &one)).is_none());
        assert!(decode(&dib(40, 4096, 4096, 32, 0, &[], &one)).is_none());
        assert!(decode(&[40, 0, 0]).is_none());
        assert!(decode(&[]).is_none());
        let mut d = dib(40, 1, 1, 8, 0, &[], &[0; 4]);
        d[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(bmp_file(&d).is_none());
    }
}
