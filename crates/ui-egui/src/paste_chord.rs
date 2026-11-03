//! Keyboard pastes of something other than text. egui swallows a paste chord's key press and sends
//! a Paste event only when the clipboard holds text, so a paste key released without its press
//! (or a Paste event) reported was a paste of a picture or of files copied in a file manager.
//! (After VectorCraft's `shortcuts/paste.rs`.)

use egui::Key;

/// Keys that paste with Ctrl/Cmd, or alone.
const PASTE_KEYS: [Key; 2] = [Key::V, Key::Paste];

#[derive(Clone, Debug, Default)]
pub(crate) struct PasteChord {
    /// Paste keys whose press was reported (typed, not pasting).
    pressed: Vec<Key>,
    /// A Paste event came since the last paste key was released.
    pasted: bool,
}

impl PasteChord {
    /// Follow one frame's `events` → whether a paste chord egui sent no Paste event for was
    /// released.
    pub(crate) fn textless_paste(&mut self, events: &[egui::Event]) -> bool {
        let mut fire = false;
        for e in events {
            match e {
                egui::Event::Paste(_) => self.pasted = true,
                // A release that comes while the window is away is never seen.
                egui::Event::WindowFocused(false) => *self = Self::default(),
                egui::Event::Key { key, pressed: true, .. } if PASTE_KEYS.contains(key) && !self.pressed.contains(key) => self.pressed.push(*key),
                egui::Event::Key { key, pressed: false, .. } if PASTE_KEYS.contains(key) => match self.pressed.iter().position(|k| k == key) {
                    Some(i) => {
                        self.pressed.swap_remove(i);
                    }
                    None => fire |= !std::mem::take(&mut self.pasted),
                },
                _ => {}
            }
        }
        fire
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key, pressed: bool) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::COMMAND }
    }

    #[test]
    fn a_paste_with_no_text_fires_on_release_once() {
        let mut chord = PasteChord::default();
        // Ctrl+V over a copied picture: egui swallows the press and sends no Paste.
        assert!(chord.textless_paste(&[key(Key::V, false)]));
        // Over text: a Paste event, then the release.
        assert!(!chord.textless_paste(&[egui::Event::Paste("a".into())]));
        assert!(!chord.textless_paste(&[key(Key::V, false)]));
        // Typing V: its press is reported.
        assert!(!chord.textless_paste(&[key(Key::V, true)]));
        assert!(!chord.textless_paste(&[key(Key::V, false)]));
    }
}
