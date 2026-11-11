//! The web build framed by Clawku (`?host=parent&open=host`, [`Services::hosted`]): one
//! presentation, the one Clawku sent. No Home, New, Close, Recent or second window; Save goes
//! back to Clawku under the presentation's name, a copy never carries that name, Print prints,
//! Share asks Clawku, and the standalone app's links and name field are gone.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::{Services, SlideApp};
use crate::Session;
use crate::backstage::{NAV_W, Page};

/// What Clawku was handed: (name, Save As).
type Saved = Rc<RefCell<Vec<(String, bool)>>>;

/// What the hosted app handed Clawku and the browser.
#[derive(Clone, Default)]
struct Log {
    saved: Saved,
    printed: Rc<RefCell<Vec<String>>>,
    shared: Rc<Cell<usize>>,
}

/// The app as the web shell starts it inside Clawku, with `deck.pptx` from Clawku open; `hosted`
/// false is the same page framed by another host (no Clawku extras).
fn app(hosted: bool) -> (SlideApp, Log) {
    let log = Log::default();
    let (saves, copies, printed, shared) = (log.saved.clone(), log.saved.clone(), log.printed.clone(), log.shared.clone());
    let services = Services {
        host_save: Some(Box::new(move |name: &str, _: &[u8], save_as: bool| {
            saves.borrow_mut().push((name.to_string(), save_as));
            Ok(())
        })),
        download: Some(Box::new(move |name: &str, _: &[u8]| copies.borrow_mut().push((name.to_string(), false)))),
        hosted,
        share: hosted.then(|| Box::new(move || shared.set(shared.get() + 1)) as Box<dyn FnMut()>),
        print: hosted.then(|| Box::new(move |name: &str, _: &[u8]| printed.borrow_mut().push(name.to_string())) as crate::DownloadFn),
        ..Default::default()
    };
    let mut app = SlideApp::new(Session::new(), services);
    app.start_for_host().unwrap();
    let mut s = Session::with_new();
    s.execute("slide.new", &json!({})).unwrap();
    let bytes = deckcraft_engine::cmd::file::save_bytes(&s.doc().unwrap().doc, "pptx").unwrap();
    app.open_from_host(Some("deck.pptx"), &bytes);
    (app, log)
}

fn harness(app: SlideApp) -> Harness<'static, SlideApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

/// Whether the File tab's side menu has an entry called `label`.
fn in_nav(h: &Harness<'_, SlideApp>, label: &str) -> bool {
    h.query_all_by_label(label).any(|n| n.rect().left() < 20.0 && n.rect().right() <= NAV_W + 20.0)
}

