//! DevTools bridge for page JavaScript.
//!
//! Exposes `__orinium_devtools(method, paramsJson)`, returning a
//! `Promise<string>` with a JSON envelope. Requests are handled by the
//! browser UI and resolved via [`JsRuntime::resolve_devtools`].

use pixi_byte::vm::VM;
use pixi_byte::{JSError, JSResult, JSValue};

use super::{JsRuntime, with_host_mut};

/// An inspection request queued by page JavaScript via
/// `__orinium_devtools`, waiting to be answered by the browser UI.
#[derive(Debug)]
pub struct JsDevToolsRequest {
    pub(crate) id: u64,
    pub(crate) method: String,
    /// JSON-encoded parameters produced by the caller.
    pub(crate) params: String,
}

/// A pending DevTools promise waiting for the browser UI's answer.
pub(crate) struct JsDevToolsCapability {
    resolve: JSValue,
}

const DEVTOOLS_GLOBAL: &str = "__orinium_devtools";

/// Installs the DevTools inspection global on the engine.
pub(super) fn install(engine: &mut pixi_byte::JSEngine) {
    engine.global_mut().borrow_mut().set(
        DEVTOOLS_GLOBAL.to_string(),
        JSValue::NativeFunction(inspect),
    );
}

fn inspect(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let JSValue::String(method) = args.get(1).cloned().unwrap_or(JSValue::Undefined) else {
        return Err(JSError::TypeError(
            "__orinium_devtools requires a method string".to_string(),
        ));
    };
    let params = match args.get(2).cloned().unwrap_or(JSValue::Undefined) {
        JSValue::String(params) => params,
        JSValue::Undefined | JSValue::Null => "{}".to_string(),
        _ => {
            return Err(JSError::TypeError(
                "__orinium_devtools params must be a JSON string".to_string(),
            ));
        }
    };

    let promise_constructor = vm.global_object.borrow().get("Promise");
    let JSValue::Object(constructor) = &promise_constructor else {
        return Err(JSError::InternalError(
            "Promise constructor is unavailable".to_string(),
        ));
    };
    let construct = constructor.borrow().get("__construct__");
    let _ = with_host_mut(vm, |host| host.constructing_devtools_capability = None);
    let promise = vm.call(
        construct,
        promise_constructor,
        vec![JSValue::NativeFunction(capture_capability)],
    )?;
    let capability = with_host_mut(vm, |host| host.constructing_devtools_capability.take())
        .flatten()
        .ok_or_else(|| JSError::InternalError("Failed to create DevTools Promise".to_string()))?;

    let _ = with_host_mut(vm, |host| {
        host.next_devtools_id = host.next_devtools_id.wrapping_add(1);
        let id = host.next_devtools_id;
        host.devtools_capabilities.insert(id, capability);
        host.devtools_requests
            .push(JsDevToolsRequest { id, method, params });
    });
    Ok(promise)
}

fn capture_capability(vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let resolve = args.get(1).cloned().unwrap_or(JSValue::Undefined);
    let Some(()) = with_host_mut(vm, |host| {
        host.constructing_devtools_capability = Some(JsDevToolsCapability { resolve });
    }) else {
        return Err(JSError::InternalError(
            "DevTools host state is unavailable".to_string(),
        ));
    };
    Ok(JSValue::Undefined)
}

impl JsRuntime {
    /// Takes DevTools inspection requests queued by page JavaScript since the
    /// previous call.
    pub(crate) fn take_devtools_requests(&mut self) -> Vec<JsDevToolsRequest> {
        with_host_mut(self.engine.vm(), |host| {
            std::mem::take(&mut host.devtools_requests)
        })
        .unwrap_or_default()
    }

    /// Settles a pending DevTools inspection request with its JSON envelope
    /// and runs the microtask checkpoint so `.then` callbacks observe it.
    pub(crate) fn resolve_devtools(&mut self, id: u64, result: String) {
        let Some(capability) = with_host_mut(self.engine.vm(), |host| {
            host.devtools_capabilities.remove(&id)
        })
        .flatten() else {
            return;
        };
        if let Err(err) = self.engine.call(
            capability.resolve,
            JSValue::Undefined,
            vec![JSValue::String(result)],
        ) {
            log::info!("JS error while resolving devtools request: {}", err);
        }
        self.perform_microtask_checkpoint();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::super::{JsProcessor, JsTask};
    use crate::engine::html::parser::Parser as HtmlParser;
    use crate::engine::layouter::dom_snapshot::DomSnapshot;

    fn wait_for_result(processor: &JsProcessor) -> crate::engine::js::JsTaskResult {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = processor.try_receive() {
                return result;
            }
            assert!(
                Instant::now() < deadline,
                "JS result did not arrive before the timeout"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn devtools_request_round_trip_resolves_the_promise() {
        let dom = HtmlParser::new("<html><body></body></html>").parse();
        let (snapshot, _) = DomSnapshot::from_tree(&dom.root);
        let processor = JsProcessor::new(snapshot);

        processor.send(JsTask::RunScript {
            source: r#"
                globalThis.__result = null;
                __orinium_devtools("getVersion").then(function (json) {
                    globalThis.__result = json;
                });
            "#
            .to_string(),
        });

        let result = wait_for_result(&processor);
        assert_eq!(result.devtools_requests.len(), 1);
        let request = &result.devtools_requests[0];
        assert_eq!(request.method, "getVersion");
        assert_eq!(request.params, "{}");

        processor.send(JsTask::ResolveDevTools {
            id: request.id,
            result: r#"{"ok":true,"data":{"version":7}}"#.to_string(),
        });
        let _ = wait_for_result(&processor);

        processor.send(JsTask::RunScript {
            source: r#"
                if (globalThis.__result !== '{"ok":true,"data":{"version":7}}') {
                    throw new Error("unexpected result: " + globalThis.__result);
                }
                document.body.setAttribute("data-ok", "yes");
            "#
            .to_string(),
        });
        let final_result = wait_for_result(&processor);
        assert!(
            final_result.needs_redraw,
            "the verification script must run without throwing"
        );
    }

    #[test]
    fn devtools_rejects_non_string_method() {
        let dom = HtmlParser::new("<html><body></body></html>").parse();
        let (snapshot, _) = DomSnapshot::from_tree(&dom.root);
        let processor = JsProcessor::new(snapshot);

        processor.send(JsTask::RunScript {
            source: r#"
                try {
                    __orinium_devtools(42);
                    throw new Error("expected __orinium_devtools to reject numbers");
                } catch (error) {
                    document.body.setAttribute("data-rejected", "yes");
                }
            "#
            .to_string(),
        });

        let result = wait_for_result(&processor);
        assert!(result.devtools_requests.is_empty());
        assert!(
            result.needs_redraw,
            "the catch block must have run without throwing"
        );
    }
}
