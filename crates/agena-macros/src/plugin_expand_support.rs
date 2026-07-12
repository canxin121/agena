use quote::{format_ident, quote};
use syn::{ImplItem, ItemImpl, LitStr, Result};

use crate::macro_parse_support::doc_text;
use crate::plugin_hooks::{
    PluginHookKind, expand_plugin_layer_hook_methods, reject_duplicate_init_hooks,
};
use crate::plugin_impl_config::{parse_plugin_impl_config, plugin_self_type_label};
use crate::plugin_manifest::{expand_plugin_layer_export, expand_plugin_layer_manifest};
use crate::plugin_runtime::{
    expand_plugin_layer_command_invoke, expand_plugin_layer_init_method,
    expand_plugin_layer_permission_networks, expand_plugin_layer_permission_paths,
    expand_plugin_layer_tool_invoke, expand_plugin_layer_tool_stream,
};
use crate::{
    PluginGeneratedToolInput, build_tool_command_plan, command_generated_input_model,
    parse_plugin_inherent_method_attrs, plugin_impl_method_infos, reject_duplicate_command_plans,
    reject_duplicate_tool_plans,
};

pub(crate) fn expand_plugin_impl_attr(
    attr: proc_macro2::TokenStream,
    item: ItemImpl,
) -> Result<proc_macro2::TokenStream> {
    if item.trait_.is_some() {
        return Err(syn::Error::new_spanned(
            &item.self_ty,
            "#[agena_plugin(...)] only supports inherent impl blocks; write `#[async_trait] impl Plugin for Type` manually for dynamic plugins",
        ));
    }

    if attr.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.self_ty,
            "#[agena_plugin(...)] inherent impls require id/version/summary metadata",
        ));
    }

    expand_plugin_inherent_impl_attr(attr, item)
}

fn expand_plugin_inherent_impl_attr(
    attr: proc_macro2::TokenStream,
    mut item: ItemImpl,
) -> Result<proc_macro2::TokenStream> {
    let config = parse_plugin_impl_config(attr)?;
    let docs = doc_text(&item.attrs);
    let self_ty = item.self_ty.as_ref().clone();
    let self_label = plugin_self_type_label(&self_ty);
    let method_infos = plugin_impl_method_infos(&item);
    let mut tool_plans = Vec::new();
    let mut hook_bindings = Vec::new();
    let mut command_plans = Vec::new();

    for impl_item in &mut item.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };
        let attrs = parse_plugin_inherent_method_attrs(method, &self_label, &method_infos)?;
        tool_plans.extend(attrs.tools);
        hook_bindings.extend(attrs.hooks);
        command_plans.extend(attrs.commands);
    }

    if (!tool_plans.is_empty() || !command_plans.is_empty()) && !item.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.generics,
            "method-level #[tool(...)]/#[command(...)] generation does not support generic plugin impls yet; use a non-generic plugin wrapper type",
        ));
    }
    command_plans.extend(
        tool_plans
            .iter()
            .filter_map(build_tool_command_plan)
            .collect::<Result<Vec<_>>>()?,
    );
    reject_duplicate_tool_plans(&tool_plans)?;
    reject_duplicate_init_hooks(&hook_bindings)?;
    reject_duplicate_command_plans(&command_plans)?;
    let generated_input_items = tool_plans
        .iter()
        .map(|tool| expand_plugin_generated_input(&tool.input_model))
        .chain(
            command_plans
                .iter()
                .filter_map(command_generated_input_model)
                .map(expand_plugin_generated_input),
        )
        .collect::<Result<Vec<_>>>()?;

    let manifest_method = expand_plugin_layer_manifest(
        &config,
        &self_ty,
        item.generics.params.is_empty(),
        docs.as_deref(),
        &tool_plans,
        &hook_bindings,
        &command_plans,
    )?;
    let tool_invoke_method = (!tool_plans.is_empty())
        .then(|| expand_plugin_layer_tool_invoke(&self_ty, &tool_plans))
        .transpose()?;
    let stream_method = tool_plans
        .iter()
        .any(|tool| tool.stream.is_some())
        .then(|| expand_plugin_layer_tool_stream(&self_ty, &tool_plans))
        .transpose()?;
    let permission_paths_method = tool_plans
        .iter()
        .any(|tool| tool.permissions.has_path_permissions())
        .then(|| expand_plugin_layer_permission_paths(&self_ty, &tool_plans))
        .transpose()?;
    let permission_networks_method = tool_plans
        .iter()
        .any(|tool| tool.permissions.has_network_permissions())
        .then(|| expand_plugin_layer_permission_networks(&self_ty, &tool_plans))
        .transpose()?;
    let command_invoke_method = (!command_plans.is_empty())
        .then(|| expand_plugin_layer_command_invoke(&self_ty, &command_plans))
        .transpose()?;
    let init_binding = hook_bindings
        .iter()
        .find(|binding| binding.hook == PluginHookKind::Init);
    let init_method =
        (config.config_field.is_some() || config.config_store || init_binding.is_some())
            .then(|| expand_plugin_layer_init_method(&config, &self_ty, init_binding))
            .transpose()?;
    let hook_methods = expand_plugin_layer_hook_methods(&self_ty, &hook_bindings)?;
    let generics = &item.generics;
    let export = expand_plugin_layer_export(&config, &self_ty, generics)?;
    let (impl_generics, _ty_generics, where_clause) = generics.split_for_impl();
    Ok(quote! {
        #item

        #(#generated_input_items)*

        #[::agena_plugin_sdk::async_trait]
        impl #impl_generics ::agena_plugin_sdk::Plugin for #self_ty #where_clause {
            #manifest_method
            #tool_invoke_method
            #stream_method
            #permission_paths_method
            #permission_networks_method
            #command_invoke_method
            #init_method
            #(#hook_methods)*
        }

        #export
    })
}

