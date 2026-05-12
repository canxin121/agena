//! WASM transport. Loads a `.wasm` module that exports two functions:
//!
//! ```text
//! agena_alloc(len: i32) -> i32
//! agena_dispatch(method_ptr: i32, method_len: i32, params_ptr: i32, params_len: i32) -> i64
//! ```
//!
//! `agena_dispatch` returns a packed `i64`: high 32 bits = result pointer in
//! the wasm linear memory, low 32 bits = result length. A `0` result length
//! indicates an empty / `null` value. Use the high bit of the length to
//! signal an error (length |= 0x8000_0000).
//!
//! WASI preview1 imports are present for ABI compatibility, but agena does
//! not expose per-plugin hard sandbox configuration. The host policy layer
//! applies to plugin host API calls; direct WASI filesystem, environment, and
//! network access is not configured by agena.

#![cfg(feature = "wasm")]

use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use wasmtime::{Engine, Instance, Linker, Module, Store, TypedFunc};
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::p1::{self, WasiP1Ctx};

use crate::error::TransportError;
use crate::sdk::PluginError;
use crate::transport::PluginTransport;

const ERROR_FLAG: u64 = 1 << 31;

pub struct WasmTransport {
    inner: Mutex<WasmInner>,
}

struct WasmInner {
    store: Store<WasiP1Ctx>,
    instance: Instance,
    alloc: TypedFunc<i32, i32>,
    dispatch: TypedFunc<(i32, i32, i32, i32), i64>,
}

impl WasmTransport {
    pub fn load(path: &Path) -> Result<Self, TransportError> {
        let bytes = std::fs::read(path)
            .map_err(|e| TransportError::Io(format!("read wasm `{}`: {e}", path.display())))?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TransportError> {
        let engine = Engine::default();
        let module = Module::new(&engine, bytes)
            .map_err(|e| TransportError::Io(format!("compile wasm: {e}")))?;
        let wasi = build_wasi_ctx();
        let mut store = Store::new(&engine, wasi);
        let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
        p1::add_to_linker_sync(&mut linker, |state: &mut WasiP1Ctx| state)
            .map_err(|e| TransportError::Io(format!("link wasi preview1: {e}")))?;
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| TransportError::Io(format!("instantiate wasm: {e}")))?;
        let alloc: TypedFunc<i32, i32> = instance
            .get_typed_func(&mut store, "agena_alloc")
            .map_err(|_| {
                TransportError::Io("wasm module is missing `agena_alloc(i32) -> i32`".into())
            })?;
        let dispatch: TypedFunc<(i32, i32, i32, i32), i64> = instance
            .get_typed_func(&mut store, "agena_dispatch")
            .map_err(|_| {
                TransportError::Io(
                    "wasm module is missing `agena_dispatch(i32,i32,i32,i32) -> i64`".into(),
                )
            })?;
        Ok(Self {
            inner: Mutex::new(WasmInner {
                store,
                instance,
                alloc,
                dispatch,
            }),
        })
    }
}

fn build_wasi_ctx() -> WasiP1Ctx {
    WasiCtxBuilder::new().build_p1()
}

#[async_trait]
impl PluginTransport for WasmTransport {
    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        let method_bytes = method.as_bytes().to_vec();
        let params_bytes = serde_json::to_vec(&params)?;

        let result_bytes = tokio::task::block_in_place(|| {
            let mut guard = self.inner.lock().expect("wasm transport poisoned");
            let inner = &mut *guard;
            let memory = inner
                .instance
                .get_memory(&mut inner.store, "memory")
                .ok_or_else(|| TransportError::Io("wasm module has no exported memory".into()))?;

            let method_ptr = inner
                .alloc
                .call(&mut inner.store, method_bytes.len() as i32)
                .map_err(|e| TransportError::Io(format!("agena_alloc method: {e}")))?;
            let params_ptr = inner
                .alloc
                .call(&mut inner.store, params_bytes.len() as i32)
                .map_err(|e| TransportError::Io(format!("agena_alloc params: {e}")))?;

            memory
                .write(&mut inner.store, method_ptr as usize, &method_bytes)
                .map_err(|e| TransportError::Io(format!("wasm memory write: {e}")))?;
            memory
                .write(&mut inner.store, params_ptr as usize, &params_bytes)
                .map_err(|e| TransportError::Io(format!("wasm memory write: {e}")))?;

            let packed = inner
                .dispatch
                .call(
                    &mut inner.store,
                    (
                        method_ptr,
                        method_bytes.len() as i32,
                        params_ptr,
                        params_bytes.len() as i32,
                    ),
                )
                .map_err(|e| TransportError::Io(format!("agena_dispatch: {e}")))?;
            let packed = packed as u64;
            let result_ptr = (packed >> 32) as usize;
            let raw_len = (packed & 0xFFFF_FFFF) as u32;
            let is_err = (raw_len as u64) & ERROR_FLAG != 0;
            let result_len = (raw_len & 0x7FFF_FFFF) as usize;
            let mut buf = vec![0u8; result_len];
            if result_len > 0 {
                memory
                    .read(&inner.store, result_ptr, &mut buf)
                    .map_err(|e| TransportError::Io(format!("wasm memory read: {e}")))?;
            }
            Ok::<(bool, Vec<u8>), TransportError>((is_err, buf))
        })?;

        let (is_err, buf) = result_bytes;
        if is_err {
            let pe: PluginError = serde_json::from_slice(&buf).unwrap_or_else(|_| PluginError {
                code: crate::sdk::PluginErrorCode::Generic,
                message: String::from_utf8_lossy(&buf).to_string(),
                hook: None,
                plugin: None,
                data: None,
            });
            return Err(TransportError::Plugin(pe));
        }
        if buf.is_empty() {
            return Ok(serde_json::Value::Null);
        }
        let value: serde_json::Value = serde_json::from_slice(&buf)?;
        Ok(value)
    }
}
