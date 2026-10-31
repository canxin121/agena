//! Adapters from a `Plugin` impl to a transport entrypoint.
//!
//! A plugin author implements `Plugin` and then uses ONE of the `export_*!`
//! macros to ship it as a particular transport.

pub mod dispatch;

#[cfg(feature = "cdylib")]
pub mod cdylib;

#[cfg(feature = "stdio")]
pub mod stdio;

#[cfg(feature = "http")]
pub mod http;

// Only callbacks supported by both remote transports belong to this list.
// The HostClient trait remains the public contract. This private declaration
// shares its mechanical implementations, not transport admission or authority.
#[cfg(any(feature = "stdio", feature = "http"))]
macro_rules! impl_callback_host_client {
    ($client:ty, $context:path, { $($special:item)* }) => {
        $crate::drivers::impl_callback_host_client!(@methods remote
            ($client, $context, { $($special)* }));
    };
    (@proxy $client:ty, { $($special:item)* }) => {
        $crate::drivers::impl_callback_host_client!(@methods proxy
            ($client, { $($special)* }));
    };
    (@methods $mode:ident ($($configuration:tt)*)) => {
        $crate::drivers::impl_callback_host_client!(@emit $mode ($($configuration)*);
            check_path_permission(request: $crate::host_api::PathPermissionQuery)
                -> $crate::host_api::PermissionQuery = HOST_PERMISSION_CHECK_PATH { "request": request };
            check_network_permission(target: String)
                -> $crate::host_api::PermissionQuery = HOST_PERMISSION_CHECK_NETWORK { "target": target };
            request_config_reload()
                -> $crate::host_api::HostConfigReloadRequestResponse = HOST_CONFIG_RELOAD_REQUEST {};
            config_reload_status(request: $crate::host_api::HostConfigReloadStatusRequest)
                -> $crate::host_api::HostConfigReloadStatusResponse = HOST_CONFIG_RELOAD_STATUS { "request": request };
            invoke_tool(tool: String, input: $crate::serde_json::Value)
                -> $crate::hooks::ToolInvokeOutput = HOST_TOOL_INVOKE { "tool": tool, "input": input };
            invoke_service(request: $crate::PluginServiceInvokeInput)
                -> $crate::PluginServiceInvokeOutput = HOST_SERVICE_INVOKE { "request": request };
            spawn_agent(request: $crate::host_api::AgentSpawnRequest)
                -> $crate::host_api::AgentHandle = HOST_AGENT_SPAWN { "request": request };
            message_agent(request: $crate::host_api::AgentMessageRequest)
                -> $crate::host_api::AgentMessageReceipt = HOST_AGENT_MESSAGE { "request": request };
            wait_agent(request: $crate::host_api::AgentWaitRequest)
                -> $crate::host_api::AgentWaitResponse = HOST_AGENT_WAIT { "request": request };
            followup_agent(request: $crate::host_api::AgentMessageRequest)
                -> $crate::host_api::AgentHandle = HOST_AGENT_FOLLOWUP { "request": request };
            cancel_agent(request: $crate::host_api::AgentTargetRequest)
                -> $crate::host_api::AgentMessageReceipt = HOST_AGENT_CANCEL { "request": request };
            image_execute(request: $crate::host_api::HostImageExecuteRequest)
                -> $crate::host_api::HostImageExecuteResponse = HOST_IMAGE_EXECUTE { "request": request };
        );
    };
    (@emit remote ($client:ty, $context:path, { $($special:item)* });
        $( $name:ident ( $( $argument:ident : $argument_type:ty ),* ) -> $result:ty
            = $rpc:ident { $( $field:literal : $value:expr ),* } ; )*
    ) => {
        #[async_trait::async_trait]
        impl $crate::HostClient for $client {
            $($special)*
            $(
                async fn $name(&self $(, $argument: $argument_type)*) -> $crate::Result<$result> {
                    self.call(
                        $crate::rpc::method::$rpc,
                        $context($crate::serde_json::json!({ $($field: $value),* })),
                    ).await
                }
            )*
        }
    };
    (@emit proxy ($client:ty, { $($special:item)* });
        $( $name:ident ( $( $argument:ident : $argument_type:ty ),* ) -> $result:ty
            = $rpc:ident { $( $field:literal : $value:expr ),* } ; )*
    ) => {
        #[async_trait::async_trait]
        impl $crate::HostClient for $client {
            $($special)*
            $(
                async fn $name(&self $(, $argument: $argument_type)*) -> $crate::Result<$result> {
                    // Resolve the destination per call and keep owner cancellation.
                    self.run(self.current().$name($($argument),*)).await
                }
            )*
        }
    };
}

#[cfg(any(feature = "stdio", feature = "http"))]
pub(crate) use impl_callback_host_client;
