//! Cdylib transport — loads an abi_stable shared library that exports
//! `agena_plugin_root_module`.

use std::path::Path;

use abi_stable::library::RootModule;
use async_trait::async_trait;

use crate::error::TransportError;
use crate::sdk::PluginError;
use crate::sdk::cdylib_abi::AgenaPluginCdylib_Ref;
use crate::transport::PluginTransport;

/// Plugin transport over a cdylib ABI.
pub struct CdylibTransport {
    module: AgenaPluginCdylib_Ref,
}

// SAFETY: `AgenaPluginCdylib_Ref` is a `RootModule` reference returned by
// abi_stable's `load_from_file`. Once loaded the underlying dylib is leaked
// for the process lifetime (abi_stable does not unload), so the inner
// pointer remains valid across threads. The plugin contract requires every
// exported function to be Send + Sync, so it is safe to share `Self`
// across threads.
unsafe impl Send for CdylibTransport {}
// SAFETY: see above — the plugin contract guarantees thread-safe entry
// points and the loaded module is never unloaded.
unsafe impl Sync for CdylibTransport {}

impl CdylibTransport {
    pub fn load(path: &Path) -> Result<Self, TransportError> {
        let module = AgenaPluginCdylib_Ref::load_from_file(path)
            .map_err(|e| TransportError::Io(format!("load cdylib `{}`: {e}", path.display())))?;
        Ok(Self { module })
    }
}

#[async_trait]
impl PluginTransport for CdylibTransport {
    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        let module = self.module;
        let method = method.to_string();
        let params_str = serde_json::to_string(&params)?;
        let join = tokio::task::spawn_blocking(move || {
            let dispatch_fn = module.dispatch();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                dispatch_fn(method.into(), params_str.into())
            }));
            match result {
                Ok(r) => match r.into_result() {
                    Ok(s) => {
                        let s: String = s.into();
                        let v: serde_json::Value = serde_json::from_str(&s)?;
                        Ok::<_, TransportError>(v)
                    }
                    Err(err_str) => {
                        let s: String = err_str.into();
                        let pe: PluginError =
                            serde_json::from_str(&s).unwrap_or_else(|_| PluginError::internal(s));
                        Err(TransportError::Plugin(pe))
                    }
                },
                Err(_) => Err(TransportError::Panicked),
            }
        });
        join.await.map_err(|_| TransportError::Panicked)?
    }

    async fn close(&self) -> Result<(), TransportError> {
        let module = self.module;
        let _ = tokio::task::spawn_blocking(move || (module.shutdown())()).await;
        Ok(())
    }
}
