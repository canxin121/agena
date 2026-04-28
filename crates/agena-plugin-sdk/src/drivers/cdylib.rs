//! Cdylib driver. Use `agena_plugin_sdk::export_cdylib!` from your plugin
//! crate to export an `agena_plugin_root_module` symbol via abi_stable.

use std::sync::OnceLock;

use abi_stable::std_types::{RResult, RString};
use tokio::runtime::Runtime;

use crate::error::PluginError;

#[doc(hidden)]
pub fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("agena cdylib runtime")
    })
}

#[doc(hidden)]
pub fn into_abi_result(value: crate::error::Result<serde_json::Value>) -> RResult<RString, RString> {
    match value {
        Ok(v) => match serde_json::to_string(&v) {
            Ok(s) => RResult::ROk(RString::from(s)),
            Err(e) => RResult::RErr(encode_error(PluginError::invalid_params(e.to_string()))),
        },
        Err(e) => RResult::RErr(encode_error(e)),
    }
}

fn encode_error(err: PluginError) -> RString {
    serde_json::to_string(&err)
        .unwrap_or_else(|_| "{\"code\":\"generic\",\"message\":\"<encode failed>\"}".into())
        .into()
}

/// Export a [`Plugin`] impl as a cdylib. The impl must be `Default + Plugin`.
///
/// ```ignore
/// agena_plugin_sdk::export_cdylib!(EchoPlugin);
/// ```
#[macro_export]
macro_rules! export_cdylib {
    ($plugin_ty:ty) => {
        const _: () = {
            use $crate::abi_stable_reexport as abi_stable;
            use $crate::cdylib_abi::{AgenaPluginCdylib, AgenaPluginCdylib_Ref, ABI_VERSION};
            use $crate::drivers::cdylib::{into_abi_result, runtime};
            use $crate::drivers::dispatch::PluginDispatcher;
            use ::std::sync::OnceLock;

            fn dispatcher() -> &'static PluginDispatcher<$plugin_ty> {
                static D: OnceLock<PluginDispatcher<$plugin_ty>> = OnceLock::new();
                D.get_or_init(|| PluginDispatcher::new(<$plugin_ty as ::core::default::Default>::default()))
            }

            extern "C" fn dispatch(
                method: $crate::abi_stable_reexport::std_types::RString,
                params: $crate::abi_stable_reexport::std_types::RString,
            ) -> $crate::abi_stable_reexport::std_types::RResult<
                $crate::abi_stable_reexport::std_types::RString,
                $crate::abi_stable_reexport::std_types::RString,
            > {
                let result = ::std::panic::catch_unwind(|| {
                    runtime().block_on(async move {
                        let method_str: ::std::string::String = method.into();
                        let params_str: ::std::string::String = params.into();
                        let value: $crate::serde_json::Value = if params_str.is_empty() {
                            $crate::serde_json::Value::Null
                        } else {
                            match $crate::serde_json::from_str(&params_str) {
                                Ok(v) => v,
                                Err(e) => {
                                    return ::std::result::Result::Err(
                                        $crate::error::PluginError::invalid_params(e.to_string()),
                                    );
                                }
                            }
                        };
                        dispatcher().dispatch(&method_str, value).await
                    })
                });
                match result {
                    Ok(value) => into_abi_result(value),
                    Err(_) => into_abi_result(::std::result::Result::Err(
                        $crate::error::PluginError {
                            code: $crate::error::PluginErrorCode::Panicked,
                            message: "plugin panicked".into(),
                            hook: None,
                            plugin: None,
                            data: None,
                        },
                    )),
                }
            }

            extern "C" fn shutdown() {
                let _ = runtime().block_on(async {
                    dispatcher()
                        .dispatch(
                            $crate::rpc::method::META_SHUTDOWN,
                            $crate::serde_json::Value::Null,
                        )
                        .await
                });
            }

            #[::abi_stable::export_root_module]
            pub fn agena_plugin_root_module() -> AgenaPluginCdylib_Ref {
                ::abi_stable::prefix_type::PrefixTypeTrait::leak_into_prefix(
                    AgenaPluginCdylib {
                        abi_version: ABI_VERSION,
                        dispatch,
                        shutdown,
                    },
                )
            }
        };
    };
}
