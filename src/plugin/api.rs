use std::collections::BTreeMap;

use abi_stable::{
    StableAbi,
    library::RootModule,
    package_version_strings,
    sabi_types::VersionStrings,
    std_types::{ROption, RResult, RString, RVec},
};

use crate::tool::{ToolBehavior, ToolSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub behavior: ToolBehavior,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginToolCallRequest {
    pub tool_name: String,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: String,
    pub input_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginToolCallResponse {
    pub title: String,
    pub output_text: String,
    pub payload_json: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginBeforeToolRequest {
    pub tool_name: String,
    pub source: ToolSource,
    pub session_id: i64,
    pub call_id: i64,
    pub input_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginBeforeToolResponse {
    pub input_json: String,
    pub title_override: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

impl PluginBeforeToolResponse {
    pub fn passthrough(input_json: impl Into<String>) -> Self {
        Self {
            input_json: input_json.into(),
            title_override: None,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginAfterToolRequest {
    pub tool_name: String,
    pub source: ToolSource,
    pub session_id: i64,
    pub call_id: i64,
    pub title: String,
    pub output_text: String,
    pub payload_json: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginAfterToolResponse {
    pub title: Option<String>,
    pub output_text: Option<String>,
    pub payload_json: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

impl Default for PluginAfterToolResponse {
    fn default() -> Self {
        Self {
            title: None,
            output_text: None,
            payload_json: None,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginShellEnvRequest {
    pub cwd: String,
    pub session_id: Option<i64>,
    pub call_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginShellEnvResponse {
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginError {
    pub message: String,
}

impl PluginError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<String> for PluginError {
    fn from(value: String) -> Self {
        Self { message: value }
    }
}

impl From<&str> for PluginError {
    fn from(value: &str) -> Self {
        Self {
            message: value.to_owned(),
        }
    }
}

pub trait AgenaPlugin: Send + Sync + 'static {
    fn metadata(&self) -> PluginMetadata;

    fn tools(&self) -> Vec<PluginToolDescriptor> {
        Vec::new()
    }

    fn invoke_tool(
        &self,
        request: PluginToolCallRequest,
    ) -> Result<PluginToolCallResponse, PluginError> {
        Err(PluginError::new(format!(
            "plugin '{}' does not implement tool '{}'",
            self.metadata().name,
            request.tool_name
        )))
    }

    fn before_tool(
        &self,
        request: PluginBeforeToolRequest,
    ) -> Result<PluginBeforeToolResponse, PluginError> {
        Ok(PluginBeforeToolResponse::passthrough(request.input_json))
    }

    fn after_tool(
        &self,
        _request: PluginAfterToolRequest,
    ) -> Result<PluginAfterToolResponse, PluginError> {
        Ok(PluginAfterToolResponse::default())
    }

    fn shell_env(
        &self,
        _request: PluginShellEnvRequest,
    ) -> Result<PluginShellEnvResponse, PluginError> {
        Ok(PluginShellEnvResponse::default())
    }
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct PluginMetadataAbi {
    pub name: RString,
    pub version: RString,
    pub description: RString,
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct PluginKeyValueAbi {
    pub key: RString,
    pub value: RString,
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct PluginToolDescriptorAbi {
    pub name: RString,
    pub description: RString,
    pub input_schema_json: RString,
    pub behavior: ToolBehavior,
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct PluginToolCallRequestAbi {
    pub tool_name: RString,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: RString,
    pub input_json: RString,
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct PluginToolCallResponseAbi {
    pub title: RString,
    pub output_text: RString,
    pub payload_json: ROption<RString>,
    pub metadata: RVec<PluginKeyValueAbi>,
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub enum PluginToolSourceAbi {
    Builtin,
    Plugin {
        plugin_name: RString,
    },
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct PluginBeforeToolRequestAbi {
    pub tool_name: RString,
    pub source: PluginToolSourceAbi,
    pub session_id: i64,
    pub call_id: i64,
    pub input_json: RString,
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct PluginBeforeToolResponseAbi {
    pub input_json: RString,
    pub title_override: ROption<RString>,
    pub metadata: RVec<PluginKeyValueAbi>,
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct PluginAfterToolRequestAbi {
    pub tool_name: RString,
    pub source: PluginToolSourceAbi,
    pub session_id: i64,
    pub call_id: i64,
    pub title: RString,
    pub output_text: RString,
    pub payload_json: ROption<RString>,
    pub metadata: RVec<PluginKeyValueAbi>,
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct PluginAfterToolResponseAbi {
    pub title: ROption<RString>,
    pub output_text: ROption<RString>,
    pub payload_json: ROption<RString>,
    pub metadata: RVec<PluginKeyValueAbi>,
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct PluginShellEnvRequestAbi {
    pub cwd: RString,
    pub session_id: ROption<i64>,
    pub call_id: ROption<i64>,
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct PluginShellEnvResponseAbi {
    pub env: RVec<PluginKeyValueAbi>,
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct PluginErrorAbi {
    pub message: RString,
}

impl From<PluginMetadata> for PluginMetadataAbi {
    fn from(value: PluginMetadata) -> Self {
        Self {
            name: value.name.into(),
            version: value.version.into(),
            description: value.description.into(),
        }
    }
}

impl From<PluginMetadataAbi> for PluginMetadata {
    fn from(value: PluginMetadataAbi) -> Self {
        Self {
            name: value.name.into(),
            version: value.version.into(),
            description: value.description.into(),
        }
    }
}

impl From<PluginToolDescriptor> for PluginToolDescriptorAbi {
    fn from(value: PluginToolDescriptor) -> Self {
        Self {
            name: value.name.into(),
            description: value.description.into(),
            input_schema_json: serde_json::to_string(&value.input_schema)
                .unwrap_or_else(|_| "{}".to_owned())
                .into(),
            behavior: value.behavior,
        }
    }
}

impl From<PluginToolDescriptorAbi> for PluginToolDescriptor {
    fn from(value: PluginToolDescriptorAbi) -> Self {
        let input_schema = serde_json::from_str(value.input_schema_json.as_str())
            .unwrap_or_else(|_| serde_json::json!({}));
        Self {
            name: value.name.into(),
            description: value.description.into(),
            input_schema,
            behavior: value.behavior,
        }
    }
}

impl From<PluginToolCallRequestAbi> for PluginToolCallRequest {
    fn from(value: PluginToolCallRequestAbi) -> Self {
        Self {
            tool_name: value.tool_name.into(),
            session_id: value.session_id,
            call_id: value.call_id,
            workspace_root: value.workspace_root.into(),
            input_json: value.input_json.into(),
        }
    }
}

impl From<PluginToolCallRequest> for PluginToolCallRequestAbi {
    fn from(value: PluginToolCallRequest) -> Self {
        Self {
            tool_name: value.tool_name.into(),
            session_id: value.session_id,
            call_id: value.call_id,
            workspace_root: value.workspace_root.into(),
            input_json: value.input_json.into(),
        }
    }
}

impl From<PluginToolCallResponse> for PluginToolCallResponseAbi {
    fn from(value: PluginToolCallResponse) -> Self {
        Self {
            title: value.title.into(),
            output_text: value.output_text.into(),
            payload_json: value.payload_json.map(Into::into).into(),
            metadata: map_to_kv_vec(value.metadata),
        }
    }
}

impl From<PluginToolCallResponseAbi> for PluginToolCallResponse {
    fn from(value: PluginToolCallResponseAbi) -> Self {
        Self {
            title: value.title.into(),
            output_text: value.output_text.into(),
            payload_json: value.payload_json.into_option().map(Into::into),
            metadata: kv_vec_to_map(value.metadata),
        }
    }
}

impl From<PluginBeforeToolRequestAbi> for PluginBeforeToolRequest {
    fn from(value: PluginBeforeToolRequestAbi) -> Self {
        Self {
            tool_name: value.tool_name.into(),
            source: value.source.into(),
            session_id: value.session_id,
            call_id: value.call_id,
            input_json: value.input_json.into(),
        }
    }
}

impl From<PluginBeforeToolRequest> for PluginBeforeToolRequestAbi {
    fn from(value: PluginBeforeToolRequest) -> Self {
        Self {
            tool_name: value.tool_name.into(),
            source: value.source.into(),
            session_id: value.session_id,
            call_id: value.call_id,
            input_json: value.input_json.into(),
        }
    }
}

impl From<PluginBeforeToolResponse> for PluginBeforeToolResponseAbi {
    fn from(value: PluginBeforeToolResponse) -> Self {
        Self {
            input_json: value.input_json.into(),
            title_override: value.title_override.map(Into::into).into(),
            metadata: map_to_kv_vec(value.metadata),
        }
    }
}

impl From<PluginBeforeToolResponseAbi> for PluginBeforeToolResponse {
    fn from(value: PluginBeforeToolResponseAbi) -> Self {
        Self {
            input_json: value.input_json.into(),
            title_override: value.title_override.into_option().map(Into::into),
            metadata: kv_vec_to_map(value.metadata),
        }
    }
}

impl From<PluginAfterToolRequestAbi> for PluginAfterToolRequest {
    fn from(value: PluginAfterToolRequestAbi) -> Self {
        Self {
            tool_name: value.tool_name.into(),
            source: value.source.into(),
            session_id: value.session_id,
            call_id: value.call_id,
            title: value.title.into(),
            output_text: value.output_text.into(),
            payload_json: value.payload_json.into_option().map(Into::into),
            metadata: kv_vec_to_map(value.metadata),
        }
    }
}

impl From<PluginAfterToolRequest> for PluginAfterToolRequestAbi {
    fn from(value: PluginAfterToolRequest) -> Self {
        Self {
            tool_name: value.tool_name.into(),
            source: value.source.into(),
            session_id: value.session_id,
            call_id: value.call_id,
            title: value.title.into(),
            output_text: value.output_text.into(),
            payload_json: value.payload_json.map(Into::into).into(),
            metadata: map_to_kv_vec(value.metadata),
        }
    }
}

impl From<PluginAfterToolResponse> for PluginAfterToolResponseAbi {
    fn from(value: PluginAfterToolResponse) -> Self {
        Self {
            title: value.title.map(Into::into).into(),
            output_text: value.output_text.map(Into::into).into(),
            payload_json: value.payload_json.map(Into::into).into(),
            metadata: map_to_kv_vec(value.metadata),
        }
    }
}

impl From<PluginAfterToolResponseAbi> for PluginAfterToolResponse {
    fn from(value: PluginAfterToolResponseAbi) -> Self {
        Self {
            title: value.title.into_option().map(Into::into),
            output_text: value.output_text.into_option().map(Into::into),
            payload_json: value.payload_json.into_option().map(Into::into),
            metadata: kv_vec_to_map(value.metadata),
        }
    }
}

impl From<PluginShellEnvRequestAbi> for PluginShellEnvRequest {
    fn from(value: PluginShellEnvRequestAbi) -> Self {
        Self {
            cwd: value.cwd.into(),
            session_id: value.session_id.into_option(),
            call_id: value.call_id.into_option(),
        }
    }
}

impl From<PluginShellEnvRequest> for PluginShellEnvRequestAbi {
    fn from(value: PluginShellEnvRequest) -> Self {
        Self {
            cwd: value.cwd.into(),
            session_id: value.session_id.into(),
            call_id: value.call_id.into(),
        }
    }
}

impl From<PluginShellEnvResponse> for PluginShellEnvResponseAbi {
    fn from(value: PluginShellEnvResponse) -> Self {
        Self {
            env: map_to_kv_vec(value.env),
        }
    }
}

impl From<PluginShellEnvResponseAbi> for PluginShellEnvResponse {
    fn from(value: PluginShellEnvResponseAbi) -> Self {
        Self {
            env: kv_vec_to_map(value.env),
        }
    }
}

impl From<PluginError> for PluginErrorAbi {
    fn from(value: PluginError) -> Self {
        Self {
            message: value.message.into(),
        }
    }
}

impl From<PluginErrorAbi> for PluginError {
    fn from(value: PluginErrorAbi) -> Self {
        Self {
            message: value.message.into(),
        }
    }
}

impl From<ToolSource> for PluginToolSourceAbi {
    fn from(value: ToolSource) -> Self {
        match value {
            ToolSource::Builtin => Self::Builtin,
            ToolSource::Plugin { plugin_name } => Self::Plugin {
                plugin_name: plugin_name.into(),
            },
        }
    }
}

impl From<PluginToolSourceAbi> for ToolSource {
    fn from(value: PluginToolSourceAbi) -> Self {
        match value {
            PluginToolSourceAbi::Builtin => ToolSource::Builtin,
            PluginToolSourceAbi::Plugin { plugin_name } => Self::Plugin {
                plugin_name: plugin_name.into(),
            },
        }
    }
}

fn map_to_kv_vec(map: BTreeMap<String, String>) -> RVec<PluginKeyValueAbi> {
    map.into_iter()
        .map(|(key, value)| PluginKeyValueAbi {
            key: key.into(),
            value: value.into(),
        })
        .collect::<Vec<_>>()
        .into()
}

fn kv_vec_to_map(entries: RVec<PluginKeyValueAbi>) -> BTreeMap<String, String> {
    entries
        .into_iter()
        .map(|entry| (entry.key.into(), entry.value.into()))
        .collect()
}

#[repr(C)]
#[derive(StableAbi)]
#[sabi(kind(Prefix(prefix_ref = AgenaPluginModuleRef)))]
pub struct AgenaPluginModule {
    pub metadata: extern "C" fn() -> PluginMetadataAbi,
    pub tools: extern "C" fn() -> RVec<PluginToolDescriptorAbi>,
    pub invoke_tool:
        extern "C" fn(PluginToolCallRequestAbi) -> RResult<PluginToolCallResponseAbi, PluginErrorAbi>,
    pub before_tool:
        extern "C" fn(PluginBeforeToolRequestAbi) -> RResult<PluginBeforeToolResponseAbi, PluginErrorAbi>,
    pub after_tool:
        extern "C" fn(PluginAfterToolRequestAbi) -> RResult<PluginAfterToolResponseAbi, PluginErrorAbi>,
    #[sabi(last_prefix_field)]
    pub shell_env:
        extern "C" fn(PluginShellEnvRequestAbi) -> RResult<PluginShellEnvResponseAbi, PluginErrorAbi>,
}

impl RootModule for AgenaPluginModuleRef {
    abi_stable::declare_root_module_statics! {AgenaPluginModuleRef}

    const BASE_NAME: &'static str = "agena_plugin";
    const NAME: &'static str = "agena_plugin";
    const VERSION_STRINGS: VersionStrings = package_version_strings!();
}

#[macro_export]
macro_rules! export_agena_plugin {
    ($constructor:expr) => {
        fn __agena_plugin_instance() -> &'static dyn $crate::plugin::AgenaPlugin {
            static INSTANCE: ::std::sync::OnceLock<
                ::std::boxed::Box<dyn $crate::plugin::AgenaPlugin>,
            > = ::std::sync::OnceLock::new();
            INSTANCE.get_or_init(|| ::std::boxed::Box::new($constructor)).as_ref()
        }

        extern "C" fn __agena_plugin_metadata() -> $crate::plugin::api::PluginMetadataAbi {
            $crate::plugin::api::PluginMetadataAbi::from(__agena_plugin_instance().metadata())
        }

        extern "C" fn __agena_plugin_tools(
        ) -> ::abi_stable::std_types::RVec<$crate::plugin::api::PluginToolDescriptorAbi> {
            __agena_plugin_instance()
                .tools()
                .into_iter()
                .map($crate::plugin::api::PluginToolDescriptorAbi::from)
                .collect::<::std::vec::Vec<_>>()
                .into()
        }

        extern "C" fn __agena_plugin_invoke_tool(
            request: $crate::plugin::api::PluginToolCallRequestAbi,
        ) -> ::abi_stable::std_types::RResult<
            $crate::plugin::api::PluginToolCallResponseAbi,
            $crate::plugin::api::PluginErrorAbi,
        > {
            match __agena_plugin_instance().invoke_tool(request.into()) {
                ::std::result::Result::Ok(value) => {
                    ::abi_stable::std_types::RResult::ROk(value.into())
                }
                ::std::result::Result::Err(err) => {
                    ::abi_stable::std_types::RResult::RErr(err.into())
                }
            }
        }

        extern "C" fn __agena_plugin_before_tool(
            request: $crate::plugin::api::PluginBeforeToolRequestAbi,
        ) -> ::abi_stable::std_types::RResult<
            $crate::plugin::api::PluginBeforeToolResponseAbi,
            $crate::plugin::api::PluginErrorAbi,
        > {
            match __agena_plugin_instance().before_tool(request.into()) {
                ::std::result::Result::Ok(value) => {
                    ::abi_stable::std_types::RResult::ROk(value.into())
                }
                ::std::result::Result::Err(err) => {
                    ::abi_stable::std_types::RResult::RErr(err.into())
                }
            }
        }

        extern "C" fn __agena_plugin_after_tool(
            request: $crate::plugin::api::PluginAfterToolRequestAbi,
        ) -> ::abi_stable::std_types::RResult<
            $crate::plugin::api::PluginAfterToolResponseAbi,
            $crate::plugin::api::PluginErrorAbi,
        > {
            match __agena_plugin_instance().after_tool(request.into()) {
                ::std::result::Result::Ok(value) => {
                    ::abi_stable::std_types::RResult::ROk(value.into())
                }
                ::std::result::Result::Err(err) => {
                    ::abi_stable::std_types::RResult::RErr(err.into())
                }
            }
        }

        extern "C" fn __agena_plugin_shell_env(
            request: $crate::plugin::api::PluginShellEnvRequestAbi,
        ) -> ::abi_stable::std_types::RResult<
            $crate::plugin::api::PluginShellEnvResponseAbi,
            $crate::plugin::api::PluginErrorAbi,
        > {
            match __agena_plugin_instance().shell_env(request.into()) {
                ::std::result::Result::Ok(value) => {
                    ::abi_stable::std_types::RResult::ROk(value.into())
                }
                ::std::result::Result::Err(err) => {
                    ::abi_stable::std_types::RResult::RErr(err.into())
                }
            }
        }

        #[::abi_stable::export_root_module]
        pub fn agena_plugin_root_module() -> $crate::plugin::api::AgenaPluginModuleRef {
            ::abi_stable::prefix_type::PrefixTypeTrait::leak_into_prefix(
                $crate::plugin::api::AgenaPluginModule {
                metadata: __agena_plugin_metadata,
                tools: __agena_plugin_tools,
                invoke_tool: __agena_plugin_invoke_tool,
                before_tool: __agena_plugin_before_tool,
                after_tool: __agena_plugin_after_tool,
                shell_env: __agena_plugin_shell_env,
                },
            )
        }
    };
}
