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
//! WASI preview1 imports are wired only when the operator opts in via
//! `[plugins.list.<id>.sandbox]`. Default policy is "no host imports": the
//! linker still adds preview1 stubs but the WasiCtx has no preopens, no
//! env, no network. Pluggable preopen / env / net align with
//! [`crate::config::WasmSandboxConfig`].

#![cfg(feature = "wasm")]

use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use wasmtime::{Engine, Instance, Linker, Module, Store, TypedFunc};
use wasmtime_wasi::preview1::{self, WasiP1Ctx};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

use crate::config::WasmSandboxConfig;
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
        Self::load_with_sandbox(path, &WasmSandboxConfig::default())
    }

    pub fn load_with_sandbox(
        path: &Path,
        sandbox: &WasmSandboxConfig,
    ) -> Result<Self, TransportError> {
        let bytes = std::fs::read(path)
            .map_err(|e| TransportError::Io(format!("read wasm `{}`: {e}", path.display())))?;
        Self::from_bytes_with_sandbox(&bytes, sandbox)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TransportError> {
        Self::from_bytes_with_sandbox(bytes, &WasmSandboxConfig::default())
    }

    pub fn from_bytes_with_sandbox(
        bytes: &[u8],
        sandbox: &WasmSandboxConfig,
    ) -> Result<Self, TransportError> {
        let engine = Engine::default();
        let module = Module::new(&engine, bytes)
            .map_err(|e| TransportError::Io(format!("compile wasm: {e}")))?;
        let wasi = build_wasi_ctx(sandbox)?;
        let mut store = Store::new(&engine, wasi);
        let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
        preview1::add_to_linker_sync(&mut linker, |state: &mut WasiP1Ctx| state)
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

fn build_wasi_ctx(sandbox: &WasmSandboxConfig) -> Result<WasiP1Ctx, TransportError> {
    let mut builder = WasiCtxBuilder::new();
    for path in &sandbox.allow_fs_read {
        let display = path.display().to_string();
        builder
            .preopened_dir(path, &display, DirPerms::READ, FilePerms::READ)
            .map_err(|e| {
                TransportError::Io(format!(
                    "wasm sandbox: cannot preopen read path `{display}`: {e}"
                ))
            })?;
    }
    for path in &sandbox.allow_fs_write {
        let display = path.display().to_string();
        builder
            .preopened_dir(path, &display, DirPerms::all(), FilePerms::all())
            .map_err(|e| {
                TransportError::Io(format!(
                    "wasm sandbox: cannot preopen write path `{display}`: {e}"
                ))
            })?;
    }
    for name in &sandbox.allow_env {
        if let Ok(value) = std::env::var(name) {
            builder.env(name, value);
        }
    }
    if sandbox.allow_net {
        builder.allow_tcp(true);
        builder.allow_udp(true);
        builder.allow_ip_name_lookup(true);
    }
    Ok(builder.build_p1())
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
