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
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use wasmtime::{Config, Engine, Instance, Linker, Module, Store, TypedFunc};
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::p1::{self, WasiP1Ctx};

use crate::error::TransportError;
use crate::sdk::PluginError;
use crate::transport::PluginTransport;

const ERROR_FLAG: u64 = 1 << 31;
const FUEL_PER_DISPATCH: u64 = 100_000_000;
const MAX_WASM_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
static WASM_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

/// Plugin transport over a WASM module.
pub struct WasmTransport {
    engine: Engine,
    inner: Arc<Mutex<WasmInner>>,
    dispatch_slot: Arc<tokio::sync::Semaphore>,
}

struct WasmInner {
    store: Store<WasiP1Ctx>,
    instance: Instance,
    alloc: TypedFunc<i32, i32>,
    dispatch: TypedFunc<(i32, i32, i32, i32), i64>,
}

/// Interrupt only the guest execution owned by this call. The VM slot stays
/// with the worker until it exits; cancelling an already finished call must
/// never advance the epoch of a successor running on the same instance.
struct GuestCall {
    engine: Engine,
    state: Mutex<GuestCallState>,
}

#[derive(Default)]
struct GuestCallState {
    cancelled: bool,
    running: bool,
}

impl GuestCall {
    fn new(engine: Engine) -> Arc<Self> {
        Arc::new(Self {
            engine,
            state: Mutex::new(GuestCallState::default()),
        })
    }

    fn start(
        self: &Arc<Self>,
        store: &mut Store<WasiP1Ctx>,
    ) -> Result<GuestExecution, TransportError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.cancelled {
            return Err(TransportError::Cancelled);
        }
        store.set_epoch_deadline(1);
        state.running = true;
        Ok(GuestExecution(Arc::clone(self)))
    }

    fn cancel(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.cancelled = true;
        if state.running {
            self.engine.increment_epoch();
        }
    }
}

struct GuestExecution(Arc<GuestCall>);

impl Drop for GuestExecution {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .running = false;
    }
}

struct CancelGuestOnDrop(Arc<GuestCall>);

