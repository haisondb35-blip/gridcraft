use super::{Backstage, Key, Language, Modifiers, RichText, Sense, SheetApp, Tokens, Ui, dim, i18n, json, section, theme, vec2};

/// File › Options panes, in order (Excel's list, with the ones GridCraft has settings for).
pub(crate) const OPTION_PANES: [(&str, &str); 7] = [
    ("general", "Options|General"),
    ("formulas", "Formulas"),
    ("proofing", "Proofing"),
    ("save", "Options|Save"),
    ("language", "Language"),
    ("advanced", "Advanced"),
    ("agents", "Agents"),
];

/// File › Options: the panes on the left, the chosen pane's settings on the right, as in Excel
/// and WordCraft. Every setting goes through a command (`view.*`, `options.*`, `formulas.*`).
pub(super) fn options(app: &mut SheetApp, ui: &mut Ui, bs: &mut Backstage) {
    let t = Tokens::get(ui.ctx());
    let lang = app.ui.language;
    let current = OPTION_PANES.iter().find(|p| p.0 == bs.options_pane).map_or("general", |p| p.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(180.0);
            for (id, label) in OPTION_PANES {
                let text = lang.tr(label);
                let active = current == id;
                let (r, resp) = ui.allocate_exact_size(vec2(180.0, 32.0), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, text));
                if active {
                    ui.painter().rect_filled(r, 4.0, t.hover);
                    ui.painter().rect_filled(egui::Rect::from_min_size(r.min, vec2(3.0, r.height())), 1.5, t.accent);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 4.0, t.hover);
                }
                let font = if active { theme::ui_bold(14.0) } else { theme::ui_font(14.0) };
                ui.painter().text(egui::pos2(r.min.x + 14.0, r.center().y), egui::Align2::LEFT_CENTER, text, font, t.text);
                if resp.clicked() {
                    bs.options_pane = id;
                }
            }
        });
        ui.add_space(12.0);
        let (line, _) = ui.allocate_exact_size(vec2(1.0, 460.0), Sense::hover());
        ui.painter().rect_filled(line, 0.0, t.separator);
        ui.add_space(22.0);
        ui.vertical(|ui| {
            ui.set_max_width(680.0);
            match current {
                "formulas" => options_formulas(app, ui),
                "proofing" => options_proofing(app, ui),
                "save" => options_save(app, ui),
                "language" => options_language(app, ui),
                "advanced" => options_advanced(app, ui),
                "agents" => {
                    section(ui, lang, "Agents");
                    ui.label(lang.tr("Every command is available to scripts and AI agents: run `gridcraft-cli mcp` for an MCP server, or start the app with `--control <port>` for the JSON control channel."));
                }
                _ => options_general(app, ui, bs),
            }
        });
    });
}

fn hint(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(text).font(theme::ui_font(11.5)).color(t.text_dim));
}

