#![allow(non_camel_case_types)]

//! C ABI for cdylib plugins. Hook requests use
//! `dispatch(request_id, method, params)`; an independent `cancel(request_id)`
//! control entry point can interrupt the request without waiting in its queue.
//! Hook semantics live above this boundary as JSON.

use abi_stable::{
    StableAbi,
    library::RootModule,
    package_version_strings,
    sabi_types::VersionStrings,
    std_types::{RResult, RString},
};

pub const ABI_VERSION: u32 = 2;

#[repr(C)]
#[derive(StableAbi)]
#[sabi(kind(Prefix(prefix_ref = AgenaPluginCdylib_Ref)))]
#[sabi(missing_field(panic))]
/// Stable-ABI struct exported by a cdylib plugin.
pub struct AgenaPluginCdylib {
    pub abi_version: u32,
    pub dispatch: extern "C" fn(
        request_id: RString,
        method: RString,
        params: RString,
    ) -> RResult<RString, RString>,
    /// Control bypasses the plugin's dispatch actor, so a running request can
    /// be cancelled without waiting for itself to finish.
    pub cancel: extern "C" fn(request_id: RString) -> bool,
    #[sabi(last_prefix_field)]
    pub shutdown: extern "C" fn(),
}

impl RootModule for AgenaPluginCdylib_Ref {
    abi_stable::declare_root_module_statics! { AgenaPluginCdylib_Ref }

    const BASE_NAME: &'static str = "agena_plugin";
    const NAME: &'static str = "agena_plugin";
    const VERSION_STRINGS: VersionStrings = package_version_strings!();
}
