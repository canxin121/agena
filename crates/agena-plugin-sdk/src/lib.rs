//! Agena Plugin SDK — write a plugin once, ship it as in-process / cdylib / stdio / HTTP.
//!
//! Implement [`Plugin`] for your type, fill in the hooks you care about (every method
//! has a default no-op), and pick a transport with one of the `export_*!` macros.

extern crate self as agena_plugin_sdk;

pub mod attachment;
pub mod error;
pub mod hooks;
pub mod host_api;
#[doc(hidden)]
pub mod macro_support;
pub mod manifest;
pub mod plugin;
pub mod prelude;
pub mod rpc;

#[cfg(feature = "cdylib")]
pub mod cdylib_abi;

#[cfg(any(feature = "cdylib", feature = "stdio", feature = "http"))]
pub mod drivers;

pub use agena_macros::{
    StaticToolSurface, ToolArgs, ToolCommand, ToolInputShape, ToolSubcommands, ToolSuite, plugin,
    plugin_init_method, plugin_manifest_method, plugin_permission_networks_method,
    plugin_permission_paths_method, plugin_tool_invoke_method, plugin_tool_invoke_stream_method,
};
pub use async_trait::async_trait;
pub use attachment::{AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource};
pub use error::{PluginError, PluginErrorCode, Result};
pub use hooks::*;
pub use host_api::{
    HostClient, HostNetworkPermissionCheckRequest, HostPathPermissionCheckRequest,
    HostPermissionCheckResponse, NoopHostClient,
};
pub use macro_support::{schema_example_texts, schema_usage_text};
pub use manifest::{
    HookSubscription, HostCapability, InputNetworkSpec, InputPathSpec, NetworkAccessSpec,
    PathAccessSpec, PathKind, PluginManifest, PluginStudioCommand, PluginStudioControl,
    PluginStudioControlOption, PluginStudioUiContributions, PluginStudioView, PluginToolDecl,
    PluginTuiContentBlock, PluginTuiStatuslineSegment, PluginTuiUiContributions, PluginUiAction,
    PluginUiContributions, PluginUiThemePalette, ToolDescriptionMode, ToolDisplayPreset,
    ToolInputShape, ToolStreamingMode, ToolSuiteSurface, ToolSurface, ToolTag, TransportKind,
    UiTextDisplayMode, normalize_tool_tag_name,
};
pub use plugin::{InitContext, InitOutcome, Plugin, ToolStreamSink};
pub use schemars::JsonSchema;

#[macro_export]
macro_rules! plugin_impl {
    (for $plugin:ty { $($methods:tt)* }) => {
        #[::agena_plugin_sdk::async_trait]
        impl ::agena_plugin_sdk::Plugin for $plugin {
            ::agena_plugin_sdk::plugin_methods! { $($methods)* }
        }
    };
}

