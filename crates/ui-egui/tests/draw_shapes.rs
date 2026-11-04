//! Insert › Shapes draws the shape on the sheet, as in Excel: the picked shape waits for the
//! pointer, a drag gives its box, a click a default-size one where it lands, Escape cancels.
use egui::{Event, Modifiers, PointerButton, Pos2, pos2};
use gridcraft_engine::Session;
use gridcraft_ui_egui::{Services, SheetApp};

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, Services::default()),
    );
    h.run_steps(4);
    h
}

fn cells(h: &Harness) -> egui::Rect {
    h.state().grid.cells_rect.expect("grid drawn")
}

fn press(h: &mut Harness, p: Pos2, pressed: bool) {
    h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

fn shapes(h: &Harness) -> Vec<gridcraft_engine::model::Shape> {
    h.state().session.active().and_then(|d| d.wb.active()).map(|s| s.shapes.clone()).unwrap_or_default()
}

#[test]
fn dragging_draws_the_picked_shape_where_the_pointer_went() {
    let mut h = harness();
    h.state_mut().shape_tool = Some("ellipse".into());
    let c = cells(&h);
    let (a, b) = (pos2(c.left() + 250.0, c.top() + 120.0), pos2(c.left() + 450.0, c.top() + 220.0));
    h.input_mut().events.push(Event::PointerMoved(a));
    h.step();
    press(&mut h, a, true);
    for t in 1..=6 {
        h.input_mut().events.push(Event::PointerMoved(a + (b - a) * (t as f32 / 6.0)));
        h.step();
    }
    press(&mut h, b, false);
    h.run_steps(2);
    let s = shapes(&h);
    assert_eq!(s.len(), 1, "one shape drawn");
    assert_eq!(s[0].kind, gridcraft_engine::model::ShapeKind::Ellipse);
    // The box in sheet units is the dragged box on screen over the grid's zoom.
    let z = h.state().grid.geo.as_ref().map_or(1.0, |g| g.z);
    assert!((s[0].anchor.width * z - 200.0).abs() < 3.0 && (s[0].anchor.height * z - 100.0).abs() < 3.0, "{:?} z {z}", s[0].anchor);
    // Not at the active cell A1: where the drag began (column 4 or so, row 6 or so).
    assert!(s[0].anchor.cell.col >= 3 && s[0].anchor.cell.row >= 5, "{:?}", s[0].anchor);
    assert!(h.state().shape_tool.is_none(), "the tool is put down");
    assert_eq!(h.state().selected_object.map(|o| o.id), Some(s[0].id), "the new shape is selected");
}

#[test]
fn a_click_drops_a_default_size_shape_and_escape_cancels() {
    let mut h = harness();
    h.state_mut().shape_tool = Some("rectangle".into());
    let c = cells(&h);
    let p = pos2(c.left() + 300.0, c.top() + 300.0);
    h.input_mut().events.push(Event::PointerMoved(p));
    h.step();
    press(&mut h, p, true);
    press(&mut h, p, false);
    h.run_steps(2);
    let s = shapes(&h);
    assert_eq!(s.len(), 1);
    assert!((s[0].anchor.width - 144.0).abs() < 0.01 && (s[0].anchor.height - 96.0).abs() < 0.01, "{:?}", s[0].anchor);
    // Escape before drawing: nothing is added.
    h.state_mut().shape_tool = Some("triangle".into());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().shape_tool.is_none());
    press(&mut h, p, true);
    press(&mut h, p, false);
    assert_eq!(shapes(&h).len(), 1);
}
