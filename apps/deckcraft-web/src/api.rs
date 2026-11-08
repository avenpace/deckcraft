//! `window.deckcraft`: the page's command API (`craft-web-api`, storytold/craft-libs).
//!
//! The crate installs `request`, `execute`, `commands`, `docInspect`, `uiInspect` and `screenshot`
//! over the same JSON methods as the desktop control channel (`docs/control-protocol.md`), as
//! promises with a timeout, so an agent driving DeckCraft in a browser tab (or an `<iframe>` in
//! another app) uses the same commands as the CLI and the MCP server:
//!
//! ```js
//! await deckcraft.execute("slide.new", {title: "Roadmap"})   // engine command → its result
//! await deckcraft.request("document.inspect")                // control method → its result
//! await deckcraft.commands()                                 // the command list
//! const {png} = await deckcraft.screenshot()                 // the window as PNG (base64)
//! ```
//!
//! Results resolve with the method's `result`; errors reject with an `Error`.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicU8;
use std::sync::mpsc::{Receiver, TryRecvError};

use craft_web_api::methods::Methods;
use craft_web_api::page::{PageApi, Request};
use deckcraft_ui_egui::{ControlRequest, ControlResponse};
use serde_json::{Value, json};
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

/// The app id in `window.<app>`.
const APP: &str = "deckcraft";

/// How often a waiting call looks for the app's reply, in milliseconds (about a frame).
const POLL_MS: i32 = 16;

thread_local! {
    static QUEUE: RefCell<Vec<ControlRequest>> = const { RefCell::new(Vec::new()) };
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
}

/// Remember the egui context, so a queued request can ask for the frame that answers it.
pub fn connect(ctx: egui::Context) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx));
}

/// The page's requests since the last pass, for the app's control channel.
pub fn take_requests() -> Vec<ControlRequest> {
    QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// A queued control request whose reply has not arrived. Dropping it before the reply (the page's
/// timeout fired) cancels the request if the app has not started it: the page was told it failed,
/// so the app must not run it late.
struct Waiting {
    reply: Receiver<ControlResponse>,
    /// The request's take/abandon gate ([`ControlRequest::state`]), kept for cancelling.
    gate: Arc<AtomicU8>,
    answered: bool,
}

impl Waiting {
    /// The protocol's reply object (`{ok, result}` / `{ok: false, error}`) once the app has
    /// answered or gone away; `None` while the request is still waiting.
    fn poll(&mut self) -> Option<Value> {
        match self.reply.try_recv() {
            Ok(reply) => {
                self.answered = true;
                Some(reply)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.answered = true;
                Some(json!({"ok": false, "error": "request dropped"}))
            }
        }
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if !self.answered {
            ControlRequest::abandon(&self.gate);
        }
    }
}

/// Queue a control request for the next frame and wait for the app's reply, as the protocol's
/// `{ok, result}` / `{ok: false, error}` object.
fn send(method: &str, params: Value) -> Pin<Box<dyn Future<Output = Value>>> {
    let (request, rx) = ControlRequest::new(method, params);
    let gate = request.state.clone();
    QUEUE.with(|q| q.borrow_mut().push(request));
    CTX.with(|c| {
        if let Some(ctx) = c.borrow().as_ref() {
            ctx.request_repaint();
        }
    });
    let mut waiting = Waiting { reply: rx, gate, answered: false };
    Box::pin(async move {
        loop {
            if let Some(reply) = waiting.poll() {
                return reply;
            }
            sleep(POLL_MS).await;
        }
    })
}

/// Yield to the page's event loop for `ms` milliseconds, so polling for a reply never blocks it.
async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = JsFuture::from(promise).await;
}

/// Install `window.deckcraft`: the shared page functions over this app's control methods.
pub fn install() -> Result<(), JsValue> {
    let request: Request = Rc::new(|method, params| send(&method, params));
    Rc::new(PageApi::new(APP, Methods::default(), request)).install()
}