#[macro_export]
macro_rules! plugin_method {
    ($($method:tt)+) => {
        $crate::plugin_methods! { $($method)+ }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! plugin_manifest_config_schema {
    (type = $config_schema_ty:ty) => {
        $crate::macro_support::json_schema_for::<$config_schema_ty>()
    };
    (type = $config_schema_ty:ty, default) => {
        $crate::macro_support::json_schema_for_with_default(
            <$config_schema_ty as ::core::default::Default>::default(),
        )
    };
    (type = $config_schema_ty:ty, default = $config_schema_default:expr) => {
        $crate::macro_support::json_schema_for_with_default($config_schema_default)
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! plugin_manifest_display {
    ($builder:expr, brief) => {
        $builder.display(::agena_plugin_sdk::ToolDisplayPreset::Compact)
    };
    ($builder:expr, compact) => {
        $builder.display(::agena_plugin_sdk::ToolDisplayPreset::Compact)
    };
    ($builder:expr, brief_detailed) => {
        $builder.display(::agena_plugin_sdk::ToolDisplayPreset::BriefDetailed)
    };
    ($builder:expr, detailed) => {
        $builder.display(::agena_plugin_sdk::ToolDisplayPreset::Detailed)
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! plugin_manifest_ui_display {
    ($builder:expr, brief) => {
        $builder.ui_display_mode(::agena_plugin_sdk::UiTextDisplayMode::Summary)
    };
    ($builder:expr, summary) => {
        $builder.ui_display_mode(::agena_plugin_sdk::UiTextDisplayMode::Summary)
    };
    ($builder:expr, detailed) => {
        $builder.ui_display_mode(::agena_plugin_sdk::UiTextDisplayMode::Detailed)
    };
}

#[macro_export]
macro_rules! tool_surface_dispatch {
    ($tool_name:expr, $input:expr, $surface:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        let __tool_name = $tool_name;
        let __input = $input;
        if __tool_name != <$surface as $crate::ToolSurface>::tool_name() {
            Err($crate::PluginError::invalid_params(format!(
                "unknown {} tool '{}'",
                <$surface as $crate::ToolSurface>::tool_name(),
                __tool_name
            )))
        } else {
            match <$surface as $crate::ToolSurface>::parse_input(__input)? {
                $($pattern => $body,)+
            }
        }
    }};
}

#[macro_export]
macro_rules! tool_suite_dispatch {
    ($tool_name:expr, $input:expr, $suite:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        let __tool_name = $tool_name;
        let __input = $input;
        match <$suite as $crate::ToolSuiteSurface>::parse_tool(__tool_name, __input)? {
            $($pattern => $body,)+
        }
    }};
}

#[macro_export]
macro_rules! tool_shape_dispatch {
    ($input:expr, $shape:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        let __input = $input;
        match <$shape as $crate::ToolInputShape>::parse_input(__input)? {
            $($pattern => $body,)+
        }
    }};
}

#[macro_export]
macro_rules! plugin_manifest {
    (
        id = $id:expr,
        version = $version:expr,
        description = $description:expr,
        hooks = $hooks:expr,
        $(config_schema_type = $config_schema_ty:ty $(, config_schema_default = $config_schema_default:ident)? ,)?
        $(config_schema = $config_schema:expr,)?
        $(display = $display:ident,)?
        $(display_if_some = $display_if_some:expr,)?
        $(ui_display = $ui_display:ident,)?
        $(ui_display_if_some = $ui_display_if_some:expr,)?
        $(about = $about:expr,)?
        $(about_if_some = $about_if_some:expr,)?
        $(long_about = $long_about:expr,)?
        $(long_about_if_some = $long_about_if_some:expr,)?
        $(long_help = $long_help:expr,)?
        $(long_help_if_some = $long_help_if_some:expr,)?
        $(after_help = $after_help:expr,)?
        $(after_help_if_some = $after_help_if_some:expr,)?
        $(after_long_help = $after_long_help:expr,)?
        $(after_long_help_if_some = $after_long_help_if_some:expr,)?
        $(before_help = $before_help:expr,)?
        $(before_help_if_some = $before_help_if_some:expr,)?
        $(before_long_help = $before_long_help:expr,)?
        $(before_long_help_if_some = $before_long_help_if_some:expr,)?
        $(summary = $summary:expr,)?
        $(summary_if_some = $summary_if_some:expr,)?
        $(help = $help:expr,)?
        $(help_if_some = $help_if_some:expr,)?
        $(tool_description_mode = $tool_description_mode:expr,)?
        $(tool_description_mode_if_some = $tool_description_mode_if_some:expr,)?
        $(ui_display_mode = $ui_display_mode:expr,)?
        $(ui_display_mode_if_some = $ui_display_mode_if_some:expr,)?
        $(tool_surface = $tool_surface:ty,)?
        $(tool_suite = $tool_suite:ty,)?
        $(tools = $tools:expr,)?
        $(commands = $commands:expr,)?
        $(plugin_capabilities = $plugin_capabilities:expr,)?
        $(ui = $ui:expr,)?
    ) => {{
        let mut builder = $crate::PluginManifest::builder($id, $version)
            .description($description)
            .hooks($hooks)
            .config_schema($crate::macro_support::empty_config_schema());
        $(
            builder = builder.config_schema($crate::plugin_manifest_config_schema!(
                type = $config_schema_ty
                $(, $config_schema_default)?
            ));
        )?
        $(
            builder = builder.config_schema($config_schema);
        )?
        $(
            builder = $crate::plugin_manifest_display!(builder, $display);
        )?
        $(
            if let Some(value) = $display_if_some {
                builder = builder.display(value);
            }
        )?
        $(
            builder = $crate::plugin_manifest_ui_display!(builder, $ui_display);
        )?
        $(
            if let Some(value) = $ui_display_if_some {
                builder = builder.ui_display_mode(value);
            }
        )?
        $(
            builder = builder.about($about);
        )?
        $(
            if let Some(value) = $about_if_some {
                builder = builder.about(value);
            }
        )?
        $(
            builder = builder.long_about($long_about);
        )?
        $(
            if let Some(value) = $long_about_if_some {
                builder = builder.long_about(value);
            }
        )?
        $(
            builder = builder.long_help($long_help);
        )?
        $(
            if let Some(value) = $long_help_if_some {
                builder = builder.long_help(value);
            }
        )?
        $(
            builder = builder.after_help($after_help);
        )?
        $(
            if let Some(value) = $after_help_if_some {
                builder = builder.after_help(value);
            }
        )?
        $(
            builder = builder.after_long_help($after_long_help);
        )?
        $(
            if let Some(value) = $after_long_help_if_some {
                builder = builder.after_long_help(value);
            }
        )?
        $(
            builder = builder.before_help($before_help);
        )?
        $(
            if let Some(value) = $before_help_if_some {
                builder = builder.before_help(value);
            }
        )?
        $(
            builder = builder.before_long_help($before_long_help);
        )?
        $(
            if let Some(value) = $before_long_help_if_some {
                builder = builder.before_long_help(value);
            }
        )?
        $(
            builder = builder.summary($summary);
        )?
        $(
            if let Some(value) = $summary_if_some {
                builder = builder.summary(value);
            }
        )?
        $(
            builder = builder.help($help);
        )?
        $(
            if let Some(value) = $help_if_some {
                builder = builder.help(value);
            }
        )?
        $(
            builder = builder.tool_description_mode($tool_description_mode);
        )?
        $(
            if let Some(value) = $tool_description_mode_if_some {
                builder = builder.tool_description_mode(value);
            }
        )?
        $(
            builder = builder.ui_display_mode($ui_display_mode);
        )?
        $(
            if let Some(value) = $ui_display_mode_if_some {
                builder = builder.ui_display_mode(value);
            }
        )?
        $(
            builder = builder.tool_surface::<$tool_surface>();
        )?
        $(
            builder = builder.tool_suite::<$tool_suite>();
        )?
        $(
            builder = builder.tools($tools);
        )?
        $(
            builder = builder.commands($commands);
        )?
        $(
            builder = builder.plugin_capabilities($plugin_capabilities);
        )?
        $(
            builder = builder.ui($ui);
        )?
        builder.build()
    }};
}

#[macro_export]
macro_rules! plugin_init {
    (
        manifest = $manifest:expr
        $(, default_config = {
            field = $default_config_field:expr,
            ty = $default_config_ty:ty,
            input = $default_config_input:expr,
            invalid = $default_config_invalid:expr,
            already = $default_config_already:expr
        })?
        $(, config = {
            field = $config_field:expr,
            value = $config_value:expr,
            already = $config_already:expr
        })?
        $(, store = {
            field = $store_field:expr,
            value = $store_value:expr,
            already = $store_already:expr
        })*
        $(, workspace_root = {
            field = $workspace_root_field:expr,
            value = $workspace_root_value:expr,
            already = $workspace_root_already:expr
        })?
        $(, host_cell = {
            field = $host_field:expr,
            value = $host_value:expr,
            poisoned = $host_poisoned:expr
        })?
        $(, after = $after:block)?
        $(,)?
    ) => {{
        $(
            let __plugin_init_config: $default_config_ty =
                $crate::macro_support::parse_defaulted_config(
                    $default_config_input,
                    $default_config_invalid,
                )?;
            $crate::macro_support::store_once(
                &$default_config_field,
                __plugin_init_config,
                $default_config_already,
            )?;
        )?
        $(
            $crate::macro_support::store_once(
                &$config_field,
                $config_value,
                $config_already,
            )?;
        )?
        $(
            $crate::macro_support::store_once(
                &$store_field,
                $store_value,
                $store_already,
            )?;
        )*
        $(
            $crate::macro_support::store_once(
                &$workspace_root_field,
                $workspace_root_value,
                $workspace_root_already,
            )?;
        )?
        $(
            $crate::macro_support::store_rwlock_option(
                &$host_field,
                $host_value,
                $host_poisoned,
            )?;
        )?
        $(
            $after
        )?
        ::core::result::Result::<$crate::InitOutcome, $crate::PluginError>::Ok(
            $crate::InitOutcome::ack($manifest),
        )
    }};
}

#[macro_export]
macro_rules! plugin_methods {
    () => {};

    (manifest { $($manifest:tt)* }; $($rest:tt)*) => {
        $crate::plugin_methods! {
            manifest => $crate::plugin_manifest!($($manifest)*);
            $($rest)*
        }
    };

    (manifest($receiver:ident) { $($manifest:tt)* }; $($rest:tt)*) => {
        $crate::plugin_methods! {
            manifest($receiver) => {
                $crate::plugin_manifest!($($manifest)*)
            };
            $($rest)*
        }
    };

    (manifest($receiver:ident) => $body:block; $($rest:tt)*) => {
        fn manifest(&self) -> $crate::PluginManifest {
            let $receiver = self;
            $body
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (manifest => $manifest:expr; $($rest:tt)*) => {
        fn manifest(&self) -> $crate::PluginManifest {
            $manifest
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (init($receiver:ident, $ctx:pat, $host:pat) { manifest = $manifest:expr, $($init:tt)* }; $($rest:tt)*) => {
        $crate::plugin_methods! {
            init($receiver, $ctx, $host) => {
                $crate::plugin_init!(manifest = $manifest, $($init)*)
            };
            $($rest)*
        }
    };

    (init($receiver:ident, $ctx:pat, $host:pat) { $($init:tt)* }; $($rest:tt)*) => {
        $crate::plugin_methods! {
            init($receiver, $ctx, $host) => {
                $crate::plugin_init!(manifest = $receiver.manifest(), $($init)*)
            };
            $($rest)*
        }
    };

    (init($receiver:ident, $ctx:pat, $host:pat) => $body:block; $($rest:tt)*) => {
        fn init<'life0, 'async_trait>(
            &'life0 self,
            ctx: $crate::InitContext,
            host: ::std::sync::Arc<dyn $crate::HostClient>,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::InitOutcome>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $ctx = ctx;
                let $host = host;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn tool_invoke<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolInvokeOutput>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke => surface($surface:ty); $($rest:tt)*) => {
        fn tool_invoke<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolInvokeOutput>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move { $crate::plugin_tool_dispatch_surface!(self, input, $surface) })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke => surface($surface:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn tool_invoke<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolInvokeOutput>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let __input = $resolve(self, input)?;
                $crate::plugin_tool_dispatch_surface!(self, __input, $surface)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke => surface_with_context($surface:ty); $($rest:tt)*) => {
        fn tool_invoke<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolInvokeOutput>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_tool_dispatch_surface_with_context!(self, input, $surface)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke => surface_with_context($surface:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn tool_invoke<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolInvokeOutput>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let __input = $resolve(self, input)?;
                $crate::plugin_tool_dispatch_surface_with_context!(self, __input, $surface)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke => suite($suite:ty); $($rest:tt)*) => {
        fn tool_invoke<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolInvokeOutput>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move { $crate::plugin_tool_dispatch_suite!(self, input, $suite) })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke => suite($suite:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn tool_invoke<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolInvokeOutput>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let __input = $resolve(self, input)?;
                $crate::plugin_tool_dispatch_suite!(self, __input, $suite)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke => suite_with_context($suite:ty); $($rest:tt)*) => {
        fn tool_invoke<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolInvokeOutput>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_tool_dispatch_suite_with_context!(self, input, $suite)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke => suite_with_context($suite:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn tool_invoke<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolInvokeOutput>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let __input = $resolve(self, input)?;
                $crate::plugin_tool_dispatch_suite_with_context!(self, __input, $suite)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke => shape($shape:ty); $($rest:tt)*) => {
        fn tool_invoke<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolInvokeOutput>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move { $crate::plugin_tool_dispatch_shape!(self, input, $shape) })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke => shape($shape:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn tool_invoke<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolInvokeOutput>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let __input = $resolve(self, input)?;
                $crate::plugin_tool_dispatch_shape!(self, __input, $shape)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke => shape_with_context($shape:ty); $($rest:tt)*) => {
        fn tool_invoke<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolInvokeOutput>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_tool_dispatch_shape_with_context!(self, input, $shape)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke => shape_with_context($shape:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn tool_invoke<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolInvokeOutput>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let __input = $resolve(self, input)?;
                $crate::plugin_tool_dispatch_shape_with_context!(self, __input, $shape)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke_stream($receiver:ident, $input:pat, $sink:pat) => $body:block; $($rest:tt)*) => {
        fn tool_invoke_stream<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
            sink: $crate::ToolStreamSink,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolStreamEnd>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                let $sink = sink;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke_stream => surface($surface:ty); $($rest:tt)*) => {
        fn tool_invoke_stream<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
            sink: $crate::ToolStreamSink,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolStreamEnd>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_tool_dispatch_stream_surface!(self, input, sink, $surface)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke_stream => surface($surface:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn tool_invoke_stream<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
            sink: $crate::ToolStreamSink,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolStreamEnd>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let __input = $resolve(self, input)?;
                $crate::plugin_tool_dispatch_stream_surface!(self, __input, sink, $surface)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke_stream => surface_with_context($surface:ty); $($rest:tt)*) => {
        fn tool_invoke_stream<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
            sink: $crate::ToolStreamSink,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolStreamEnd>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_tool_dispatch_stream_surface_with_context!(self, input, sink, $surface)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke_stream => surface_with_context($surface:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn tool_invoke_stream<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
            sink: $crate::ToolStreamSink,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolStreamEnd>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let __input = $resolve(self, input)?;
                $crate::plugin_tool_dispatch_stream_surface_with_context!(
                    self,
                    __input,
                    sink,
                    $surface
                )
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke_stream => suite($suite:ty); $($rest:tt)*) => {
        fn tool_invoke_stream<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
            sink: $crate::ToolStreamSink,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolStreamEnd>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_tool_dispatch_stream_suite!(self, input, sink, $suite)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke_stream => suite($suite:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn tool_invoke_stream<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
            sink: $crate::ToolStreamSink,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolStreamEnd>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let __input = $resolve(self, input)?;
                $crate::plugin_tool_dispatch_stream_suite!(self, __input, sink, $suite)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke_stream => suite_with_context($suite:ty); $($rest:tt)*) => {
        fn tool_invoke_stream<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
            sink: $crate::ToolStreamSink,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolStreamEnd>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_tool_dispatch_stream_suite_with_context!(self, input, sink, $suite)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke_stream => suite_with_context($suite:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn tool_invoke_stream<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
            sink: $crate::ToolStreamSink,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolStreamEnd>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let __input = $resolve(self, input)?;
                $crate::plugin_tool_dispatch_stream_suite_with_context!(
                    self,
                    __input,
                    sink,
                    $suite
                )
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke_stream => shape($shape:ty); $($rest:tt)*) => {
        fn tool_invoke_stream<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
            sink: $crate::ToolStreamSink,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolStreamEnd>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_tool_dispatch_stream_shape!(self, input, sink, $shape)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke_stream => shape($shape:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn tool_invoke_stream<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
            sink: $crate::ToolStreamSink,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolStreamEnd>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let __input = $resolve(self, input)?;
                $crate::plugin_tool_dispatch_stream_shape!(self, __input, sink, $shape)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke_stream => shape_with_context($shape:ty); $($rest:tt)*) => {
        fn tool_invoke_stream<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
            sink: $crate::ToolStreamSink,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolStreamEnd>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_tool_dispatch_stream_shape_with_context!(self, input, sink, $shape)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_invoke_stream => shape_with_context($shape:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn tool_invoke_stream<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolInvokeInput,
            sink: $crate::ToolStreamSink,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<$crate::ToolStreamEnd>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let __input = $resolve(self, input)?;
                $crate::plugin_tool_dispatch_stream_shape_with_context!(
                    self,
                    __input,
                    sink,
                    $shape
                )
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_paths => surface($surface:ty); $($rest:tt)*) => {
        fn permission_paths<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            tool: &'life1 str,
            input: &'life2 $crate::serde_json::Value,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Vec<$crate::PathRequest>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_permission_dispatch_paths_surface!(self, tool, input, $surface)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_paths => surface($surface:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn permission_paths<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            tool: &'life1 str,
            input: &'life2 $crate::serde_json::Value,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Vec<$crate::PathRequest>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let (__tool, __input) = $resolve(self, tool, input)?;
                $crate::plugin_permission_dispatch_paths_surface!(
                    self,
                    __tool.as_str(),
                    &__input,
                    $surface
                )
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_paths => suite($suite:ty); $($rest:tt)*) => {
        fn permission_paths<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            tool: &'life1 str,
            input: &'life2 $crate::serde_json::Value,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Vec<$crate::PathRequest>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_permission_dispatch_paths_suite!(self, tool, input, $suite)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_paths => suite($suite:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn permission_paths<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            tool: &'life1 str,
            input: &'life2 $crate::serde_json::Value,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Vec<$crate::PathRequest>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let (__tool, __input) = $resolve(self, tool, input)?;
                $crate::plugin_permission_dispatch_paths_suite!(
                    self,
                    __tool.as_str(),
                    &__input,
                    $suite
                )
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_paths => shape($shape:ty); $($rest:tt)*) => {
        fn permission_paths<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            tool: &'life1 str,
            input: &'life2 $crate::serde_json::Value,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Vec<$crate::PathRequest>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_permission_dispatch_paths_shape!(self, tool, input, $shape)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_paths => shape($shape:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn permission_paths<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            tool: &'life1 str,
            input: &'life2 $crate::serde_json::Value,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Vec<$crate::PathRequest>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let (__tool, __input) = $resolve(self, tool, input)?;
                $crate::plugin_permission_dispatch_paths_shape!(
                    self,
                    __tool.as_str(),
                    &__input,
                    $shape
                )
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_paths($receiver:ident, $tool:pat, $input:pat) => $body:block; $($rest:tt)*) => {
        fn permission_paths<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            tool: &'life1 str,
            input: &'life2 $crate::serde_json::Value,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Vec<$crate::PathRequest>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $tool = tool;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_paths($receiver:ident) => surface($surface:ty, { $($pattern:pat => $body:expr),+ $(,)? }); $($rest:tt)*) => {
        $crate::plugin_methods! {
            permission_paths($receiver, tool, input) => {
                $crate::plugin_permission_paths_surface!(tool, input, $surface, {
                    $($pattern => $body,)+
                })
            };
            $($rest)*
        }
    };

    (permission_paths($receiver:ident) => suite($suite:ty, { $($pattern:pat => $body:expr),+ $(,)? }); $($rest:tt)*) => {
        $crate::plugin_methods! {
            permission_paths($receiver, tool, input) => {
                $crate::plugin_permission_paths_suite!(tool, input, $suite, {
                    $($pattern => $body,)+
                })
            };
            $($rest)*
        }
    };

    (permission_paths($receiver:ident) => shape($shape:ty, { $($pattern:pat => $body:expr),+ $(,)? }); $($rest:tt)*) => {
        $crate::plugin_methods! {
            permission_paths($receiver, _tool, input) => {
                $crate::plugin_permission_paths_shape!(input, $shape, {
                    $($pattern => $body,)+
                })
            };
            $($rest)*
        }
    };

    (permission_networks => surface($surface:ty); $($rest:tt)*) => {
        fn permission_networks<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            tool: &'life1 str,
            input: &'life2 $crate::serde_json::Value,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Vec<$crate::NetworkRequest>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_permission_dispatch_networks_surface!(self, tool, input, $surface)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_networks => surface($surface:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn permission_networks<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            tool: &'life1 str,
            input: &'life2 $crate::serde_json::Value,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Vec<$crate::NetworkRequest>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let (__tool, __input) = $resolve(self, tool, input)?;
                $crate::plugin_permission_dispatch_networks_surface!(
                    self,
                    __tool.as_str(),
                    &__input,
                    $surface
                )
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_networks => suite($suite:ty); $($rest:tt)*) => {
        fn permission_networks<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            tool: &'life1 str,
            input: &'life2 $crate::serde_json::Value,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Vec<$crate::NetworkRequest>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_permission_dispatch_networks_suite!(self, tool, input, $suite)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_networks => suite($suite:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn permission_networks<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            tool: &'life1 str,
            input: &'life2 $crate::serde_json::Value,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Vec<$crate::NetworkRequest>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let (__tool, __input) = $resolve(self, tool, input)?;
                $crate::plugin_permission_dispatch_networks_suite!(
                    self,
                    __tool.as_str(),
                    &__input,
                    $suite
                )
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_networks => shape($shape:ty); $($rest:tt)*) => {
        fn permission_networks<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            tool: &'life1 str,
            input: &'life2 $crate::serde_json::Value,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Vec<$crate::NetworkRequest>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                $crate::plugin_permission_dispatch_networks_shape!(self, tool, input, $shape)
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_networks => shape($shape:ty, resolve = $resolve:path); $($rest:tt)*) => {
        fn permission_networks<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            tool: &'life1 str,
            input: &'life2 $crate::serde_json::Value,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Vec<$crate::NetworkRequest>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let (__tool, __input) = $resolve(self, tool, input)?;
                $crate::plugin_permission_dispatch_networks_shape!(
                    self,
                    __tool.as_str(),
                    &__input,
                    $shape
                )
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_networks($receiver:ident, $tool:pat, $input:pat) => $body:block; $($rest:tt)*) => {
        fn permission_networks<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            tool: &'life1 str,
            input: &'life2 $crate::serde_json::Value,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Vec<$crate::NetworkRequest>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $tool = tool;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_networks($receiver:ident) => surface($surface:ty, { $($pattern:pat => $body:expr),+ $(,)? }); $($rest:tt)*) => {
        $crate::plugin_methods! {
            permission_networks($receiver, tool, input) => {
                $crate::plugin_permission_networks_surface!(tool, input, $surface, {
                    $($pattern => $body,)+
                })
            };
            $($rest)*
        }
    };

    (permission_networks($receiver:ident) => suite($suite:ty, { $($pattern:pat => $body:expr),+ $(,)? }); $($rest:tt)*) => {
        $crate::plugin_methods! {
            permission_networks($receiver, tool, input) => {
                $crate::plugin_permission_networks_suite!(tool, input, $suite, {
                    $($pattern => $body,)+
                })
            };
            $($rest)*
        }
    };

    (permission_networks($receiver:ident) => shape($shape:ty, { $($pattern:pat => $body:expr),+ $(,)? }); $($rest:tt)*) => {
        $crate::plugin_methods! {
            permission_networks($receiver, _tool, input) => {
                $crate::plugin_permission_networks_shape!(input, $shape, {
                    $($pattern => $body,)+
                })
            };
            $($rest)*
        }
    };

    (tool_execute_before($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn tool_execute_before<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolBeforeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::ToolBeforePatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_execute_after($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn tool_execute_after<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolAfterInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::ToolAfterPatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (command_execute_before($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn command_execute_before<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::CommandBeforeInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::CommandBeforeResponse>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (shell_env($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn shell_env<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ShellEnvInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::ShellEnvPatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (chat_params($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn chat_params<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ChatParamsInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::ChatParamsPatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (chat_message($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn chat_message<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ChatMessageInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::ChatMessagePatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (chat_headers($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn chat_headers<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ChatHeadersInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::ChatHeadersPatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (chat_system_transform($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn chat_system_transform<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ChatSystemTransformInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::ChatSystemTransformPatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (chat_messages_transform($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn chat_messages_transform<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ChatMessagesTransformInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::ChatMessagesTransformPatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (shutdown($receiver:ident) => $body:block; $($rest:tt)*) => {
        fn shutdown<'life0, 'async_trait>(
            &'life0 self,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<Output = $crate::Result<()>> + ::core::marker::Send + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (event($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn event<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::EventEnvelope,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<Output = $crate::Result<()>> + ::core::marker::Send + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (auth($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn auth<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::AuthInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::AuthOutput>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (pre_run($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn pre_run<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::PreRunInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<Output = $crate::Result<()>> + ::core::marker::Send + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (post_run($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn post_run<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::PostRunInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<Output = $crate::Result<()>> + ::core::marker::Send + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (session_start($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn session_start<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::SessionStartInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::SessionStartPatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (session_end($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn session_end<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::SessionEndInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<Output = $crate::Result<()>> + ::core::marker::Send + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (notification($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn notification<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::NotificationInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<Output = $crate::Result<()>> + ::core::marker::Send + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (provider_list($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn provider_list<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ProviderListInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::ProviderListPatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (user_prompt_submit($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn user_prompt_submit<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::UserPromptSubmitInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::UserPromptSubmitPatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_definition($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn tool_definition<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolDefinitionInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::ToolDefinitionPatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (tool_execute_failure($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn tool_execute_failure<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ToolFailureInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<Output = $crate::Result<()>> + ::core::marker::Send + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (command_execute_after($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn command_execute_after<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::CommandAfterInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::CommandAfterPatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (config_resolved($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn config_resolved<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::ConfigInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::ConfigPatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (permission_ask($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn permission_ask<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::PermissionAskInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::PermissionAskDecision>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };

    (agent_stop($receiver:ident, $input:pat) => $body:block; $($rest:tt)*) => {
        fn agent_stop<'life0, 'async_trait>(
            &'life0 self,
            input: $crate::AgentStopInput,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<
                        Output = $crate::Result<Option<$crate::AgentStopPatch>>,
                    > + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let $receiver = self;
                let $input = input;
                $body
            })
        }

        $crate::plugin_methods! { $($rest)* }
    };
}

#[macro_export]
macro_rules! plugin_tool_invoke_surface {
    ($invoke:ident, $surface:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        $crate::tool_surface_dispatch!(
            $invoke.tool_name.as_str(),
            $invoke.input,
            $surface,
            { $($pattern => $body,)+ }
        )
    }};
}

#[macro_export]
macro_rules! plugin_tool_invoke_suite {
    ($invoke:ident, $suite:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        $crate::tool_suite_dispatch!(
            $invoke.tool_name.as_str(),
            $invoke.input,
            $suite,
            { $($pattern => $body,)+ }
        )
    }};
}

#[macro_export]
macro_rules! plugin_tool_invoke_shape {
    ($invoke:ident, $shape:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        $crate::tool_shape_dispatch!($invoke.input, $shape, { $($pattern => $body,)+ })
    }};
}

#[macro_export]
macro_rules! plugin_tool_invoke_stream_surface {
    ($invoke:expr, $sink:expr, $surface:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        let __invoke = $invoke;
        let __tool_name = __invoke.tool_name.as_str();
        let __input = __invoke.input;
        let _ = &$sink;
        if __tool_name != <$surface as $crate::ToolSurface>::tool_name() {
            Err($crate::PluginError::invalid_params(format!(
                "unknown {} tool '{}'",
                <$surface as $crate::ToolSurface>::tool_name(),
                __tool_name
            )))
        } else {
            match <$surface as $crate::ToolSurface>::parse_input(__input)? {
                $($pattern => $body,)+
            }
        }
    }};
}

#[macro_export]
macro_rules! plugin_tool_invoke_stream_suite {
    ($invoke:expr, $sink:expr, $suite:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        let __invoke = $invoke;
        let __tool_name = __invoke.tool_name.as_str();
        let __input = __invoke.input;
        let _ = &$sink;
        match <$suite as $crate::ToolSuiteSurface>::parse_tool(__tool_name, __input)? {
            $($pattern => $body,)+
        }
    }};
}

#[macro_export]
macro_rules! plugin_tool_invoke_stream_shape {
    ($invoke:expr, $sink:expr, $shape:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        let __invoke = $invoke;
        let __input = __invoke.input;
        let _ = &$sink;
        match <$shape as $crate::ToolInputShape>::parse_input(__input)? {
            $($pattern => $body,)+
        }
    }};
}

#[macro_export]
macro_rules! plugin_tool_dispatch_surface {
    ($receiver:expr, $invoke:expr, $surface:ty $(,)?) => {{
        let __receiver = $receiver;
        let __invoke = $invoke;
        let __tool_name = __invoke.tool_name.as_str();
        if __tool_name != <$surface as $crate::ToolSurface>::tool_name() {
            Err($crate::PluginError::invalid_params(format!(
                "unknown {} tool '{}'",
                <$surface as $crate::ToolSurface>::tool_name(),
                __tool_name
            )))
        } else {
            let __parsed = <$surface as $crate::ToolSurface>::parse_input(__invoke.input)?;
            __parsed.dispatch_tool_invoke(__receiver).await
        }
    }};
}

#[macro_export]
macro_rules! plugin_tool_dispatch_surface_with_context {
    ($receiver:expr, $invoke:expr, $surface:ty $(,)?) => {{
        let __receiver = $receiver;
        let $crate::ToolInvokeInput {
            tool_name,
            session_id,
            call_id,
            workspace_root,
            input,
        } = $invoke;
        if tool_name.as_str() != <$surface as $crate::ToolSurface>::tool_name() {
            Err($crate::PluginError::invalid_params(format!(
                "unknown {} tool '{}'",
                <$surface as $crate::ToolSurface>::tool_name(),
                tool_name
            )))
        } else {
            let __context = $crate::ToolInvokeContext {
                tool_name: tool_name.as_str(),
                session_id,
                call_id,
                workspace_root: workspace_root.as_str(),
            };
            let __parsed = <$surface as $crate::ToolSurface>::parse_input(input)?;
            __parsed
                .dispatch_tool_invoke_with_context(__receiver, &__context)
                .await
        }
    }};
}

#[macro_export]
macro_rules! plugin_tool_dispatch_stream_surface {
    ($receiver:expr, $invoke:expr, $sink:expr, $surface:ty $(,)?) => {{
        let __receiver = $receiver;
        let __invoke = $invoke;
        let __sink = $sink;
        let __tool_name = __invoke.tool_name.as_str();
        if __tool_name != <$surface as $crate::ToolSurface>::tool_name() {
            Err($crate::PluginError::invalid_params(format!(
                "unknown {} tool '{}'",
                <$surface as $crate::ToolSurface>::tool_name(),
                __tool_name
            )))
        } else {
            let __parsed = <$surface as $crate::ToolSurface>::parse_input(__invoke.input)?;
            __parsed
                .dispatch_tool_invoke_stream(__receiver, __sink)
                .await
        }
    }};
}

#[macro_export]
macro_rules! plugin_tool_dispatch_stream_surface_with_context {
    ($receiver:expr, $invoke:expr, $sink:expr, $surface:ty $(,)?) => {{
        let __receiver = $receiver;
        let __sink = $sink;
        let $crate::ToolInvokeInput {
            tool_name,
            session_id,
            call_id,
            workspace_root,
            input,
        } = $invoke;
        if tool_name.as_str() != <$surface as $crate::ToolSurface>::tool_name() {
            Err($crate::PluginError::invalid_params(format!(
                "unknown {} tool '{}'",
                <$surface as $crate::ToolSurface>::tool_name(),
                tool_name
            )))
        } else {
            let __context = $crate::ToolInvokeContext {
                tool_name: tool_name.as_str(),
                session_id,
                call_id,
                workspace_root: workspace_root.as_str(),
            };
            let __parsed = <$surface as $crate::ToolSurface>::parse_input(input)?;
            __parsed
                .dispatch_tool_invoke_stream_with_context(__receiver, &__context, __sink)
                .await
        }
    }};
}

#[macro_export]
macro_rules! plugin_tool_dispatch_suite {
    ($receiver:expr, $invoke:expr, $suite:ty $(,)?) => {{
        let __receiver = $receiver;
        let __invoke = $invoke;
        let __parsed = <$suite as $crate::ToolSuiteSurface>::parse_tool(
            __invoke.tool_name.as_str(),
            __invoke.input,
        )?;
        __parsed.dispatch_tool_invoke(__receiver).await
    }};
}

#[macro_export]
macro_rules! plugin_tool_dispatch_suite_with_context {
    ($receiver:expr, $invoke:expr, $suite:ty $(,)?) => {{
        let __receiver = $receiver;
        let $crate::ToolInvokeInput {
            tool_name,
            session_id,
            call_id,
            workspace_root,
            input,
        } = $invoke;
        let __context = $crate::ToolInvokeContext {
            tool_name: tool_name.as_str(),
            session_id,
            call_id,
            workspace_root: workspace_root.as_str(),
        };
        let __parsed = <$suite as $crate::ToolSuiteSurface>::parse_tool(tool_name.as_str(), input)?;
        __parsed
            .dispatch_tool_invoke_with_context(__receiver, &__context)
            .await
    }};
}

#[macro_export]
macro_rules! plugin_tool_dispatch_stream_suite {
    ($receiver:expr, $invoke:expr, $sink:expr, $suite:ty $(,)?) => {{
        let __receiver = $receiver;
        let __sink = $sink;
        let __invoke = $invoke;
        let __parsed = <$suite as $crate::ToolSuiteSurface>::parse_tool(
            __invoke.tool_name.as_str(),
            __invoke.input,
        )?;
        __parsed
            .dispatch_tool_invoke_stream(__receiver, __sink)
            .await
    }};
}

#[macro_export]
macro_rules! plugin_tool_dispatch_stream_suite_with_context {
    ($receiver:expr, $invoke:expr, $sink:expr, $suite:ty $(,)?) => {{
        let __receiver = $receiver;
        let __sink = $sink;
        let $crate::ToolInvokeInput {
            tool_name,
            session_id,
            call_id,
            workspace_root,
            input,
        } = $invoke;
        let __context = $crate::ToolInvokeContext {
            tool_name: tool_name.as_str(),
            session_id,
            call_id,
            workspace_root: workspace_root.as_str(),
        };
        let __parsed = <$suite as $crate::ToolSuiteSurface>::parse_tool(tool_name.as_str(), input)?;
        __parsed
            .dispatch_tool_invoke_stream_with_context(__receiver, &__context, __sink)
            .await
    }};
}

#[macro_export]
macro_rules! plugin_tool_dispatch_shape {
    ($receiver:expr, $invoke:expr, $shape:ty $(,)?) => {{
        let __receiver = $receiver;
        let __invoke = $invoke;
        let __parsed = <$shape as $crate::ToolInputShape>::parse_input(__invoke.input)?;
        __parsed.dispatch_tool_invoke(__receiver).await
    }};
}

#[macro_export]
macro_rules! plugin_tool_dispatch_shape_with_context {
    ($receiver:expr, $invoke:expr, $shape:ty $(,)?) => {{
        let __receiver = $receiver;
        let $crate::ToolInvokeInput {
            tool_name,
            session_id,
            call_id,
            workspace_root,
            input,
        } = $invoke;
        let __context = $crate::ToolInvokeContext {
            tool_name: tool_name.as_str(),
            session_id,
            call_id,
            workspace_root: workspace_root.as_str(),
        };
        let __parsed = <$shape as $crate::ToolInputShape>::parse_input(input)?;
        __parsed
            .dispatch_tool_invoke_with_context(__receiver, &__context)
            .await
    }};
}

#[macro_export]
macro_rules! plugin_tool_dispatch_stream_shape {
    ($receiver:expr, $invoke:expr, $sink:expr, $shape:ty $(,)?) => {{
        let __receiver = $receiver;
        let __sink = $sink;
        let __invoke = $invoke;
        let __parsed = <$shape as $crate::ToolInputShape>::parse_input(__invoke.input)?;
        __parsed
            .dispatch_tool_invoke_stream(__receiver, __sink)
            .await
    }};
}

#[macro_export]
macro_rules! plugin_tool_dispatch_stream_shape_with_context {
    ($receiver:expr, $invoke:expr, $sink:expr, $shape:ty $(,)?) => {{
        let __receiver = $receiver;
        let __sink = $sink;
        let $crate::ToolInvokeInput {
            tool_name,
            session_id,
            call_id,
            workspace_root,
            input,
        } = $invoke;
        let __context = $crate::ToolInvokeContext {
            tool_name: tool_name.as_str(),
            session_id,
            call_id,
            workspace_root: workspace_root.as_str(),
        };
        let __parsed = <$shape as $crate::ToolInputShape>::parse_input(input)?;
        __parsed
            .dispatch_tool_invoke_stream_with_context(__receiver, &__context, __sink)
            .await
    }};
}

#[macro_export]
macro_rules! plugin_permission_paths_surface {
    ($tool:expr, $input:expr, $surface:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        let __tool = $tool;
        let __input = $input;
        if __tool != <$surface as $crate::ToolSurface>::tool_name() {
            Ok(Vec::new())
        } else {
            $crate::tool_surface_dispatch!(
                __tool,
                __input.clone(),
                $surface,
                { $($pattern => $body,)+ }
            )
        }
    }};
}

#[macro_export]
macro_rules! plugin_permission_dispatch_paths_surface {
    ($receiver:expr, $tool:expr, $input:expr, $surface:ty $(,)?) => {{
        let __receiver = $receiver;
        let __tool = $tool;
        let __input = $input;
        if __tool != <$surface as $crate::ToolSurface>::tool_name() {
            Ok(Vec::new())
        } else {
            let __parsed = <$surface as $crate::ToolSurface>::parse_input(__input.clone())?;
            __parsed.dispatch_permission_paths(__receiver).await
        }
    }};
}

#[macro_export]
macro_rules! plugin_permission_paths_shape {
    ($input:expr, $shape:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        let __input = $input;
        $crate::tool_shape_dispatch!(__input.clone(), $shape, { $($pattern => $body,)+ })
    }};
}

#[macro_export]
macro_rules! plugin_permission_dispatch_paths_shape {
    ($receiver:expr, $tool:expr, $input:expr, $shape:ty $(,)?) => {{
        let __receiver = $receiver;
        let _ = $tool;
        let __input = $input;
        let __parsed = <$shape as $crate::ToolInputShape>::parse_input(__input.clone())?;
        __parsed.dispatch_permission_paths(__receiver).await
    }};
}

#[macro_export]
macro_rules! plugin_permission_paths_suite {
    ($tool:expr, $input:expr, $suite:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        let __tool = $tool;
        let __input = $input;
        match <$suite as $crate::ToolSuiteSurface>::parse_tool(__tool, __input.clone()) {
            Ok(parsed) => match parsed {
                $($pattern => $body,)+
            },
            Err(_) => Ok(Vec::new()),
        }
    }};
}

#[macro_export]
macro_rules! plugin_permission_dispatch_paths_suite {
    ($receiver:expr, $tool:expr, $input:expr, $suite:ty $(,)?) => {{
        let __receiver = $receiver;
        let __tool = $tool;
        let __input = $input;
        match <$suite as $crate::ToolSuiteSurface>::parse_tool(__tool, __input.clone()) {
            Ok(__parsed) => __parsed.dispatch_permission_paths(__receiver).await,
            Err(_) => Ok(Vec::new()),
        }
    }};
}

#[macro_export]
macro_rules! plugin_permission_networks_surface {
    ($tool:expr, $input:expr, $surface:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        let __tool = $tool;
        let __input = $input;
        if __tool != <$surface as $crate::ToolSurface>::tool_name() {
            Ok(Vec::new())
        } else {
            $crate::tool_surface_dispatch!(
                __tool,
                __input.clone(),
                $surface,
                { $($pattern => $body,)+ }
            )
        }
    }};
}

#[macro_export]
macro_rules! plugin_permission_dispatch_networks_surface {
    ($receiver:expr, $tool:expr, $input:expr, $surface:ty $(,)?) => {{
        let __receiver = $receiver;
        let __tool = $tool;
        let __input = $input;
        if __tool != <$surface as $crate::ToolSurface>::tool_name() {
            Ok(Vec::new())
        } else {
            let __parsed = <$surface as $crate::ToolSurface>::parse_input(__input.clone())?;
            __parsed.dispatch_permission_networks(__receiver).await
        }
    }};
}

#[macro_export]
macro_rules! plugin_permission_networks_shape {
    ($input:expr, $shape:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        let __input = $input;
        $crate::tool_shape_dispatch!(__input.clone(), $shape, { $($pattern => $body,)+ })
    }};
}

#[macro_export]
macro_rules! plugin_permission_dispatch_networks_shape {
    ($receiver:expr, $tool:expr, $input:expr, $shape:ty $(,)?) => {{
        let __receiver = $receiver;
        let _ = $tool;
        let __input = $input;
        let __parsed = <$shape as $crate::ToolInputShape>::parse_input(__input.clone())?;
        __parsed.dispatch_permission_networks(__receiver).await
    }};
}

#[macro_export]
macro_rules! plugin_permission_networks_suite {
    ($tool:expr, $input:expr, $suite:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        let __tool = $tool;
        let __input = $input;
        match <$suite as $crate::ToolSuiteSurface>::parse_tool(__tool, __input.clone()) {
            Ok(parsed) => match parsed {
                $($pattern => $body,)+
            },
            Err(_) => Ok(Vec::new()),
        }
    }};
}

#[macro_export]
macro_rules! plugin_permission_dispatch_networks_suite {
    ($receiver:expr, $tool:expr, $input:expr, $suite:ty $(,)?) => {{
        let __receiver = $receiver;
        let __tool = $tool;
        let __input = $input;
        match <$suite as $crate::ToolSuiteSurface>::parse_tool(__tool, __input.clone()) {
            Ok(__parsed) => __parsed.dispatch_permission_networks(__receiver).await,
            Err(_) => Ok(Vec::new()),
        }
    }};
}

// Re-exports used by macros so plugin authors don't have to add deps directly.
#[doc(hidden)]
#[cfg(feature = "cdylib")]
pub use abi_stable as abi_stable_reexport;
#[doc(hidden)]
pub use serde_json;