/// The File tab opens on Info, without Home, New or Close; Open is Browse (from Clawku) alone,
/// with no Recent list; Info shows no path and no Revert; About has no community or GitHub links.
/// The same app framed by another page keeps all of them.
#[test]
fn hosted_file_tab_shows_only_the_one_presentation() {
    let (mut app, _) = app(true);
    app.ui.recent = vec!["/tmp/old talk.pptx".into()];
    let mut h = harness(app);
    h.state_mut().run("app.backstage", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().backstage.as_ref().map(|b| b.page), Some(Page::Info), "Home gives way to Info");
    for gone in ["Home", "New", "Close"] {
        assert!(!in_nav(&h, gone), "{gone} is Clawku's");
    }
    for kept in ["Open", "Info", "Save", "Save As", "Print", "Export", "About", "Options"] {
        assert!(in_nav(&h, kept), "{kept} stays");
    }
    assert!(h.query_by_label("Copy path").is_none());
    assert!(h.query_by_label("Revert").is_none());
    assert!(h.query_by_label("Location").is_none());
    h.state_mut().run("app.backstage", json!({"page": "open"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Browse").is_some());
    assert!(h.query_by_label("old talk.pptx").is_none(), "no Recent list");
    assert!(h.query_by_label("Recent").is_none());
    h.state_mut().run("app.backstage", json!({"page": "new"})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().backstage.as_ref().map(|b| b.page), Some(Page::Info), "no New page");
    assert!(h.query_by_label("Blank Presentation").is_none());
    h.state_mut().run("app.backstage", json!({"page": "about"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Source on GitHub").is_none());
    assert!(h.query_by_label("Join the Clawku community").is_none());

    let (mut other, _) = self::app(false);
    other.ui.recent = vec!["/tmp/old talk.pptx".into()];
    let mut h = harness(other);
    h.state_mut().run("app.backstage", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().backstage.as_ref().map(|b| b.page), Some(Page::Home));
    for kept in ["Home", "New", "Close"] {
        assert!(in_nav(&h, kept), "{kept} stays when Clawku doesn't host the app");
    }
}

/// Save overwrites Clawku's file under its own name; Save As says it is a new name (Clawku asks
/// for it); an export under the presentation's own name is a copy beside it, never an overwrite.
#[test]
fn hosted_save_save_as_and_copies() {
    let (mut app, log) = app(true);
    app.session.execute("slide.new", &json!({})).unwrap();
    app.save();
    app.save_as_dialog("pptx");
    assert!(app.export_as("pptx", json!({})));
    assert!(app.export_as("pdf", json!({})));
    let want = [("deck.pptx", false), ("deck.pptx", true), ("deck (copy).pptx", false), ("deck.pdf", false)];
    assert_eq!(*log.saved.borrow(), want.map(|(n, s)| (n.to_string(), s)));
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("deck.pptx"));
}

/// Print opens the browser's print dialog: nothing is stored in Clawku.
#[test]
fn hosted_print_prints_instead_of_storing_a_pdf() {
    let (mut app, log) = app(true);
    assert!(app.print_pdf(json!({})));
    assert_eq!(app.run("app.print", json!({})).unwrap()["printed"], true);
    assert_eq!(*log.printed.borrow(), ["deck.pdf", "deck.pdf"]);
    assert!(log.saved.borrow().is_empty(), "Print stored nothing: {:?}", log.saved.borrow());
}

/// The title bar's Share asks Clawku; framed by another page there is no Share button.
#[test]
fn hosted_share_button_asks_the_host() {
    let (app, log) = app(true);
    let mut h = harness(app);
    h.get_by_label("Share").click();
    h.run_steps(2);
    assert_eq!(log.shared.get(), 1);
    let (other, _) = self::app(false);
    let h = harness(other);
    assert!(h.query_by_label("Share").is_none());
}

/// Ctrl+N and Ctrl+W do nothing hosted, and the command palette and Quick Access Toolbar leave
/// out New, Close, Home, New Window and Check for Updates.
#[test]
fn hosted_new_and_close_are_hidden_and_their_keys_do_nothing() {
    let (mut app, _) = app(true);
    app.ui.qat.push(crate::qat::QatItem::new("file.new", "New", crate::icons::Icon::File));
    let mut h = harness(app);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::N);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::W);
    h.run_steps(2);
    assert_eq!(h.state().session.documents().len(), 1, "no new presentation");
    assert!(h.state().dialog.is_none() && h.state().session.active().is_some(), "nothing closed");
    for id in ["file.new", "app.closePresentation", "file.close", "app.home", "view.newWindow", "app.checkUpdates", "file.revert"] {
        assert!(h.state().command_hidden(id), "{id}");
    }
    assert!(!h.state().command_hidden("file.save"));
    assert!(!crate::qat::is_enabled(h.state(), "file.new"));
    assert!(h.query_by_label("New").is_none(), "no New button in the Quick Access Toolbar");
    h.state_mut().palette = Some(("new".into(), 0));
    h.run_steps(2);
    assert!(h.query_by_label("New Presentation").is_none());
    assert!(h.query_by_label("New Window").is_none());

    let (other, _) = self::app(false);
    let mut h = harness(other);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::N);
    h.run_steps(2);
    assert_eq!(h.state().session.documents().len(), 2, "Ctrl+N makes a presentation when not hosted");
    h.state_mut().palette = Some(("new".into(), 0));
    h.run_steps(2);
    assert!(h.query_by_label("New Presentation").is_some(), "the palette keeps New when not hosted");
}

/// The ribbon leaves out New Window, Learn More and the buttons that only say they need the
/// desktop app; Preferences has no name field (Clawku's `?author=` names the user).
#[test]
fn hosted_ribbon_and_preferences_drop_the_standalone_extras() {
    let (app, _) = app(true);
    let mut h = harness(app);
    h.state_mut().run("view.tab", json!({"tab": "View"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("New\nWindow").is_none() && h.query_by_label("New Window").is_none());
    h.state_mut().run("view.tab", json!({"tab": "Review"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Compare").is_none());
    h.state_mut().run("view.tab", json!({"tab": "Slide Show"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Learn\nMore").is_none() && h.query_by_label("Learn More").is_none());
    h.state_mut().run("app.preferences", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Your name (comments):").is_none());

    let (other, _) = self::app(false);
    let mut h = harness(other);
    h.state_mut().run("view.tab", json!({"tab": "View"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("New\nWindow").is_some() || h.query_by_label("New Window").is_some(), "New Window stays when not hosted");
    h.state_mut().run("app.preferences", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Your name (comments):").is_some());
}