fn options_general(app: &mut SheetApp, ui: &mut Ui, bs: &mut Backstage) {
    let lang = app.ui.language;
    section(ui, lang, "Interface");
    egui::Grid::new("options_interface").num_columns(2).spacing(vec2(18.0, 10.0)).show(ui, |ui| {
        let label = ui.label(lang.tr("Display theme"));
        let mode = app.ui.theme_mode();
        let current = match mode {
            "system" => "System",
            "dark" => "Dark",
            _ => "Light",
        };
        craft_ui::ComboBox::from_id_salt("backstage_theme")
            .width(200.0)
            .selected_text(lang.tr(current))
            .show_ui(ui, |ui| {
                for (value, label) in [("system", "System"), ("light", "Light"), ("dark", "Dark")] {
                    if ui.selectable_label(mode == value, lang.tr(label)).clicked() {
                        app.run_or_alert("view.theme", json!({"mode": value}));
                    }
                }
            })
            .response
            .labelled_by(label.id);
        ui.end_row();
        // Interface size (#230), as in the View tab: the ribbon, its buttons, menus and dialogs
        // grow together, on top of the desktop's scale.
        let label = ui.label(lang.tr("Interface size"));
        let size = app.ui.interface_scale;
        let text = |v: u16| if v == 100 { lang.tr("Auto").to_string() } else { format!("{v}%") };
        craft_ui::ComboBox::from_id_salt("backstage_interface_size")
            .width(200.0)
            .selected_text(text(size))
            .show_ui(ui, |ui| {
                for v in [100, 75, 90, 110, 125, 150, 175, 200] {
                    if ui.selectable_label(v == size, text(v)).clicked() {
                        app.run_or_alert("view.interfaceScale", json!({"percent": v}));
                    }
                }
            })
            .response
            .labelled_by(label.id);
        ui.end_row();
    });
    hint(
        ui,
        lang.tr(
            "Makes the ribbon, its buttons, menus and dialogs larger or smaller, on top of your system's display scaling. Sheet zoom is separate.",
        ),
    );
    ui.add_space(6.0);
    let mut formula_bar = app.session.prefs.formula_bar;
    if ui.checkbox(&mut formula_bar, lang.tr("Show formula bar")).changed() {
        app.run_or_alert("view.formulaBar", json!({"on": formula_bar}));
    }
    ui.add_space(16.0);
    section(ui, lang, "When creating new workbooks");
    let prefs = app.session.prefs.clone();
    egui::Grid::new("options_new_workbooks").num_columns(2).spacing(vec2(18.0, 10.0)).show(ui, |ui| {
        let label = ui.label(lang.tr("Use this as the default font"));
        let fonts = ["Calibri", "Aptos", "Arial", "Times New Roman", "Cambria", "Tahoma", "Segoe UI", "Verdana"];
        craft_ui::ComboBox::from_id_salt("options_default_font")
            .width(200.0)
            .selected_text(&prefs.default_font)
            .show_ui(ui, |ui| {
                for f in fonts {
                    if ui.selectable_label(prefs.default_font == f, f).clicked() {
                        app.run_or_alert("options.set", json!({"defaultFont": f}));
                    }
                }
            })
            .response
            .labelled_by(label.id);
        ui.end_row();
        let label = ui.label(lang.tr("Font size"));
        let mut size = prefs.default_font_size;
        if ui.add(egui::DragValue::new(&mut size).range(1.0..=409.0).speed(0.5)).labelled_by(label.id).changed() {
            app.run_or_alert("options.set", json!({"defaultFontSize": size}));
        }
        ui.end_row();
        let label = ui.label(lang.tr("Include this many sheets"));
        let mut sheets = prefs.sheets_in_new_workbook;
        if ui.add(egui::DragValue::new(&mut sheets).range(1..=255)).labelled_by(label.id).changed() {
            app.run_or_alert("options.set", json!({"sheetsInNewWorkbook": sheets}));
        }
        ui.end_row();
    });
    ui.add_space(16.0);
    section(ui, lang, "Personalize");
    ui.horizontal(|ui| {
        let label = ui.label(lang.tr("User name"));
        let id = ui.make_persistent_id("options_user_name");
        if !ui.ctx().memory(|m| m.has_focus(id)) {
            bs.user_name.clone_from(&prefs.user_name);
        }
        let resp = ui.add(egui::TextEdit::singleline(&mut bs.user_name).id(id).desired_width(260.0)).labelled_by(label.id);
        if resp.lost_focus() && bs.user_name.trim() != prefs.user_name && !bs.user_name.trim().is_empty() {
            app.run_or_alert("options.set", json!({"userName": bs.user_name.trim()}));
        }
    });
    hint(ui, lang.tr("New comments and the workbook's Last Modified By are recorded under this name."));
}

