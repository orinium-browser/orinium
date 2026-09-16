//! Web Platform API surface installed into the JS runtime.
//!
//! # Implementation roadmap (youtube.com rendering)
//!
//! Current gap analysis (from `examples/yt.rs` + the youtube fetch test) shows
//! the remaining blockers, roughly in dependency order:
//!
//! 1. **Real constructors for remaining interfaces** — `Document`, `Text`,
//!    `Comment`, `ShadowRoot`, `SVGElement`, `HTMLTemplateElement`,
//!    `KeyboardEvent` and friends are still `undefined`, so any site that does
//!    `instanceof` / feature-detect trips. `Node` and `Element` are done;
//!    mirror that pattern (`make_node_interface` in `element.rs`).
//! 2. **Wire node instances to `Node.prototype`** — Text/Comment/Fragment
//!    objects are still created with a bare `JSObject` (no prototype), so
//!    `getRootNode`/`contains`/`hasChildNodes` stay unreachable from them.
//! 3. **`InnerHTML`/`outerHTML` fidelity** — synthetic HTML parsing differs from
//!    the full parser; template-to-DOM sync in webcomponents-sd needs this.
//! 4. **Shadow DOM semantics depth** — `attachShadow`, `getRootNode` exist;
//!    composed-path events, slotting, and the event retargeting rules are next.
//! 5. **Event dispatch fidelity** — `initCustomEvent`, capture phase, composed
//!    propagation. `CustomEvent` and `createEvent` shapes are in place.
//! 6. **Missing media/network globals** — `HTMLVideoElement`, `XMLHttpRequest`,
//!    `fetch`/`Response` stubs reported by `missing_api_detector`.
//!
//! Run a report to enumerate the current gaps:
//!
//! ```sh
//! YT_MISSING_APIS=1 cargo run --example yt -- <youtube-url> 2>&1 \
//!   | rg 'missing-api'
//! ```

pub(crate) mod browser_env;
pub(crate) mod console;
pub(crate) mod dom;
pub(crate) mod encoding;
pub(crate) mod message_channel;
pub(crate) mod misc;
pub mod missing_api_detector;
pub(crate) mod network;
pub(crate) mod observers;
pub(crate) mod performance;
pub(crate) mod storage;
pub(crate) mod timers;
pub(crate) mod url;
