//! Missing Web Platform API detector
//!
//! Investigation/diagnostic helper ("log mode"). Scripts probe a wide surface
//! of Web Platform APIs that this engine has not implemented yet; when a page
//! reaches an unimplemented API it fails with a generic `TypeError`. To make
//! those gaps visible without breaking execution, this module emits a
//! `log::info!` report of which global / document / Element.prototype APIs are
//! *absent* at boot time.
//!
//! It is deliberately side-effect free: it only reads, never installs stubs or
//! shadows already-present implementations. Call [`install_missing_api_report`]
//! right after creating a [`crate::engine::js::JsRuntime`] in an investigation
//! harness (`examples/yt.rs`, the youtube fetch test) and grep for
//! `missing-api` in the logs.

use crate::engine::js::JsRuntime;

/// Globals high-traffic sites probe for. Only entries that are actually
/// undefined get reported.
pub(crate) const SUSPECT_GLOBALS: &[&str] = &[
    "AbortController",
    "AbortSignal",
    "Blob",
    "Comment",
    "CustomElementRegistry",
    "Document",
    "DocumentFragment",
    "Element",
    "EventTarget",
    "FileReader",
    "FormData",
    "HTMLAudioElement",
    "HTMLCanvasElement",
    "HTMLMediaElement",
    "HTMLScriptElement",
    "HTMLSlotElement",
    "HTMLTemplateElement",
    "HTMLVideoElement",
    "Headers",
    "HTMLElement",
    "IntersectionObserver",
    "KeyboardEvent",
    "MediaSource",
    "MutationObserver",
    "MouseEvent",
    "Node",
    "PointerEvent",
    "RegExp",
    "Request",
    "Response",
    "ResizeObserver",
    "SVGElement",
    "SVGSVGElement",
    "ShadowRoot",
    "SourceBuffer",
    "Text",
    "TextDecoder",
    "TextEncoder",
    "URL",
    "URLSearchParams",
    "WheelEvent",
    "XMLHttpRequest",
    "cancelAnimationFrame",
    "customElements",
    "getComputedStyle",
    "matchMedia",
    "performance",
    "queueMicrotask",
    "requestAnimationFrame",
];

/// Members probed on `document` (object style: `typeof document[name]`).
///
/// Properties such as `scripts` and `currentScript` live here too: YouTube's
/// bootstrap reads them before it defines anything of its own.
pub(crate) const SUSPECT_DOCUMENT_METHODS: &[&str] = &[
    "adoptedStyleSheets",
    "body",
    "createElementNS",
    "currentScript",
    "documentElement",
    "elementFromPoint",
    "elementsFromPoint",
    "execCommand",
    "fonts",
    "getSelection",
    "head",
    "importNode",
    "pictureInPictureElement",
    "scripts",
];

/// Members probed on `Element.prototype` (object style:
/// `typeof Element.prototype[name]`).
pub(crate) const SUSPECT_ELEMENT_PROTOTYPE_METHODS: &[&str] = &[
    "attachShadow",
    "closest",
    "getAnimations",
    "getRootNode",
    "insertAdjacentElement",
    "matches",
    "requestFullscreen",
    "scrollIntoView",
    "scrollTo",
];

/// Builds the JS prologue that reports which APIs are absent.
///
/// The browser-layer probes inject it after their console collector so the
/// report is captured alongside the page's own warnings.
pub(crate) fn prologue() -> String {
    let mut code = String::new();
    code.push_str("(function(){var missing = [];\n");
    for name in SUSPECT_GLOBALS {
        code.push_str(&format!(
            "if (typeof {name} === 'undefined') missing.push('window.{name}');\n"
        ));
    }
    for name in SUSPECT_DOCUMENT_METHODS {
        code.push_str(&format!(
            "if (typeof document.{name} === 'undefined') missing.push('document.{name}');\n"
        ));
    }
    for name in SUSPECT_ELEMENT_PROTOTYPE_METHODS {
        code.push_str(&format!(
            "if (typeof Element !== 'undefined' && Element.prototype && typeof Element.prototype.{name} === 'undefined') missing.push('Element.prototype.{name}');\n"
        ));
    }
    code.push_str("if (missing.length) console.warn('[missing-api] ' + missing.join(','));\n");
    code.push_str("}).call(window);\n");
    code
}

/// Runs the missing-API report on `runtime` and forwards each name to
/// `log::info!(target: "MissingApi", ...)`.
///
/// Usage in a harness:
/// ```no_run
/// use std::rc::Rc;
/// use orinium_browser::engine::html::Parser;
/// use orinium_browser::engine::js::{JsRuntime, missing_api_detector};
///
/// let dom = Rc::new(Parser::new("<div></div>").parse());
/// let mut runtime = JsRuntime::new(dom);
/// missing_api_detector::install_missing_api_report(&mut runtime);
/// ```
pub fn install_missing_api_report(runtime: &mut JsRuntime) {
    runtime.run_script(&prologue());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::html::Parser;
    use std::rc::Rc;

    #[test]
    fn prologue_evaluates_without_error() {
        let dom = Rc::new(Parser::new(r#"<div></div>"#).parse());
        let mut runtime = JsRuntime::new(dom);
        install_missing_api_report(&mut runtime);
        // No panic means the prologue ran; absence entries are only log lines.
    }
}
