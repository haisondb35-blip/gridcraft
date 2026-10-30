use super::geo::Geo;
use super::editor::forward_to_editor;
use super::interact::delete_selected_object;
use super::menus::is_checkbox;
use super::{CellRef, Drag, Sheet, SheetApp, json};

/// Input methods compose over the active cell (see [`keyboard`]).
fn allow_ime(app: &SheetApp, ctx: &egui::Context, resp: &egui::Response, geo: &Geo, sh: &Sheet) {
    if let Some(active) = app.session.active().map(|d| d.selection.active) {
        let r = geo.mirror_rect(geo.cell_rect(sh, active));
        ctx.output_mut(|o| o.ime = Some(egui::output::IMEOutput::new(resp.id, 0, Default::default(), r, r)));
    }
}

/// A typed character, or an input method's first composition or commit: it starts an edit of the
/// active cell.
fn starts_edit(ev: &egui::Event) -> bool {
    match ev {
        egui::Event::Text(text) => !text.is_empty() && text.chars().all(|c| !c.is_control()),
        egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => !text.is_empty(),
        egui::Event::Ime(egui::ImeEvent::Commit(text)) => text.chars().any(|c| !c.is_control()),
        _ => false,
    }
}

/// Start editing the active cell with `ev` (see [`starts_edit`]); the editor gets the rest of
/// this frame's events, so the input method carries on in it.
fn begin_typing(app: &mut SheetApp, ctx: &egui::Context, ev: egui::Event, rest: impl Iterator<Item = egui::Event>) {
    match ev {
        egui::Event::Text(text) | egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
            app.begin_edit(Some(text), false);
            if let Some(ed) = app.editor.as_mut() {
                ed.caret = ed.text.chars().count();
            }
            forward_to_editor(app, ctx, rest);
        }
        // A composition in progress: the editor shows it and takes the commit.
        ev => {
            app.begin_edit(Some(String::new()), false);
            forward_to_editor(app, ctx, std::iter::once(ev).chain(rest));
        }
    }
}

