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

// The HostClient trait remains the public contract. These private declarations
// share mechanical forwarding while keeping transport capability groups explicit.
// HTTP uses only the common group; stdio and the replaceable proxy also use the
// extended group. Admission, permission waits and scoped authority stay local.
#[cfg(any(feature = "stdio", feature = "http"))]
macro_rules! impl_callback_host_client {
    ($client:ty, $context:path, { $($special:item)* }) => {
        $crate::drivers::impl_callback_host_client!(@methods remote
            ($client, $context, { $($special)* }));
    };
    (@stdio $client:ty, $context:path, { $($special:item)* }) => {
        $crate::drivers::impl_callback_host_client!(@methods stdio
            ($client, $context, { $($special)* }));
    };
    (@proxy $client:ty, { $($special:item)* }) => {
        $crate::drivers::impl_callback_host_client!(@methods proxy
            ($client, { $($special)* }));
    };
    (@methods $mode:ident ($($configuration:tt)*)) => {
        $crate::drivers::impl_callback_host_client!(@select $mode ($($configuration)*);
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
            companion_context() -> $crate::serde_json::Value = HOST_COMPANION_CONTEXT {};
            companion_operation(request: $crate::host_api::CompanionRequest)
                -> $crate::serde_json::Value = HOST_COMPANION_OPERATION { "request": request };
            manage_integrations(request: $crate::host_api::IntegrationRequest)
                -> $crate::serde_json::Value = HOST_MANAGE_INTEGRATIONS { "request": request };
            bot_context() -> $crate::serde_json::Value = HOST_BOT_CONTEXT {};
            bot_operation(request: $crate::host_api::BotRequest)
                -> $crate::serde_json::Value = HOST_BOT_OPERATION { "request": request };
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
            notify(request: $crate::host_api::PluginNotifyRequest)
                -> () = HOST_NOTIFY { "request": request } [discard_response];
        );
    };
    (@select remote ($($configuration:tt)*); $($common:tt)*) => {
        $crate::drivers::impl_callback_host_client!(@emit remote
            ($($configuration)*); $($common)*);
    };
    (@select stdio ($($configuration:tt)*); $($common:tt)*) => {
        $crate::drivers::impl_callback_host_client!(@extended remote
            ($($configuration)*); $($common)*);
    };
    (@select proxy ($($configuration:tt)*); $($common:tt)*) => {
        $crate::drivers::impl_callback_host_client!(@extended proxy
            ($($configuration)*); $($common)*);
    };
    (@extended $mode:ident ($($configuration:tt)*); $($common:tt)*) => {
        $crate::drivers::impl_callback_host_client!(@emit $mode ($($configuration)*);
            $($common)*
            ask_user(request: $crate::host_api::AskUserRequest)
                -> $crate::host_api::AskUserResponse = HOST_ASK_USER { "request": request };
            list_tools()
                -> Vec<$crate::host_api::ToolDescriptor> = HOST_TOOL_LIST {  };
            get_context_status(request: $crate::host_api::HostContextStatusRequest)
                -> $crate::host_api::HostContextStatusResponse = HOST_CONTEXT_STATUS { "request": request };
            set_session_model(request: $crate::host_api::HostSetSessionModelRequest)
                -> $crate::host_api::HostSetSessionModelResponse = HOST_SESSION_SET_MODEL { "request": request };
            session_query(request: $crate::host_api::HostSessionQueryRequest)
                -> $crate::host_api::HostSessionQueryResponse = HOST_SESSION_QUERY { "request": request };
            session_command(request: $crate::host_api::HostSessionCommandRequest)
                -> $crate::host_api::HostSessionCommandResponse = HOST_SESSION_COMMAND { "request": request };
            session_stream(request: $crate::host_api::HostSessionStreamRequest)
                -> $crate::host_api::EventSubscription = HOST_SESSION_STREAM { "request": request };
            workspace_query(request: $crate::host_api::HostWorkspaceQueryRequest)
                -> $crate::host_api::HostWorkspaceQueryResponse = HOST_WORKSPACE_QUERY { "request": request };
            monitor_start(request: $crate::host_api::MonitorStartRequest)
                -> $crate::host_api::MonitorHandle = HOST_MONITOR_START { "request": request };
            monitor_list()
                -> Vec<$crate::host_api::MonitorHandle> = HOST_MONITOR_LIST {  };
            monitor_read(request: $crate::host_api::MonitorReadRequest)
                -> $crate::host_api::MonitorReadResponse = HOST_MONITOR_READ { "request": request };
            monitor_stop(request: $crate::host_api::MonitorStopRequest)
                -> $crate::host_api::MonitorHandle = HOST_MONITOR_STOP { "request": request };
            register_tool(request: $crate::host_api::HostToolRegisterRequest)
                -> $crate::host_api::HostToolMutationResponse = HOST_TOOL_REGISTRY_REGISTER { "request": request };
            update_tool(request: $crate::host_api::HostToolUpdateRequest)
                -> $crate::host_api::HostToolMutationResponse = HOST_TOOL_REGISTRY_UPDATE { "request": request };
            remove_tool(request: $crate::host_api::HostToolRemoveRequest)
                -> $crate::host_api::HostToolMutationResponse = HOST_TOOL_REGISTRY_REMOVE { "request": request };
            list_registered_tools()
                -> $crate::host_api::HostRegisteredToolListResponse = HOST_TOOL_REGISTRY_LIST {  };
            storage_get(request: $crate::host_api::HostStorageGetRequest)
                -> $crate::host_api::HostStorageGetResponse = HOST_STORAGE_GET { "request": request };
            storage_set(request: $crate::host_api::HostStorageSetRequest)
                -> () = HOST_STORAGE_SET { "request": request } [discard_response];
            storage_delete(request: $crate::host_api::HostStorageDeleteRequest)
                -> () = HOST_STORAGE_DELETE { "request": request } [discard_response];
            storage_list(request: $crate::host_api::HostStorageListRequest)
                -> $crate::host_api::HostStorageListResponse = HOST_STORAGE_LIST { "request": request };
            secret_get(request: $crate::host_api::HostSecretGetRequest)
                -> $crate::host_api::HostSecretGetResponse = HOST_SECRET_GET { "request": request };
            secret_set(request: $crate::host_api::HostSecretSetRequest)
                -> () = HOST_SECRET_SET { "request": request } [discard_response];
            secret_delete(request: $crate::host_api::HostSecretDeleteRequest)
                -> () = HOST_SECRET_DELETE { "request": request } [discard_response];
            secret_list()
                -> $crate::host_api::HostSecretListResponse = HOST_SECRET_LIST {  };
            plugin_status_list()
                -> $crate::host_api::HostPluginStatusListResponse = HOST_PLUGIN_STATUS_LIST {  };
            plugin_status_get(request: $crate::host_api::HostPluginStatusGetRequest)
                -> $crate::host_api::HostPluginStatusGetResponse = HOST_PLUGIN_STATUS_GET { "request": request };
            lsp_list_servers()
                -> $crate::host_api::HostLspListServersResponse = HOST_LSP_LIST_SERVERS {  };
            lsp_list_diagnostics(request: $crate::host_api::HostLspListDiagnosticsRequest)
                -> $crate::host_api::HostLspListDiagnosticsResponse = HOST_LSP_LIST_DIAGNOSTICS { "request": request };
            scheduler_list()
                -> $crate::host_api::HostSchedulerListResponse = HOST_SCHEDULER_LIST {  };
            scheduler_create(request: $crate::host_api::HostSchedulerCreateRequest)
                -> $crate::host_api::HostSchedulerCreateResponse = HOST_SCHEDULER_CREATE { "request": request };
            scheduler_delete(request: $crate::host_api::HostSchedulerDeleteRequest)
                -> $crate::host_api::HostSchedulerDeleteResponse = HOST_SCHEDULER_DELETE { "request": request };
            hook_list()
                -> $crate::host_api::HostHookListResponse = HOST_HOOK_LIST {  };
            mcp_list_servers()
                -> $crate::host_api::HostMcpListServersResponse = HOST_MCP_LIST_SERVERS {  };
            mcp_add_server(request: $crate::host_api::HostMcpAddServerRequest)
                -> () = HOST_MCP_ADD_SERVER { "request": request } [discard_response];
            mcp_remove_server(request: $crate::host_api::HostMcpRemoveServerRequest)
                -> $crate::host_api::HostMcpRemoveServerResponse = HOST_MCP_REMOVE_SERVER { "request": request };
            display_contribute(request: $crate::host_api::HostDisplayContributeRequest)
                -> () = HOST_UI_DISPLAY_CONTRIBUTE { "request": request } [discard_response];
            display_remove(request: $crate::host_api::HostDisplayRemoveRequest)
                -> $crate::host_api::HostDisplayRemoveResponse = HOST_UI_DISPLAY_REMOVE { "request": request };
            ui_theme_register(request: $crate::host_api::HostThemeRegisterRequest)
                -> () = HOST_UI_THEME_REGISTER { "request": request } [discard_response];
            ui_theme_list()
                -> $crate::host_api::HostThemeListResponse = HOST_UI_THEME_LIST {  };
            ui_theme_remove(request: $crate::host_api::HostThemeRemoveRequest)
                -> $crate::host_api::HostThemeRemoveResponse = HOST_UI_THEME_REMOVE { "request": request };
        );
    };
    (@emit remote ($client:ty, $context:path, { $($special:item)* });
        $( $name:ident ( $( $argument:ident : $argument_type:ty ),* ) -> $result:ty
            = $rpc:ident { $( $field:literal : $value:expr ),* } $( [$response:ident] )? ; )*
    ) => {
        #[async_trait::async_trait]
        impl $crate::HostClient for $client {
            $($special)*
            $(
                async fn $name(&self $(, $argument: $argument_type)*) -> $crate::Result<$result> {
                    $crate::drivers::impl_callback_host_client!(@response
                        self.call(
                            $crate::rpc::method::$rpc,
                            $context($crate::serde_json::json!({ $($field: $value),* })),
                        ).await; $($response)?
                    )
                }
            )*
        }
    };
    (@emit proxy ($client:ty, { $($special:item)* });
        $( $name:ident ( $( $argument:ident : $argument_type:ty ),* ) -> $result:ty
            = $rpc:ident { $( $field:literal : $value:expr ),* } $( [$response:ident] )? ; )*
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
    // Some stdio callbacks acknowledge with an arbitrary JSON payload. Keep
    // accepting those payloads instead of changing them to null-only unit decoding.
    (@response $result:expr; discard_response) => {
        $result.map(|_: $crate::serde_json::Value| ())
    };
    (@response $result:expr;) => { $result };
}

#[cfg(any(feature = "stdio", feature = "http"))]
pub(crate) use impl_callback_host_client;
