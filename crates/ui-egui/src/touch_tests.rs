//! Fingers on the slide view, the thumbnails and the notes, end to end: the window's input goes
//! through the app's raw-input step (`SlideApp::raw_input_hook`, which eframe runs and kittest
//! doesn't), then through egui and the app (`crate::finger`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use deckcraft_engine::Session;
use egui::{Event, Modifiers, PointerButton, Pos2, TouchDeviceId, TouchId, TouchPhase, Vec2, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::{Services, SlideApp};

type H = Harness<'static, SlideApp>;

const FRAME: f64 = 1.0 / 60.0;

/// The app on `session` in a phone-tall window, and the time to start at.
fn harness(session: Session) -> (H, f64) {
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SlideApp::new(session, Services::default()),
    );
    h.run_steps(3);
    let t = h.ctx.input(|i| i.time) + 1.0;
    (h, t)
}

/// One frame of window input at `t`, as eframe hands it to the app.
fn frame(h: &mut H, t: f64, events: Vec<Event>) {
    let ctx = h.ctx.clone();
    let mut raw = egui::RawInput { time: Some(t), events, ..Default::default() };
    h.state_mut().raw_input_hook(&ctx, &mut raw);
    let input = h.input_mut();
    input.time = Some(t);
    input.events.extend(raw.events);
    h.step();
}

