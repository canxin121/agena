//! The worker's JavaScript realm. It has no host functions other than the two
//! bounded callbacks captured by `tools`.

use std::{cell::RefCell, io};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use viperjs::{
    api::{Engine, Error, Host},
    heap::{Heap, NativeCall, PromiseState},
    value::{Completion, Value as JsValue},
    vm::Vm,
};

use super::{MAX_RESULT_BYTES, bounded_json, encode_error, prefix};

const MEMORY_LIMIT: usize = 16 * 1024 * 1024;
const MAX_ARGUMENT_BYTES: usize = 64 * 1024;

// Source is a string value, not interpolated into the wrapper. The async
// function constructor gives the program global scope, not the closure that
// captures the raw callbacks and pristine JSON functions.
const RUN_PROGRAM: &str = r#"
    ((invoke, search, source) => {
        'use strict';
        delete globalThis.__agenaInvoke;
        delete globalThis.__agenaSearch;
        delete globalThis.__agenaSource;
        const parse = JSON.parse;
        const stringify = JSON.stringify;
        const ToolError = Error;
        const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
        const parseResponse = (reply) => {
            const result = parse(reply);
            if (result.error_code) {
                const error = new ToolError(result.error);
                error.code = result.error_code;
                throw error;
            }
            return result;
        };
        Object.defineProperty(globalThis, 'tools', {value: Object.freeze({
            call(name, args = {}) {
                return parseResponse(invoke(name, stringify(args)));
            },
            search(query, limit = 5) {
                return parseResponse(search(query, limit));
            },
        })});
        return (async () => stringify(await AsyncFunction(source)()))();
    })(__agenaInvoke, __agenaSearch, __agenaSource)
"#;

#[derive(Debug, Deserialize, Serialize)]
pub(super) enum Operation {
    Call { name: String, args: Value },
    Search { query: String, limit: usize },
}

type Dispatch = Box<dyn FnMut(Operation) -> io::Result<String>>;

thread_local! {
    // ViperJS natives are function pointers. The worker evaluates one
    // program on its own thread; only the scoped pipe exchange lives here.
    static DISPATCH: RefCell<Option<Dispatch>> = const { RefCell::new(None) };
}

struct ResetDispatch;

impl Drop for ResetDispatch {
    fn drop(&mut self) {
        DISPATCH.replace(None);
    }
}

pub(super) fn run(
    source: &str,
    dispatch: impl FnMut(Operation) -> io::Result<String> + 'static,
) -> Result<(Value, bool), String> {
    let mut engine = Engine::new();
    engine.set_heap_budget(MEMORY_LIMIT);
    // The parent enforces the active deadline, including compile, native
    // built-ins and Promise jobs, without charging time blocked on host I/O.
    engine.bind("__agenaInvoke", 2, call_tool);
    engine.bind("__agenaSearch", 2, search_tools);
    let source: Vec<u16> = "\"use strict\";\n"
        .encode_utf16()
        .chain(source.encode_utf16())
        .collect();
    let source = JsValue::String(engine.heap_mut().intern(&source));
    engine
        .set_global("__agenaSource", source)
        .map_err(engine_error)?;
    let program = {
        let _reset = ResetDispatch;
        DISPATCH.replace(Some(Box::new(dispatch)));
        engine.eval(RUN_PROGRAM)
    };
    if engine.footprint() > MEMORY_LIMIT {
        return Err("Tool script JavaScript exceeded its 16 MiB heap budget.".to_owned());
    }
    let JsValue::Object(id) = program.map_err(engine_error)? else {
        return Err("Tool script program did not return a promise.".to_owned());
    };
    let promise = engine
        .heap_mut()
        .promise(id)
        .ok_or("Tool script program did not return a promise.")?;
    let value = promise.result;
    match promise.state {
        PromiseState::Pending => {
            Err("Tool script program is waiting on an unresolved promise.".to_owned())
        }
        PromiseState::Rejected => Err(engine
            .text(value)
            .map(|error| prefix(&error, 1024).to_owned())
            .unwrap_or_else(|_| "Tool script program threw an exception.".to_owned())),
        PromiseState::Fulfilled => serialize_result(engine.heap_mut(), value),
    }
}

