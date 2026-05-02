#![allow(non_camel_case_types)]

//! Stable C ABI for cdylib plugins. Only one method goes through the FFI line:
//! `dispatch(method, params)`. All hook semantics live above it as JSON. New
//! hooks therefore never bump the ABI.

use abi_stable::{
    StableAbi,
    library::RootModule,
    package_version_strings,
    sabi_types::VersionStrings,
    std_types::{RResult, RString},
};

pub const ABI_VERSION: u32 = 1;

#[repr(C)]
#[derive(StableAbi)]
#[sabi(kind(Prefix(prefix_ref = AgenaPluginCdylib_Ref)))]
#[sabi(missing_field(panic))]
pub struct AgenaPluginCdylib {
    pub abi_version: u32,
    pub dispatch: extern "C" fn(method: RString, params: RString) -> RResult<RString, RString>,
    #[sabi(last_prefix_field)]
    pub shutdown: extern "C" fn(),
}

impl RootModule for AgenaPluginCdylib_Ref {
    abi_stable::declare_root_module_statics! { AgenaPluginCdylib_Ref }

    const BASE_NAME: &'static str = "agena_plugin";
    const NAME: &'static str = "agena_plugin";
    const VERSION_STRINGS: VersionStrings = package_version_strings!();
}
