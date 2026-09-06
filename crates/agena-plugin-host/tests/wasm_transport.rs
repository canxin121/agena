#![cfg(feature = "wasm")]

use agena_plugin_host::transport::{PluginTransport, wasm::WasmTransport};
use serde_json::json;

fn module(initialization: &str, dispatch: &str) -> Vec<u8> {
    wat::parse_str(format!(
        r#"(module
            (memory (export "memory") 1)
            (global $state (mut i32) (i32.const 0))
            (data (i32.const 0) "{{\"ready\":true}}")
            {initialization}
            (func (export "agena_alloc") (param i32) (result i32) i32.const 128)
            (func (export "agena_dispatch") (param i32 i32 i32 i32) (result i64)
                {dispatch})
        )"#
    ))
    .expect("valid fixture module")
}

#[tokio::test]
async fn wasm_start_function_initializes_before_dispatch() {
    let bytes = module(
        "(func $initialize i32.const 1 global.set $state) (start $initialize)",
        "global.get $state i32.eqz if unreachable end i64.const 14",
    );
    let transport = WasmTransport::from_bytes(&bytes).expect("initialize module");
    assert_eq!(
        transport.dispatch("ping", json!({})).await.unwrap(),
        json!({"ready": true})
    );
}

#[test]
fn wasm_start_function_has_a_finite_execution_budget() {
    let bytes = module(
        "(func $initialize (loop br 0)) (start $initialize)",
        "i64.const 14",
    );
    let error = WasmTransport::from_bytes(&bytes)
        .err()
        .expect("infinite initialization must trap");
    assert!(error.to_string().contains("fuel"), "{error}");
}

#[tokio::test]
async fn wasm_dispatch_gets_fresh_fuel_after_an_exhausted_call() {
    let bytes = module(
        "",
        "global.get $state i32.eqz if
        i32.const 1 global.set $state (loop br 0)
        end i64.const 14",
    );
    let transport = WasmTransport::from_bytes(&bytes).unwrap();
    let error = transport
        .dispatch("loop", json!({}))
        .await
        .expect_err("infinite dispatch must trap");
    assert!(error.to_string().contains("fuel"), "{error}");
    assert_eq!(
        transport.dispatch("ping", json!({})).await.unwrap(),
        json!({"ready": true})
    );
}

#[tokio::test]
async fn wasm_dispatch_rejects_response_outside_linear_memory() {
    // Packed result: pointer 65,536 (one past memory), length 14.
    let bytes = module("", "i64.const 281474976710670");
    let transport = WasmTransport::from_bytes(&bytes).unwrap();
    let error = transport
        .dispatch("ping", json!({}))
        .await
        .expect_err("invalid response range");
    assert!(error.to_string().contains("wasm memory read"), "{error}");
}