/// Grid keyboard handling when no text field has focus.
pub(super) fn keyboard(app: &mut SheetApp, ctx: &egui::Context, resp: &egui::Response, geo: &Geo, sh: &Sheet) {
    // Focus left on an editor that no longer exists goes back to the grid.
    let stale = [egui::Id::unique("gridcraft.cell_editor"), egui::Id::unique("gridcraft.formula_bar"), crate::text_box::editor_id()];
    let editing = app.editor.is_some() || app.text_box_editor.is_some();
    if !editing && ctx.memory(|m| m.focused()).is_some_and(|f| stale.contains(&f)) {
        resp.request_focus();
    }
    let other_focus = ctx.memory(|m| m.focused()).is_some_and(|f| f != resp.id && (editing || !stale.contains(&f)));
    // An open filter menu owns the keyboard even when a click inside it left focus on nothing (#1516).
    // So do the fill menu (its Esc closes it, not the copy) and the right-drag that opens it (an
    // arrow key would move the selection the menu fills from), and a macro's question.
    if editing
        || app.dialog.is_some()
        || other_focus
        || app.message.is_some()
        || app.grid.filter_menu.is_some()
        || app.grid.fill_menu.is_some()
        || matches!(app.grid.drag, Drag::Fill { menu: true, .. })
        || app.macros.question.is_some()
    {
        return;
    }
    if ctx.memory(|m| m.focused()).is_none() {
        resp.request_focus();
    }
    // Input methods (the Windows Vietnamese Telex keyboard, Chinese, Japanese, Korean) compose only
    // while the window allows IME, which egui does only for a focused text field. Allow it over the
    // active cell, so the first word typed into a cell composes as well: without it the first key
    // arrived raw and the input method took over mid-word (Telex `awn` became `aưn`, not `ăn`).
    allow_ime(app, ctx, resp, geo, sh);
    // The grid owns arrows, Tab and Escape (egui would otherwise move focus between widgets).
    ctx.memory_mut(|m| {
        m.set_focus_lock_filter(resp.id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
    });
    let events = ctx.input(|i| i.events.clone());
    // A shortcut key that also types a character (Shift+Space sends Text(" ")) must not start an edit.
    let shortcut_pressed = events.iter().any(|e| {
        matches!(e, egui::Event::Key { pressed: true, modifiers: m, key, .. }
            if !m.alt && (m.ctrl || m.command || (m.shift && *key == egui::Key::Space)))
    });
    let page = ((geo.cells.height() / geo.z) / sh.default_row_height).floor().max(1.0) as i64;
    // Alt shortcuts type their character too on Windows and Linux (Alt+= sends Text("=")): once
    // a shortcut ran, that text must not start an edit over what it just did.
    let mut shortcut_ran = false;
    let mut events = events.into_iter();
    while let Some(ev) = events.next() {
        match ev {
            egui::Event::Copy => {
                app.copy_to_clipboard(ctx, "edit.copy");
            }
            egui::Event::Cut => {
                app.copy_to_clipboard(ctx, "edit.cut");
            }
            egui::Event::Paste(text) => app.paste_external(text),
            // GridCraft has no image-clipboard pipeline and no cell-picture insert-from-clipboard
            // command. Choosing to ignore `PasteImage` is explicit: inserting the bytes as a
            // floating picture without a chosen anchor/size would be a new feature, not parity.
            // A text representation, when the clipboard has one, still arrives as `Paste` above.
            egui::Event::PasteImage(_) => {}
            // PRIMARY middle-click paste belongs to egui's TextEdit; the grid has no caret to
            // insert text at, so a middle click on a cell is deliberately not a paste.
            egui::Event::MiddleClickPaste { .. } => {}
            egui::Event::Text(_) if shortcut_pressed || shortcut_ran => {}
            ev if !shortcut_pressed && starts_edit(&ev) => {
                begin_typing(app, ctx, ev, events);
                return;
            }
            egui::Event::Key { key, physical_key, pressed: true, modifiers: m, .. } => {
                use egui::Key;
                if crate::keymap::dispatch(app, ctx, key, physical_key, m, geo, sh) {
                    shortcut_ran = true;
                    continue;
                }
                let cmd = m.command;
                let shift = m.shift;
                let mv = |app: &mut SheetApp, dr: i64, dc: i64| {
                    let from = app.session.active().map(|d| if shift { d.selection.moving_end() } else { d.selection.active });
                    let _ = app.session.run("selection.move", json!({"dr": dr, "dc": dc, "extend": shift, "jump": cmd}));
                    app.grid.ensure_visible = true;
                    app.grid.reveal_moving_end = shift;
                    app.grid.reveal_from = from;
                };
                match key {
                    Key::ArrowUp => mv(app, -1, 0),
                    Key::ArrowDown => mv(app, 1, 0),
                    // Arrows follow the screen: on a right-to-left sheet Left goes to the next column.
                    Key::ArrowLeft => mv(app, 0, if sh.right_to_left { 1 } else { -1 }),
                    Key::ArrowRight => mv(app, 0, if sh.right_to_left { -1 } else { 1 }),
                    Key::PageDown if !cmd => mv(app, page, 0),
                    Key::PageUp if !cmd => mv(app, -page, 0),
                    Key::PageDown => app.run_or_alert("sheet.next", json!({})),
                    Key::PageUp => app.run_or_alert("sheet.previous", json!({})),
                    Key::Home if cmd => {
                        let r = sh.freeze.map(|f| f.0).unwrap_or(0);
                        let c = sh.freeze.map(|f| f.1).unwrap_or(0);
                        let _ = app.session.run("selection.set", json!({"cell": CellRef::new(r, c).a1()}));
                        app.grid.ensure_visible = true;
                    }
                    Key::End if cmd => {
                        let end = sh.used_range().map(|u| u.end).unwrap_or_default();
                        let _ = app.session.run("selection.set", json!({"cell": end.a1()}));
                        app.grid.ensure_visible = true;
                    }
                    Key::Home => {
                        let a = app.session.active().map(|d| d.selection.active).unwrap_or_default();
                        let _ = app.session.run("selection.set", json!({"cell": CellRef::new(a.row, 0).a1()}));
                        app.grid.ensure_visible = true;
                    }
                    // Excel: Enter while a copied range is marked pastes it and ends copy mode.
                    Key::Enter if !m.alt && !m.ctrl && !shift && !cmd && app.session.clipboard.is_some() => {
                        app.run_or_alert("edit.paste", json!({}));
                        app.session.clipboard = None;
                        app.grid.ensure_visible = true;
                    }
                    Key::Enter => {
                        if m.alt || m.ctrl {
                            app.begin_edit(None, false);
                        } else {
                            let (dr, dc) = app.enter_step(shift);
                            if dr != 0 || dc != 0 {
                                app.move_after_enter(dr, dc);
                            }
                        }
                    }
                    Key::Tab => app.move_after_enter(0, if shift { -1 } else { 1 }),
                    Key::F2 => app.begin_edit(None, false),
                    Key::Delete if delete_selected_object(app, sh) => {}
                    Key::Delete => app.run_or_alert("edit.clearContents", json!({})),
                    Key::Backspace if delete_selected_object(app, sh) => {}
                    Key::Backspace => {
                        // Excel for Mac: Delete clears the cell and starts editing it.
                        app.run_or_alert("edit.clearContents", json!({}));
                        if app.session.active().is_some_and(|d| d.selection.is_single_cell()) {
                            app.begin_edit(Some(String::new()), false);
                        }
                    }
                    Key::Escape => {
                        app.session.clipboard = None;
                        app.session.format_painter = None;
                    }
                    Key::Space if !cmd && !shift && is_checkbox(sh, app) => app.run_or_alert("cell.toggleCheckbox", json!({})),
                    Key::F8 if m.alt => {
                        app.open_dialog("macros", json!({}));
                    }
                    Key::F9 => app.run_or_alert("formulas.calculateNow", json!({})),
                    Key::F4 if !cmd => {}
                    // Ctrl+Shift+letter (⌘⇧letter) runs the macro with that shortcut, unless GridCraft uses it.
                    _ if cmd && shift && !m.alt && crate::macro_ui::run_shortcut(app, key) => {}
                    _ if cmd && !crate::keymap::altgr_text(ctx, m, ctx.os() == egui::os::OperatingSystem::Mac) => {
                        crate::ribbon::shortcut(app, key, m)
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
