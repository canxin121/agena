//! Unit-level smoke for the WASM transport. Real round-trip integration
//! requires a wasm fixture binary; here we just validate that loading
//! malformed bytes is rejected without panicking.

#![cfg(feature = "wasm")]

use agena_plugin_host::transport::wasm::WasmTransport;

#[test]
fn rejects_malformed_wasm() {
    let result = WasmTransport::from_bytes(b"not a wasm module");
    assert!(result.is_err());
}