fn options_formulas(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    section(ui, lang, "Calculation options");
    let Some(calc) = app.session.active().map(|d| d.wb.calc.clone()) else {
        dim(ui, lang.tr("Open a workbook to change how it calculates."));
        return;
    };
    ui.label(lang.tr("Workbook calculation"));
    calc_options(app, ui, lang, &calc);
    ui.add_space(6.0);
    dim(ui, lang.tr("Calculation settings are saved with the workbook."));
    ui.add_space(16.0);
    section(ui, lang, "Working with formulas");
    let mut autocomplete = app.ui.autocomplete;
    if ui.checkbox(&mut autocomplete, lang.tr("Enable AutoComplete for cell values")).changed() {
        app.run_or_alert("options.autoComplete", json!({"on": autocomplete}));
    }
}

fn options_proofing(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    section(ui, lang, "When correcting spelling");
    let n = app.session.prefs.user_dictionary.len();
    ui.label(format!("{}: {n}", lang.tr("Words added to the custom dictionary")));
    hint(ui, lang.tr("Review › Spelling checks the sheet; Add to Dictionary keeps a word for every workbook."));
}

fn options_save(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    section(ui, lang, "Save workbooks");
    let mut autosave = app.ui.autosave;
    if ui.checkbox(&mut autosave, lang.tr("Save .xlsx workbooks automatically after every change")).changed() {
        app.toggle_autosave();
    }
}

fn options_language(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    section(ui, lang, "Interface language");
    ui.horizontal(|ui| {
        let label = ui.label(lang.tr("Interface language"));
        craft_ui::ComboBox::from_id_salt("backstage_language")
            .width(200.0)
            .selected_text(lang.name())
            .show_ui(ui, |ui| {
                for l in i18n::Language::for_picker() {
                    if ui.selectable_label(l == lang, l.name()).clicked() {
                        app.run_or_alert("app.language.set", json!({"language": l.code()}));
                    }
                }
            })
            .response
            .labelled_by(label.id);
    });
}

fn options_advanced(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    section(ui, lang, "Editing options");
    let mut autocomplete = app.ui.autocomplete;
    if ui.checkbox(&mut autocomplete, lang.tr("Enable AutoComplete for cell values")).changed() {
        app.run_or_alert("options.autoComplete", json!({"on": autocomplete}));
    }
    ui.add_space(6.0);
    let dir = app.session.prefs.enter_direction.clone();
    ui.horizontal(|ui| {
        let caption = ui.label(lang.tr("After pressing Enter, move selection"));
        let label = |d: &str| match d {
            "right" => "Right",
            "up" => "Up",
            "left" => "Left",
            "none" => "Don't move",
            _ => "Down",
        };
        craft_ui::ComboBox::from_id_salt("options_enter_direction")
            .width(140.0)
            .selected_text(lang.tr(label(&dir)))
            .show_ui(ui, |ui| {
                for d in ["down", "right", "up", "left", "none"] {
                    if ui.selectable_label(dir == d, lang.tr(label(d))).clicked() {
                        app.run_or_alert("options.set", json!({"enterDirection": d}));
                    }
                }
            })
            .response
            .labelled_by(caption.id);
    });
    ui.add_space(16.0);
    section(ui, lang, "Display");
    let mut formula_bar = app.session.prefs.formula_bar;
    if ui.checkbox(&mut formula_bar, lang.tr("Show formula bar")).changed() {
        app.run_or_alert("view.formulaBar", json!({"on": formula_bar}));
    }
}

/// The calculation mode, and iterative calculation, of the active workbook.
fn calc_options(app: &mut SheetApp, ui: &mut Ui, lang: Language, calc: &gridcraft_engine::model::CalcSettings) {
    use gridcraft_engine::model::CalcMode;
    for (mode, id, label) in [
        (CalcMode::Automatic, "automatic", "Automatic"),
        (CalcMode::AutomaticExceptTables, "automaticExceptTables", "Automatic Except for Data Tables"),
        (CalcMode::Manual, "manual", "Manual"),
    ] {
        if ui.radio(calc.mode == mode, lang.tr(label)).clicked() && calc.mode != mode {
            app.run_or_alert("formulas.calculationOptions", json!({"mode": id}));
        }
    }
    ui.add_space(4.0);
    let mut iterative = calc.iterative;
    if ui.checkbox(&mut iterative, lang.tr("Enable iterative calculation")).changed() {
        app.run_or_alert("formulas.calculationOptions", json!({"iterative": iterative}));
    }
    // Maximum Iterations and Maximum Change apply only while iteration is on, as in Excel.
    iteration_options(app, ui, lang, calc, iterative);
    // TODO: add "Set precision as displayed" here once `formulas.calculationOptions` takes
    // `precisionAsDisplayed` (PR #1366); the model already stores it.
}