fn press(p: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn touch(id: u64, phase: TouchPhase, p: Pos2) -> Event {
    Event::Touch { device_id: TouchDeviceId(7), id: TouchId(id), phase, pos: p, force: None }
}

/// A finger down at `p`, as a phone's browser sends it: `pointerdown`, then `touchstart`. The
/// web app tells a touch-first device so (`expect_touch`).
fn finger_down(h: &mut H, t: f64, p: Pos2) {
    h.state_mut().expect_touch();
    frame(h, t, vec![press(p, true)]);
    frame(h, t + 0.001, vec![press(p, true), touch(1, TouchPhase::Start, p)]);
}

fn finger_move(h: &mut H, t: f64, p: Pos2) {
    frame(h, t, vec![Event::PointerMoved(p), touch(1, TouchPhase::Move, p)]);
}

/// The finger lifts at `p`: `pointerup`, then `touchend`.
fn finger_up(h: &mut H, t: f64, p: Pos2) {
    frame(h, t, vec![press(p, false)]);
    frame(h, t + 0.001, vec![press(p, false), Event::PointerGone, touch(1, TouchPhase::End, p)]);
}

/// A finger dragged from `from` by `by` over ten frames, starting at `t`; the time after.
fn finger_drag(h: &mut H, mut t: f64, from: Pos2, by: Vec2) -> f64 {
    finger_down(h, t, from);
    for k in 1..=10 {
        t += FRAME;
        finger_move(h, t, from + by * (k as f32 / 10.0));
    }
    t += FRAME;
    finger_up(h, t, from + by);
    t
}

/// A mouse drag from `from` by `by`, over ten frames.
fn mouse_drag(h: &mut H, mut t: f64, from: Pos2, by: Vec2) {
    frame(h, t, vec![Event::PointerMoved(from)]);
    t += FRAME;
    frame(h, t, vec![press(from, true)]);
    for k in 1..=10 {
        t += FRAME;
        frame(h, t, vec![Event::PointerMoved(from + by * (k as f32 / 10.0))]);
    }
    t += FRAME;
    frame(h, t, vec![press(from + by, false)]);
    frame(h, t + FRAME, Vec::new());
}

fn idle(h: &mut H, t: &mut f64, frames: usize) {
    for _ in 0..frames {
        *t += FRAME;
        frame(h, *t, Vec::new());
    }
}

// --- The slide view ---------------------------------------------------------------------------

/// A slide with one rectangle, zoomed in so the slide view scrolls; the shape's id.
fn zoomed_with_a_shape(zoom: Option<f32>) -> (H, f64, u32) {
    let mut s = Session::with_new();
    // An empty layout: nothing but the rectangle on the slide.
    s.execute("slide.layout", &json!({"layout": "blank"})).ok();
    let id = s.execute("shape.insert", &json!({"preset": "rect", "rect": [200.0, 150.0, 240.0, 160.0]})).unwrap()["id"].as_u64().unwrap() as u32;
    s.execute("select.none", &json!({})).ok();
    let (mut h, t) = harness(s);
    h.state_mut().ui.zoom = zoom;
    h.run_steps(3);
    h.state_mut().session.doc_mut().unwrap().selection.shapes.clear();
    h.run_steps(2);
    (h, t, id)
}

/// The shape's box (x, y) in slide points.
fn shape_at(h: &H, id: u32) -> (f64, f64) {
    let d = h.state().session.active().unwrap();
    let sh = d.shape(deckcraft_model::ShapeId(id)).unwrap();
    let x = sh.xfrm.unwrap();
    (x.x, x.y)
}

/// The middle of the shape on screen.
fn shape_centre(h: &H) -> Pos2 {
    let (r, s) = (h.state().slide_rect.unwrap(), h.state().canvas_scale);
    pos2(r.min.x + (200.0 + 120.0) * s, r.min.y + (150.0 + 80.0) * s)
}

/// A move of `moved` slide points is the drag's `expected`, give or take the snap to the grid and
/// the smart guides.
fn followed(moved: f64, expected: f64) -> bool {
    (moved - expected).abs() <= expected.abs() * 0.25 + 12.0
}

fn selected(h: &H) -> Vec<u32> {
    h.state().session.active().unwrap().selection.shapes.iter().map(|s| s.0).collect()
}

#[test]
fn a_finger_dragged_over_a_shape_scrolls_the_slide_view_and_moves_nothing() {
    let (mut h, t, id) = zoomed_with_a_shape(Some(2.0));
    let before = h.state().slide_rect.unwrap().min;
    let at = shape_at(&h, id);
    let from = shape_centre(&h);
    let t = finger_drag(&mut h, t, from, vec2(-60.0, -200.0));
    let after = h.state().slide_rect.unwrap().min;
    let moved = after - before;
    assert!((moved.y + 200.0).abs() < 2.0 && (moved.x + 60.0).abs() < 2.0, "the slide follows the finger 1:1: {moved:?}");
    assert_eq!(shape_at(&h, id), at, "the shape under the finger stays where it is");
    assert!(selected(&h).is_empty(), "and nothing is selected: the press never reached the slide");
    // Lifted at speed, the view glides on and slows down.
    let mut t = t;
    let lifted = h.state().slide_rect.unwrap().min.y;
    let mut steps = Vec::new();
    for _ in 0..12 {
        t += FRAME;
        frame(&mut h, t, Vec::new());
        steps.push(h.state().slide_rect.unwrap().min.y);
    }
    let glide = lifted - h.state().slide_rect.unwrap().min.y;
    assert!(glide > 20.0, "it keeps going after the lift: {glide}");
    let (first, last) = (steps[0] - steps[1], steps[10] - steps[11]);
    assert!(last < first, "and decelerates: {first} then {last}");
    assert_eq!(shape_at(&h, id), at);
    assert!(selected(&h).is_empty());
}

#[test]
fn a_tap_on_a_shape_selects_it() {
    let (mut h, mut t, id) = zoomed_with_a_shape(None);
    let before = h.state().slide_rect.unwrap();
    let at = shape_centre(&h);
    finger_down(&mut h, t, at);
    finger_move(&mut h, t + 0.04, at + vec2(2.0, 3.0));
    finger_up(&mut h, t + 0.1, at + vec2(2.0, 3.0));
    t += 0.12;
    idle(&mut h, &mut t, 2);
    assert_eq!(selected(&h), [id], "the tap selected the shape, as a click does");
    assert_eq!(h.state().slide_rect.unwrap(), before, "and scrolled nothing");
}

#[test]
fn a_long_press_then_a_drag_moves_a_shape() {
    let (mut h, mut t, id) = zoomed_with_a_shape(None);
    let (x0, y0) = shape_at(&h, id);
    let before = h.state().slide_rect.unwrap();
    let start = shape_centre(&h);
    finger_down(&mut h, t, start);
    // Held still past the long press, with frames in between as the app asks for them.
    for _ in 0..6 {
        t += 0.1;
        frame(&mut h, t, Vec::new());
    }
    let by = vec2(80.0, 40.0);
    for k in 1..=8 {
        t += FRAME;
        finger_move(&mut h, t, start + by * (k as f32 / 8.0));
    }
    t += FRAME;
    finger_up(&mut h, t, start + by);
    idle(&mut h, &mut t, 2);
    let (x1, y1) = shape_at(&h, id);
    let s = f64::from(h.state().canvas_scale);
    assert!(followed(x1 - x0, 80.0 / s) && followed(y1 - y0, 40.0 / s), "the shape moved with the finger: ({x0}, {y0}) to ({x1}, {y1}) at {s}");
    assert_eq!(h.state().slide_rect.unwrap(), before, "and the view didn't scroll");
}

#[test]
fn two_fingers_pinch_zoom_the_slide_view_about_their_centre() {
    let (mut h, mut t, id) = zoomed_with_a_shape(None);
    let fit = h.state().canvas_scale;
    let c = h.state().canvas_rect.unwrap().center() + vec2(40.0, 20.0);
    let slide_point = |h: &H, p: Pos2| {
        let (r, s) = (h.state().slide_rect.unwrap(), h.state().canvas_scale);
        (p - r.min) / s
    };
    let anchor = slide_point(&h, c);
    let (mut a, mut b) = (c - vec2(60.0, 0.0), c + vec2(60.0, 0.0));
    finger_down(&mut h, t, a);
    t += FRAME;
    frame(&mut h, t, vec![touch(2, TouchPhase::Start, b)]);
    for _ in 0..6 {
        t += FRAME;
        a -= vec2(15.0, 0.0);
        b += vec2(15.0, 0.0);
        frame(&mut h, t, vec![Event::PointerMoved(a), touch(1, TouchPhase::Move, a), touch(2, TouchPhase::Move, b)]);
    }
    let zoomed = h.state().canvas_scale;
    assert!(zoomed > fit * 1.5, "spread apart, the fingers zoomed in: {fit} to {zoomed}");
    let kept = slide_point(&h, c);
    assert!((kept - anchor).length() < 3.0, "about the point between them: {anchor:?} is now under {kept:?}");
    t += FRAME;
    frame(&mut h, t, vec![touch(2, TouchPhase::End, b)]);
    finger_up(&mut h, t + FRAME, a);
    idle(&mut h, &mut t, 2);
    assert!(selected(&h).is_empty(), "and selected nothing");
    let _ = id;
}

#[test]
fn a_finger_swipe_over_a_slide_that_fits_turns_one_slide() {
    let mut s = Session::with_new();
    for _ in 0..4 {
        s.execute("slide.new", &json!({})).unwrap();
    }
    s.execute("slide.go", &json!({"index": 0})).unwrap();
    let (mut h, mut t) = harness(s);
    let c = h.state().canvas_rect.unwrap().center();
    // A fast flick left: it glides after the lift, still one slide.
    t = finger_drag(&mut h, t, c + vec2(150.0, 0.0), vec2(-300.0, 0.0));
    idle(&mut h, &mut t, 60);
    assert_eq!(h.state().session.active().unwrap().selection.slide, 1, "one swipe, one slide on");
    let _ = finger_drag(&mut h, t + 1.0, c - vec2(150.0, 0.0), vec2(300.0, 0.0));
    idle(&mut h, &mut t, 60);
    assert_eq!(h.state().session.active().unwrap().selection.slide, 0, "and back");
}

#[test]
fn a_mouse_drag_still_moves_a_shape_at_once() {
    let (mut h, t, id) = zoomed_with_a_shape(None);
    let (x0, y0) = shape_at(&h, id);
    let before = h.state().slide_rect.unwrap();
    let from = shape_centre(&h);
    mouse_drag(&mut h, t, from, vec2(80.0, 40.0));
    let (x1, y1) = shape_at(&h, id);
    let s = f64::from(h.state().canvas_scale);
    assert!(
        followed(x1 - x0, 80.0 / s) && followed(y1 - y0, 40.0 / s),
        "a mouse drag moves the shape, as before: ({x0}, {y0}) to ({x1}, {y1}) at {s}"
    );
    assert_eq!(h.state().slide_rect.unwrap(), before, "and doesn't scroll");
}

// --- The slide thumbnails -----------------------------------------------------------------------

fn deck(slides: usize) -> Session {
    let mut s = Session::with_new();
    while s.doc().unwrap().doc.slides.len() < slides {
        s.execute("slide.new", &json!({})).unwrap();
    }
    s.execute("slide.go", &json!({"index": 0})).unwrap();
    s
}

fn order(h: &H) -> Vec<u32> {
    h.state().session.active().unwrap().doc.slides.iter().map(|s| s.id.0).collect()
}

#[test]
fn a_finger_dragged_over_the_thumbnails_scrolls_them_and_moves_no_slide() {
    let (mut h, mut t) = harness(deck(30));
    let ids = order(&h);
    assert_eq!(h.state().ui.thumbs_scroll, 0.0);
    let from = h.get_by_label("Slide 3").rect().center();
    t = finger_drag(&mut h, t, from, vec2(0.0, -250.0));
    let scrolled = h.state().ui.thumbs_scroll;
    assert!((scrolled - 250.0).abs() < 2.0, "the thumbnails followed the finger: {scrolled}");
    idle(&mut h, &mut t, 20);
    assert!(h.state().ui.thumbs_scroll > scrolled + 10.0, "and glide on after the lift");
    assert_eq!(order(&h), ids, "no slide was dragged");
    assert_eq!(h.state().session.active().unwrap().selection.slide, 0, "or selected");
}

#[test]
fn a_tap_on_a_thumbnail_selects_its_slide() {
    let (mut h, mut t) = harness(deck(6));
    let at = h.get_by_label("Slide 3").rect().center();
    finger_down(&mut h, t, at);
    finger_up(&mut h, t + 0.1, at);
    t += 0.12;
    idle(&mut h, &mut t, 2);
    assert_eq!(h.state().session.active().unwrap().selection.slide, 2);
    assert_eq!(h.state().ui.thumbs_scroll, 0.0);
}

#[test]
fn a_long_press_then_a_drag_reorders_thumbnails() {
    let (mut h, mut t) = harness(deck(4));
    let ids = order(&h);
    let (second, first) = (h.get_by_label("Slide 2").rect(), h.get_by_label("Slide 1").rect());
    let (from, to) = (second.center(), pos2(first.center().x, first.top() + 4.0));
    finger_down(&mut h, t, from);
    for _ in 0..6 {
        t += 0.1;
        frame(&mut h, t, Vec::new());
    }
    for k in 1..=8 {
        t += FRAME;
        finger_move(&mut h, t, from + (to - from) * (k as f32 / 8.0));
    }
    t += FRAME;
    finger_up(&mut h, t, to);
    idle(&mut h, &mut t, 2);
    assert_eq!(order(&h), [ids[1], ids[0], ids[2], ids[3]], "held, the finger moves the slide");
}

// --- The notes pane -----------------------------------------------------------------------------

#[test]
fn a_finger_dragged_over_the_notes_scrolls_them_and_a_tap_puts_the_caret_there() {
    let mut s = Session::with_new();
    let notes = (1..=60).map(|i| format!("Speaker note line {i}")).collect::<Vec<_>>().join("\n");
    s.execute("slide.notes", &json!({"text": notes})).unwrap();
    let (mut h, mut t) = harness(s);
    let id = egui::Id::unique("notes_edit");
    let field = h.ctx.read_response(id).expect("the notes field").rect;
    let visible = h.ctx.read_response(id).unwrap().interact_rect;
    let from = pos2(visible.center().x, visible.bottom() - 10.0);
    t = finger_drag(&mut h, t, from, vec2(0.0, -40.0));
    idle(&mut h, &mut t, 30);
    let now = h.ctx.read_response(id).unwrap().rect;
    assert!(now.top() < field.top() - 39.0, "the notes scrolled with the finger: {} to {}", field.top(), now.top());
    assert!(!h.ctx.memory(|m| m.has_focus(id)), "a scroll doesn't focus the notes or select in them");
    let at = h.ctx.read_response(id).unwrap().interact_rect.center();
    finger_down(&mut h, t + 1.0, at);
    finger_up(&mut h, t + 1.1, at);
    t += 1.2;
    idle(&mut h, &mut t, 2);
    assert!(h.ctx.memory(|m| m.has_focus(id)), "a tap puts the caret in the notes");
}