fn engine_error(error: Error) -> String {
    match error {
        Error::Thrown(message) | Error::Syntax { message, .. } => prefix(&message, 1024).to_owned(),
        other => format!("Tool script JavaScript engine failed: {other:?}"),
    }
}

fn serialize_result(heap: &Heap, value: JsValue) -> Result<(Value, bool), String> {
    if matches!(value, JsValue::Undefined) {
        return Ok((Value::Null, false));
    }
    let units = units(heap, value).ok_or("Tool script result was not serialized to JSON.")?;
    // UTF-8 uses at least as many bytes as there are UTF-16 units. Decode
    // enough to detect truncation without copying the entire result.
    let serialized = String::from_utf16_lossy(&units[..units.len().min(MAX_RESULT_BYTES + 1)]);
    if serialized.len() > MAX_RESULT_BYTES {
        // Escaping the preview itself also counts against the result budget.
        let preview = Value::String(prefix(&serialized, MAX_RESULT_BYTES / 8).to_owned());
        debug_assert!(bounded_json(&preview, MAX_RESULT_BYTES).is_some());
        return Ok((preview, true));
    }
    let result = serde_json::from_str(&serialized).map_err(|error| error.to_string())?;
    Ok((result, false))
}

fn units(heap: &Heap, value: JsValue) -> Option<&[u16]> {
    let JsValue::String(id) = value else {
        return None;
    };
    heap.string(id)
}

fn label(heap: &Heap, value: JsValue, limit: usize) -> Option<String> {
    let units = units(heap, value)?;
    let content = |&unit: &u16| !char::from_u32(u32::from(unit)).is_some_and(char::is_whitespace);
    let units = &units[units.iter().position(content)?..=units.iter().rposition(content)?];
    if units.len() > limit {
        return None;
    }
    let text = String::from_utf16(units).ok()?;
    (text.len() <= limit).then_some(text)
}

fn read_call(heap: &Heap, call: &NativeCall<'_>) -> Result<Operation, &'static str> {
    const INVALID: &str = "Supply a tool name and an argument object under 64 KiB.";
    let name = label(heap, call.argument(0), 256).ok_or(INVALID)?;
    let units = units(heap, call.argument(1)).ok_or(INVALID)?;
    if units.len() > MAX_ARGUMENT_BYTES {
        return Err(INVALID);
    }
    let raw = String::from_utf16(units).map_err(|_| INVALID)?;
    if raw.len() > MAX_ARGUMENT_BYTES {
        return Err(INVALID);
    }
    match serde_json::from_str(&raw) {
        Ok(args @ Value::Object(_)) => Ok(Operation::Call { name, args }),
        _ => Err("Tool arguments must be a JSON object."),
    }
}

fn call_tool(vm: &mut Vm, heap: &mut Heap, call: &NativeCall<'_>) -> Completion<JsValue> {
    let response = match read_call(heap, call) {
        Ok(operation) => exchange(operation),
        Err(message) => encode_error("tool_script.invalid_call", message),
    };
    Ok(Host::new(vm, heap).string(&response))
}

fn search_tools(vm: &mut Vm, heap: &mut Heap, call: &NativeCall<'_>) -> Completion<JsValue> {
    let response = match (label(heap, call.argument(0), 512), call.argument(1)) {
        (Some(query), JsValue::Number(limit))
            if limit.fract() == 0.0 && (1.0..=10.0).contains(&limit) =>
        {
            exchange(Operation::Search {
                query,
                limit: limit as usize,
            })
        }
        _ => encode_error(
            "tool_script.invalid_search",
            "Supply a query up to 512 bytes and an integer limit from 1 to 10.",
        ),
    };
    Ok(Host::new(vm, heap).string(&response))
}

fn exchange(operation: Operation) -> String {
    DISPATCH
        .with_borrow_mut(|dispatch| {
            dispatch
                .as_mut()
                .and_then(|dispatch| dispatch(operation).ok())
        })
        .unwrap_or_else(|| {
            encode_error(
                "tool_script.interrupted",
                "Tool script host stopped the invocation.",
            )
        })
}