fn expand_plugin_generated_input(
    generated: &PluginGeneratedToolInput,
) -> Result<proc_macro2::TokenStream> {
    let Some(input_ident) = generated.input_ident.as_ref() else {
        return Ok(quote! {});
    };
    let fields = generated.input_fields.iter().map(|field| {
        let ident = &field.ident;
        let wire_name = &field.wire_name;
        let flatten_attrs = field
            .flatten_shape
            .then(|| quote! { #[serde(flatten)] #[schemars(flatten)] });
        let rename_attr = (!field.flatten_shape && field.ident != field.wire_name.value())
            .then(|| quote! { #[serde(rename = #wire_name)] });
        let alias_attrs = if field.flatten_shape {
            Vec::new()
        } else {
            field
                .aliases
                .iter()
                .map(|alias| quote! { #[serde(alias = #alias)] })
                .collect::<Vec<_>>()
        };
        let ty = &field.ty;
        let default_attr = if field.flatten_shape {
            None
        } else if field.default_expr.is_some() {
            let helper = format_ident!("{}_default_{}", input_ident, ident);
            let helper_name = LitStr::new(&helper.to_string(), helper.span());
            Some(quote! { #[serde(default = #helper_name)] })
        } else {
            field.default.then(|| quote! { #[serde(default)] })
        };
        quote! {
            #flatten_attrs
            #default_attr
            #rename_attr
            #(#alias_attrs)*
            #ident: #ty
        }
    });
    let default_helpers = generated
        .input_fields
        .iter()
        .filter_map(|field| {
            let expr = field.default_expr.as_ref()?;
            let helper = format_ident!("{}_default_{}", input_ident, field.ident);
            let ty = &field.ty;
            Some(quote! {
                #[allow(non_snake_case)]
                fn #helper() -> #ty {
                    #expr
                }
            })
        })
        .collect::<Vec<_>>();
    let docs_attr = generated
        .docs
        .as_ref()
        .map(|docs| quote! { #[doc = #docs] })
        .unwrap_or_default();

    Ok(quote! {
        #[allow(non_camel_case_types)]
        #docs_attr
        #[derive(
            ::agena_plugin_sdk::serde::Serialize,
            ::agena_plugin_sdk::serde::Deserialize,
            ::agena_plugin_sdk::JsonSchema
        )]
        #[serde(deny_unknown_fields)]
        struct #input_ident {
            #(#fields),*
        }

        #(#default_helpers)*
    })
}