impl Drop for CancelGuestOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl WasmTransport {
    pub fn load(path: &Path) -> Result<Self, TransportError> {
        let bytes = std::fs::read(path)
            .map_err(|e| TransportError::Io(format!("read wasm `{}`: {e}", path.display())))?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TransportError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config)
            .map_err(|error| TransportError::Io(format!("configure wasm engine: {error:#}")))?;
        let module = Module::new(&engine, bytes)
            .map_err(|e| TransportError::Io(format!("compile wasm: {e:#}")))?;
        let wasi = build_wasi_ctx();
        let mut store = Store::new(&engine, wasi);
        store.set_epoch_deadline(1);
        // Wasmtime stores start with zero fuel. Instantiation can run guest
        // code (the start function and memory/table initializers), so it needs
        // its own bounded budget before the first dispatch ever occurs.
        store.set_fuel(FUEL_PER_DISPATCH).map_err(|error| {
            TransportError::Io(format!("set wasm initialization fuel: {error:#}"))
        })?;
        let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
        p1::add_to_linker_sync(&mut linker, |state: &mut WasiP1Ctx| state)
            .map_err(|e| TransportError::Io(format!("link wasi preview1: {e:#}")))?;
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| TransportError::Io(format!("instantiate wasm: {e:#}")))?;
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
            engine,
            inner: Arc::new(Mutex::new(WasmInner {
                store,
                instance,
                alloc,
                dispatch,
            })),
            dispatch_slot: Arc::new(tokio::sync::Semaphore::new(1)),
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
        let dispatch_slot = Arc::clone(&self.dispatch_slot)
            .acquire_owned()
            .await
            .map_err(|error| {
                TransportError::disconnected_error(
                    "Wasm plugin dispatch limiter closed before a slot was acquired",
                    &error,
                )
            })?;

        let inner = Arc::clone(&self.inner);
        let call = GuestCall::new(self.engine.clone());
        let _cancel = CancelGuestOnDrop(Arc::clone(&call));
        WASM_WORKERS
            .run(move || {
                let _dispatch_slot = dispatch_slot;
                let params_bytes = serde_json::to_vec(&params)?;
                if method_bytes.len() > MAX_WASM_MESSAGE_BYTES
                    || params_bytes.len() > MAX_WASM_MESSAGE_BYTES
                {
                    return Err(TransportError::Rpc(format!(
                        "wasm request exceeds the {MAX_WASM_MESSAGE_BYTES}-byte limit"
                    )));
                }
                let mut guard = inner.lock().map_err(|error| {
                    TransportError::Io(format!("wasm transport lock poisoned: {error}"))
                })?;
                let inner = &mut *guard;
                let _execution = call.start(&mut inner.store)?;
                inner
                    .store
                    .set_fuel(FUEL_PER_DISPATCH)
                    .map_err(|error| TransportError::Io(format!("reset wasm fuel: {error:#}")))?;
                let memory = inner
                    .instance
                    .get_memory(&mut inner.store, "memory")
                    .ok_or_else(|| {
                        TransportError::Io("wasm module has no exported memory".into())
                    })?;

                let method_ptr = inner
                    .alloc
                    .call(&mut inner.store, method_bytes.len() as i32)
                    .map_err(|e| TransportError::Io(format!("agena_alloc method: {e:#}")))?;
                let params_ptr = inner
                    .alloc
                    .call(&mut inner.store, params_bytes.len() as i32)
                    .map_err(|e| TransportError::Io(format!("agena_alloc params: {e:#}")))?;

                if method_ptr < 0 || params_ptr < 0 {
                    return Err(TransportError::Io(
                        "wasm allocator returned a negative memory pointer".to_string(),
                    ));
                }

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
                    .map_err(|e| TransportError::Io(format!("agena_dispatch: {e:#}")))?;
                let packed = packed as u64;
                let result_ptr = (packed >> 32) as usize;
                let raw_len = (packed & 0xFFFF_FFFF) as u32;
                let is_err = (raw_len as u64) & ERROR_FLAG != 0;
                let result_len = (raw_len & 0x7FFF_FFFF) as usize;
                if result_len > MAX_WASM_MESSAGE_BYTES {
                    return Err(TransportError::Rpc(format!(
                        "wasm response exceeds the {MAX_WASM_MESSAGE_BYTES}-byte limit"
                    )));
                }
                let mut buf = vec![0u8; result_len];
                if result_len > 0 {
                    memory
                        .read(&inner.store, result_ptr, &mut buf)
                        .map_err(|e| TransportError::Io(format!("wasm memory read: {e}")))?;
                }
                // Keep large result decoding on the same bounded worker as guest execution.
                drop(guard);
                if is_err {
                    let error: PluginError = serde_json::from_slice(&buf)
                        .unwrap_or_else(|_| PluginError::internal(String::from_utf8_lossy(&buf)));
                    return Err(TransportError::Plugin(error));
                }
                if buf.is_empty() {
                    return Ok(serde_json::Value::Null);
                }
                serde_json::from_slice(&buf).map_err(TransportError::from)
            })
            .await
            .map_err(|error| TransportError::Io(format!("wasm dispatch worker failed: {error}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // (module (func (export "spin") (loop br 0)) (func (export "noop")))
    // The infinite guest has no fuel limit: only request cancellation can stop it.
    const GUEST: &[u8] = &[
        0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 3, 2, 0, 0, 7, 15, 2, 4, 115, 112, 105,
        110, 0, 0, 4, 110, 111, 111, 112, 0, 1, 10, 12, 2, 7, 0, 3, 64, 12, 0, 11, 11, 2, 0, 11,
    ];

    fn guest() -> (Engine, Store<WasiP1Ctx>, Instance) {
        let mut config = Config::new();
        config.epoch_interruption(true);
        let engine = Engine::new(&config).unwrap();
        let module = Module::from_binary(&engine, GUEST).unwrap();
        let mut store = Store::new(&engine, build_wasi_ctx());
        store.set_epoch_deadline(1);
        let instance = Instance::new(&mut store, &module, &[]).unwrap();
        (engine, store, instance)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_request_interrupts_executing_guest() {
        let (engine, mut store, instance) = guest();
        let spin = instance
            .get_typed_func::<(), ()>(&mut store, "spin")
            .unwrap();
        let call = GuestCall::new(engine.clone());
        let cancellation = CancelGuestOnDrop(Arc::clone(&call));
        let (started, ready) = tokio::sync::oneshot::channel();
        let mut worker = tokio::task::spawn_blocking(move || {
            let _execution = call.start(&mut store).unwrap();
            started.send(()).unwrap();
            spin.call(&mut store, ())
        });
        ready.await.unwrap();
        drop(cancellation);
        match tokio::time::timeout(Duration::from_secs(2), &mut worker).await {
            Ok(result) => assert!(result.unwrap().is_err()),
            Err(_) => {
                engine.increment_epoch();
                let _ = worker.await;
                panic!("cancelled guest kept executing");
            }
        }
    }

    #[test]
    fn cancelling_finished_request_preserves_successor_deadline() {
        let (engine, mut store, instance) = guest();
        let noop = instance
            .get_typed_func::<(), ()>(&mut store, "noop")
            .unwrap();
        let previous = GuestCall::new(engine.clone());
        drop(previous.start(&mut store).unwrap());
        let successor = GuestCall::new(engine);
        let _execution = successor.start(&mut store).unwrap();
        previous.cancel();
        noop.call(&mut store, ()).unwrap();
    }
}
