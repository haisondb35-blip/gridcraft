//! The web build: no system clipboard here (the browser's is reached through egui's own
//! copy and paste events), so copies stay inside the app and nothing is read.

use std::path::PathBuf;

use crate::{Copy, Holds, Html};

pub fn clear() -> Result<(), String> {
    Ok(())
}

pub fn write(_copy: &Copy) -> Result<(), String> {
    Err("no system clipboard in the web build".into())
}

pub fn text() -> Option<String> {
    None
}

pub fn html() -> Option<Html> {
    None
}

pub fn rtf() -> Option<Vec<u8>> {
    None
}

pub fn svg() -> Option<Vec<u8>> {
    None
}

pub fn bitmap() -> Option<Vec<u8>> {
    None
}

pub fn files() -> Vec<PathBuf> {
    Vec::new()
}

pub fn own(_name: &str) -> Option<Vec<u8>> {
    None
}

pub fn holds_own(_name: &str) -> bool {
    false
}

pub fn holds() -> Holds {
    Holds::default()
}

pub fn sequence() -> Option<u32> {
    None
}

pub fn format_names() -> Vec<String> {
    Vec::new()
}
