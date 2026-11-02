//! The font source of metafile text (EMF and WMF pictures, as old `.xls` files carry logos and
//! signatures in): the installed faces of GridCraft's font catalogue, each file loaded once.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use gridcraft_metafile::FaceData;

/// Font files kept loaded for metafile text at most.
const MAX_FONT_FILES: usize = 16;

fn file_cache() -> &'static Mutex<HashMap<PathBuf, Arc<Vec<u8>>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Vec<u8>>>>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

/// The installed face of `family` closest to bold/italic, or `None` when it isn't installed.
fn installed_face(family: &str, bold: bool, italic: bool) -> Option<FaceData> {
    let faces = crate::fonts::faces_of(family);
    let face = crate::fonts::pick_face(&faces, bold, italic)?;
    let mut cache = file_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let bytes = match cache.get(&face.path) {
        Some(b) => b.clone(),
        None => {
            let b = Arc::new(std::fs::read(&face.path).ok()?);
            if cache.len() >= MAX_FONT_FILES {
                cache.clear();
            }
            cache.insert(face.path.clone(), b.clone());
            b
        }
    };
    Some(FaceData { bytes, index: face.index, weight: face.weight, italic: face.italic })
}

/// Let metafile pictures (on screen and in print) draw their text with the installed fonts.
pub(crate) fn register() {
    gridcraft_metafile::set_font_source(installed_face);
}