/// Maximum Iterations and Maximum Change, live only while iterative calculation is on.
fn iteration_options(app: &mut SheetApp, ui: &mut Ui, lang: Language, calc: &gridcraft_engine::model::CalcSettings, iterative: bool) {
    ui.add_enabled_ui(iterative, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            egui::Grid::new("backstage_options_iteration").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                let label = ui.label(lang.tr("Maximum Iterations:"));
                let id = ui.make_persistent_id("backstage_max_iterations");
                // Text left in the field when iteration is switched off (it loses focus) is dropped.
                let committed = crate::widgets::committed_text(ui, id, &calc.max_iterations.to_string(), Some(80.0));
                if let Some(response) = ui.ctx().read_response(id) {
                    response.labelled_by(label.id);
                }
                if let Some(text) = committed.filter(|_| iterative) {
                    match parse_max_iterations(&text) {
                        Some(n) if n != calc.max_iterations => {
                            app.run_or_alert("formulas.calculationOptions", json!({"maxIterations": n}));
                        }
                        Some(_) => {}
                        None => refuse(app, ui, "Maximum Iterations must be a whole number from 1 to 32767."),
                    }
                }
                ui.end_row();
                let label = ui.label(lang.tr("Maximum Change:"));
                let id = ui.make_persistent_id("backstage_max_change");
                let committed = crate::widgets::committed_text(ui, id, &calc.max_change.to_string(), Some(80.0));
                if let Some(response) = ui.ctx().read_response(id) {
                    response.labelled_by(label.id);
                }
                if let Some(text) = committed.filter(|_| iterative) {
                    match parse_max_change(&text, lang.formula_locale()) {
                        Some(d) if d != calc.max_change => {
                            app.run_or_alert("formulas.calculationOptions", json!({"maxChange": d}));
                        }
                        Some(_) => {}
                        None => refuse(app, ui, "Maximum Change must be a number of 0 or more."),
                    }
                }
                ui.end_row();
            });
        });
    });
}

/// Refuses a typed setting with a message, leaving the workbook as it was. The Enter that
/// committed the field is consumed so it doesn't also dismiss the message box in this frame.
fn refuse(app: &mut SheetApp, ui: &mut Ui, text: &str) {
    ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    app.message = Some(("GridCraft".into(), app.ui.language.tr(text).into()));
}

/// Maximum Iterations as typed: a whole number from 1 to 32767, or `None` (Excel refuses others).
fn parse_max_iterations(text: &str) -> Option<u32> {
    let n = text.trim().parse::<u32>().ok()?;
    (1..=gridcraft_engine::model::CalcSettings::MAX_ITERATIONS).contains(&n).then_some(n)
}

/// Maximum Change as typed: a finite number of 0 or more, or `None`. The interface's decimal
/// separator is accepted as well as a dot.
fn parse_max_change(text: &str, locale: gridcraft_engine::formula::FormulaLocale) -> Option<f64> {
    let d = text.trim().replace(locale.decimal_separator(), ".").parse::<f64>().ok()?;
    (d.is_finite() && d >= 0.0).then_some(d)
}

/// The platform the app was built for, e.g. "macOS (aarch64)"; the web build reports "Web".
pub(super) fn platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        "freebsd" => "FreeBSD",
        "openbsd" => "OpenBSD",
        "netbsd" => "NetBSD",
        _ if cfg!(target_arch = "wasm32") => return "Web".into(),
        other => other,
    };
    format!("{os} ({})", std::env::consts::ARCH)
}
