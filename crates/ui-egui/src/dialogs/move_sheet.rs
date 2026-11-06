//! Home › Format › Move or Copy Sheet…, as Excel lays it out: To book (this workbook, another
//! open one, or a new book), Before sheet (that book's sheets, then "(move to end)") and Create a
//! copy. OK runs `sheet.move` with `book` when the sheet goes to another workbook.

use super::show::ok_cancel;
use super::{Dialog, Json, SheetApp, json};

/// The choice in To book: an open workbook (its uid) or a new one.
fn book(d: &Dialog, current: u64) -> Option<u64> {
    match d.values.get("book") {
        Some(Json::String(s)) if s == "new" => None,
        Some(v) => v.as_u64().or(Some(current)),
        None => Some(current),
    }
}

pub(super) fn move_sheet(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    let lang = app.ui.language;
    let Some(current) = app.session.active().map(|doc| doc.uid) else {
        *open = false;
        return;
    };
    let sheet = d.values.get("sheet").and_then(Json::as_u64).map(|v| v as usize).or_else(|| app.session.active().map(|doc| doc.wb.active_sheet()));
    ui.label(lang.tr("Move selected sheets"));
    ui.add_space(4.0);
    // To book.
    let chosen = book(d, current);
    let books: Vec<(u64, String)> = app.session.documents().iter().map(|doc| (doc.uid, doc.display_title())).collect();
    let shown = match chosen {
        Some(uid) => books.iter().find(|(u, _)| *u == uid).map_or_else(String::new, |(_, t)| t.clone()),
        None => lang.tr("(new book)").to_string(),
    };
    let label = ui.label(lang.tr("To book:"));
    let mut pick = chosen;
    craft_ui::ComboBox::from_id_salt("move_sheet_book")
        .width(ui.available_width())
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for (uid, title) in &books {
                ui.selectable_value(&mut pick, Some(*uid), title);
            }
            ui.selectable_value(&mut pick, None, lang.tr("(new book)"));
        })
        .response
        .labelled_by(label.id);
    if pick != chosen {
        d.values.insert("book".into(), pick.map_or(json!("new"), |u| json!(u)));
        d.values.insert("before".into(), Json::Null);
    }
    // Before sheet: the chosen book's sheets, then the end.
    ui.add_space(6.0);
    let label = ui.label(lang.tr("Before sheet:"));
    let sheets: Vec<String> = match pick {
        Some(uid) => app
            .session
            .documents()
            .iter()
            .find(|doc| doc.uid == uid)
            .map(|doc| doc.wb.sheets.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let before = d.values.get("before").and_then(Json::as_u64).map(|v| v as usize);
    let mut choice = before;
    egui::ScrollArea::vertical().max_height(150.0).auto_shrink([false, true]).show(ui, |ui| {
        for (k, s) in sheets.iter().enumerate() {
            ui.selectable_value(&mut choice, Some(k), s).labelled_by(label.id);
        }
        ui.selectable_value(&mut choice, None, lang.tr("(move to end)"));
    });
    if choice != before {
        d.values.insert("before".into(), choice.map_or(Json::Null, |k| json!(k)));
    }
    ui.add_space(6.0);
    let mut copy = d.values.get("copy").and_then(Json::as_bool).unwrap_or(false);
    if ui.checkbox(&mut copy, lang.tr("Create a copy")).changed() {
        d.values.insert("copy".into(), json!(copy));
    }
    ok_cancel(ui, confirm, open);
    if *confirm {
        let mut p = json!({"copy": copy});
        if let Some(i) = sheet {
            p["sheet"] = json!(i);
        }
        if pick == Some(current) {
            // In this workbook `to` is where the sheet ends up: before a later sheet that is one
            // less once a move has taken it out.
            let n = sheets.len();
            let at = choice.unwrap_or(n);
            let to = match (copy, sheet) {
                (false, Some(i)) if i < at => at - 1,
                _ => at,
            };
            p["to"] = json!(to);
        } else {
            p["book"] = pick.map_or(json!("new"), |u| json!(u));
            if let Some(k) = choice {
                p["to"] = json!(k);
            }
        }
        app.run_or_alert("sheet.move", p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_book_choice_reads_back() {
        let mut d = Dialog::form("moveSheet", "Move or Copy", "sheet.move", vec![], json!({}));
        assert_eq!(book(&d, 7), Some(7));
        d.values.insert("book".into(), json!("new"));
        assert_eq!(book(&d, 7), None);
        d.values.insert("book".into(), json!(3));
        assert_eq!(book(&d, 7), Some(3));
    }
}
