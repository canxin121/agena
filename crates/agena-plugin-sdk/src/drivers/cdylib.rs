//! Cdylib driver. Use `agena_plugin_sdk::export_cdylib!` from your plugin
//! crate to export an `agena_plugin_root_module` symbol via abi_stable.

use std::sync::OnceLock;

use abi_stable::std_types::{RResult, RString};
use tokio::runtime::Runtime;

use crate::error::PluginError;

#[doc(hidden)]
pub fn runtime() -> crate::error::Result<&'static Runtime> {
    static RT: OnceLock<Result<Runtime, String>> = OnceLock::new();
    match RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| {
                agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to build the Agena cdylib plugin runtime",
                    &error,
                )
            })
    }) {
        Ok(runtime) => Ok(runtime),
        Err(diagnostic) => Err(PluginError::internal(diagnostic)),
    }
}

#[doc(hidden)]
pub fn into_abi_result(
    value: crate::error::Result<serde_json::Value>,
) -> RResult<RString, RString> {
    match value {
        Ok(v) => match serde_json::to_string(&v) {
            Ok(s) => RResult::ROk(RString::from(s)),
            Err(e) => RResult::RErr(encode_error(PluginError::invalid_params_error(&e))),
        },
        Err(e) => RResult::RErr(encode_error(e)),
    }
}

fn encode_error(err: PluginError) -> RString {
    match serde_json::to_string(&err) {
        Ok(encoded) => encoded.into(),
        Err(error) => {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to encode a cdylib plugin error for the ABI boundary",
                    &error,
                ),
                original_plugin_diagnostic = %err.diagnostic_message(),
                "using the fixed cdylib ABI error fallback"
            );
            "{\"kind\":\"internal\",\"failure\":{\"code\":\"plugin.internal\"},\"diagnostic\":{\"message\":\"cdylib error encoding failed; inspect the plugin log\"}}"
                .into()
        }
    }
}

#[doc(hidden)]
pub fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        format!("non-string panic payload of type {:?}", payload.type_id())
    }
}

#[doc(hidden)]
pub fn report_shutdown_error(error: &PluginError) {
    tracing::error!(
        diagnostic = %error.diagnostic_message(),
        "cdylib plugin shutdown failed"
    );
}

#[doc(hidden)]
pub fn report_shutdown_panic(payload: &(dyn std::any::Any + Send)) {
    tracing::error!(
        panic = %panic_payload_message(payload),
        "cdylib plugin panicked during shutdown"
    );
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
            use ::std::sync::OnceLock;
            use $crate::abi_stable_reexport as abi_stable;
            use $crate::cdylib_abi::{ABI_VERSION, AgenaPluginCdylib, AgenaPluginCdylib_Ref};
            use $crate::drivers::cdylib::{
                into_abi_result, panic_payload_message, report_shutdown_error,
                report_shutdown_panic, runtime,
            };
            use $crate::drivers::dispatch::PluginDispatcher;

            fn dispatcher() -> &'static PluginDispatcher<$plugin_ty> {
                static D: OnceLock<PluginDispatcher<$plugin_ty>> = OnceLock::new();
                D.get_or_init(|| {
                    PluginDispatcher::new(<$plugin_ty as ::core::default::Default>::default())
                })
            }

            extern "C" fn dispatch(
                method: $crate::abi_stable_reexport::std_types::RString,
                params: $crate::abi_stable_reexport::std_types::RString,
            ) -> $crate::abi_stable_reexport::std_types::RResult<
                $crate::abi_stable_reexport::std_types::RString,
                $crate::abi_stable_reexport::std_types::RString,
            > {
                let result = ::std::panic::catch_unwind(|| {
                    let runtime = runtime()?;
                    runtime.block_on(async move {
                        let method_str: ::std::string::String = method.into();
                        let params_str: ::std::string::String = params.into();
                        let value: $crate::serde_json::Value = if params_str.is_empty() {
                            $crate::serde_json::Value::Null
                        } else {
                            match $crate::serde_json::from_str(&params_str) {
                                Ok(v) => v,
                                Err(e) => {
                                    return ::std::result::Result::Err(
                                        $crate::error::PluginError::invalid_params_error(&e),
                                    );
                                }
                            }
                        };
                        dispatcher().dispatch(&method_str, value).await
                    })
                });
                match result {
                    Ok(value) => into_abi_result(value),
                    Err(payload) => into_abi_result(::std::result::Result::Err(
                        $crate::error::PluginError::from_kind(
                            $crate::error::PluginErrorKind::Panicked,
                            format_args!(
                                "plugin panicked: {}",
                                panic_payload_message(payload.as_ref())
                            ),
                        ),
                    )),
                }
            }

            extern "C" fn shutdown() {
                let result = ::std::panic::catch_unwind(|| {
                    let runtime = runtime()?;
                    runtime.block_on(async {
                        dispatcher()
                            .dispatch(
                                $crate::rpc::method::META_SHUTDOWN,
                                $crate::serde_json::Value::Null,
                            )
                            .await
                    })
                });
                match result {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => report_shutdown_error(&error),
                    Err(payload) => report_shutdown_panic(payload.as_ref()),
                }
            }

            #[::abi_stable::export_root_module]
            pub fn agena_plugin_root_module() -> AgenaPluginCdylib_Ref {
                ::abi_stable::prefix_type::PrefixTypeTrait::leak_into_prefix(AgenaPluginCdylib {
                    abi_version: ABI_VERSION,
                    dispatch,
                    shutdown,
                })
            }
        };
    };
}
