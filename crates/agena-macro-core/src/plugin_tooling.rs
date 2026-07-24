use quote::{format_ident, quote};
use syn::{Ident, LitStr, Result};

use crate::plugin_impl_config::sanitize_generated_ident_label;

use super::{
    PluginCommandHandlerPlan, PluginCommandInputPlan, PluginCommandPlan, PluginGeneratedToolInput,
    PluginInputNetworkSpec, PluginInputPathSpec, PluginNetworkSemantic, PluginPathPermissionKind,
    built_in_normalization_tokens, built_in_post_parse_normalization_tokens,
    built_in_validation_tokens, doc_summary, expand_flatten_shape_schema_normalize_expr,
    expand_generated_input_post_parse_tokens, expand_input_alias_normalize_tokens,
    expand_nested_shape_network_specs_expr, expand_nested_shape_path_specs_expr,
    expand_nested_shape_schema_normalize_expr, expand_plugin_command_usage_expr,
    generated_input_alias_specs, generated_input_flatten_shape_types,
    generated_input_nested_shape_fields, lit_str_from_text, nested_input_shape_spec_from_type,
    tool_spec_schema_metadata_calls,
};

pub fn expand_plugin_command_definition(
    command: &PluginCommandPlan,
) -> Result<proc_macro2::TokenStream> {
    let id = &command.id;
    let title = &command.title;
    let description = &command.description;
    let category = &command.category;
    let slash = option_lit_str_expr(command.slash.as_ref());
    let aliases = &command.aliases;
    let usage = expand_plugin_command_usage_expr(command)?;
    let location = &command.location;
    let input_schema = match &command.handler {
        PluginCommandHandlerPlan::Method { input, .. } => match input {
            PluginCommandInputPlan::Typed { ty, .. } => {
                quote! { Some(<#ty as ::agena_plugin_sdk::ToolInput>::input_schema()) }
            }
            PluginCommandInputPlan::Generated { input_model, .. } => {
                let schema = expand_plugin_tool_input_schema(input_model)?;
                quote! { Some(#schema) }
            }
            PluginCommandInputPlan::None | PluginCommandInputPlan::Raw { .. } => quote! { None },
        },
        PluginCommandHandlerPlan::InvokeTool { input_model, .. } => {
            let schema = expand_plugin_tool_input_schema(input_model)?;
            quote! { Some(#schema) }
        }
    };
    let action = command.action.as_ref().map_or_else(
        || match &command.handler {
            PluginCommandHandlerPlan::Method { .. } => quote! {
                ::agena_plugin_sdk::PluginUiAction::InvokeCommand {
                    command: #id.to_string(),
                    input: None,
                }
            },
            PluginCommandHandlerPlan::InvokeTool {
                tool,
                submit_output_as_prompt,
                ..
            } => quote! {
                ::agena_plugin_sdk::PluginUiAction::InvokeTool {
                    tool: #tool.to_string(),
                    input: None,
                    submit_output_as_prompt: #submit_output_as_prompt,
                }
            },
        },
        |action| quote! { #action },
    );
    let handler = match &command.handler {
        PluginCommandHandlerPlan::Method { .. } => quote! { Some(#id.to_string()) },
        PluginCommandHandlerPlan::InvokeTool { .. } => quote! { Some(#id.to_string()) },
    };
    Ok(quote! {
        manifest.commands.push(::agena_plugin_sdk::PluginCommandDefinition {
            id: #id.to_string(),
            title: #title.to_string(),
            description: #description.to_string(),
            category: #category.to_string(),
            slash: #slash,
            aliases: vec![#(#aliases.to_string()),*],
            usage: #usage,
            location: #location.to_string(),
            input_schema: #input_schema,
            handler: #handler,
            action: #action,
        });
    })
}

pub fn expand_input_path_specs(specs: &[PluginInputPathSpec]) -> proc_macro2::TokenStream {
    if specs.is_empty() {
        return quote! { ::std::vec::Vec::new() };
    }
    let items = specs.iter().map(|spec| {
        let jsonpath = &spec.jsonpath;
        let kind = path_permission_kind_expr(spec.kind);
        let fallback = option_lit_str_expr(spec.fallback.as_ref());
        let optional = spec.optional;
        quote! {
            ::agena_plugin_sdk::InputPathSpec {
                jsonpath: #jsonpath.to_string(),
                kind: #kind,
                fallback: #fallback,
                optional: #optional,
            }
        }
    });
    quote! { vec![#(#items),*] }
}

pub fn expand_input_network_specs(specs: &[PluginInputNetworkSpec]) -> proc_macro2::TokenStream {
    if specs.is_empty() {
        return quote! { ::std::vec::Vec::new() };
    }
    let items = specs.iter().map(|spec| {
        let jsonpath = &spec.jsonpath;
        let fallback = option_lit_str_expr(spec.fallback.as_ref());
        let optional = spec.optional;
        quote! {
            ::agena_plugin_sdk::InputNetworkSpec {
                jsonpath: #jsonpath.to_string(),
                fallback: #fallback,
                optional: #optional,
            }
        }
    });
    quote! { vec![#(#items),*] }
}

pub fn expand_input_tags(
    paths: &[PluginInputPathSpec],
    networks: &[PluginInputNetworkSpec],
) -> proc_macro2::TokenStream {
    let mut tags = Vec::new();
    for path in paths {
        tags.push(match path.kind {
            PluginPathPermissionKind::Read => {
                quote! { ::agena_plugin_sdk::ToolTag::FilesystemRead }
            }
            PluginPathPermissionKind::Write => {
                quote! { ::agena_plugin_sdk::ToolTag::FilesystemWrite }
            }
        });
    }
    for network in networks {
        tags.push(quote! { ::agena_plugin_sdk::ToolTag::Network });
        match network.semantic {
            PluginNetworkSemantic::Internet => {
                tags.push(quote! { ::agena_plugin_sdk::ToolTag::Internet });
            }
            PluginNetworkSemantic::Private => {
                tags.push(quote! { ::agena_plugin_sdk::ToolTag::PrivateNetwork });
            }
            PluginNetworkSemantic::Network
            | PluginNetworkSemantic::Url
            | PluginNetworkSemantic::Host => {}
        }
    }
    if tags.is_empty() {
        quote! { ::std::vec::Vec::new() }
    } else {
        quote! { vec![#(#tags),*] }
    }
}

pub fn expand_plugin_tool_definition(
    model: &PluginGeneratedToolInput,
) -> Result<proc_macro2::TokenStream> {
    let spec = &model.spec;
    let flatten_shapes = generated_input_flatten_shape_types(&model.input_fields)?;
    let nested_shapes = generated_input_nested_shape_fields(&model.input_fields);
    let docs = model.docs.as_deref();
    let tool = spec.tool.as_ref().ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "generated tool is missing a tool name",
        )
    })?;
    let summary = spec
        .summary
        .clone()
        .or_else(|| lit_str_from_text(doc_summary(docs).as_deref()))
        .ok_or_else(|| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                "generated tool is missing summary metadata or doc comments",
            )
        })?;
    let concurrency_safe = spec.concurrency_safe;
    let strict = spec.strict;
    let input_schema_expr = expand_plugin_tool_input_schema(model)?;
    let output_schema_expr = spec
        .output_ty
        .as_ref()
        .map(|ty| quote! { ::agena_plugin_sdk::macro_support::json_schema_for::<#ty>() })
        .unwrap_or_else(|| quote! { ::agena_plugin_sdk::serde_json::Value::Null });
    let help_expr = spec
        .help
        .as_ref()
        .cloned()
        .or_else(|| lit_str_from_text(docs))
        .map(|value| quote! { Some(#value.to_string()) })
        .unwrap_or_else(|| quote! { None });
    let before_help_expr = spec
        .before_help
        .as_ref()
        .map(|value| quote! { Some(#value.to_string()) })
        .unwrap_or_else(|| quote! { None });
    let after_help_expr = spec
        .after_help
        .as_ref()
        .map(|value| quote! { Some(#value.to_string()) })
        .unwrap_or_else(|| quote! { None });
    let examples_expr = if spec.examples.is_empty() {
        quote! { ::std::vec::Vec::new() }
    } else {
        let examples = &spec.examples;
        quote! { vec![#(#examples.to_string()),*] }
    };
    let display_preset = match spec.display.as_ref().map(LitStr::value).as_deref() {
        Some("brief") | Some("compact") => {
            Some(quote! { ::agena_plugin_sdk::manifest::ToolDisplayPreset::Compact })
        }
        Some("brief_detailed") => {
            Some(quote! { ::agena_plugin_sdk::manifest::ToolDisplayPreset::BriefDetailed })
        }
        Some("detailed") => {
            Some(quote! { ::agena_plugin_sdk::manifest::ToolDisplayPreset::Detailed })
        }
        Some(other) => {
            let invalid = spec.display.clone().expect("display was matched as Some");
            return Err(syn::Error::new_spanned(
                invalid,
                format!("unsupported tool display preset '{other}'"),
            ));
        }
        None => None,
    };
    let ui_display_override = match spec.ui_display.as_ref().map(LitStr::value).as_deref() {
        Some("brief") | Some("summary") => {
            Some(quote! { ::agena_plugin_sdk::UiTextDisplayMode::Summary })
        }
        Some("detailed") => Some(quote! { ::agena_plugin_sdk::UiTextDisplayMode::Detailed }),
        Some(other) => {
            let invalid = spec
                .ui_display
                .clone()
                .expect("ui_display was matched as Some");
            return Err(syn::Error::new_spanned(
                invalid,
                format!("unsupported ui display mode '{other}'"),
            ));
        }
        None => None,
    };
    let description_mode_override =
        match spec.description_mode.as_ref().map(LitStr::value).as_deref() {
            Some("brief") => Some(quote! { ::agena_plugin_sdk::ToolDescriptionMode::Brief }),
            Some("detailed") => Some(quote! { ::agena_plugin_sdk::ToolDescriptionMode::Detailed }),
            Some(other) => {
                let invalid = spec
                    .description_mode
                    .clone()
                    .expect("description_mode was matched as Some");
                return Err(syn::Error::new_spanned(
                    invalid,
                    format!("unsupported tool description mode '{other}'"),
                ));
            }
            None => None,
        };
    let ui_display_mode_override = match spec.ui_display_mode.as_ref().map(LitStr::value).as_deref()
    {
        Some("summary") => Some(quote! { ::agena_plugin_sdk::UiTextDisplayMode::Summary }),
        Some("detailed") => Some(quote! { ::agena_plugin_sdk::UiTextDisplayMode::Detailed }),
        Some(other) => {
            let invalid = spec
                .ui_display_mode
                .clone()
                .expect("ui_display_mode was matched as Some");
            return Err(syn::Error::new_spanned(
                invalid,
                format!("unsupported tool ui display mode '{other}'"),
            ));
        }
        None => None,
    };
    let description_mode_expr = if let Some(mode) = description_mode_override {
        quote! { Some(#mode) }
    } else if let Some(preset) = display_preset.as_ref() {
        quote! { Some(#preset.tool_description_mode()) }
    } else {
        quote! { None }
    };
    let ui_display_mode_expr = if let Some(mode) = ui_display_mode_override.or(ui_display_override)
    {
        quote! { Some(#mode) }
    } else if let Some(preset) = display_preset.as_ref() {
        quote! { Some(#preset.ui_display_mode()) }
    } else {
        quote! { None }
    };
    let tags_expr = if spec.tags.is_empty() {
        quote! { ::std::vec::Vec::new() }
    } else {
        let tags = &spec.tags;
        quote! { vec![#(#tags),*] }
    };
    let spec_input_paths_expr = expand_input_path_specs(&spec.input_paths);
    let spec_input_networks_expr = expand_input_network_specs(&spec.input_networks);
    let spec_input_tags_expr = expand_input_tags(&spec.input_paths, &spec.input_networks);
    let nested_paths_expr = expand_nested_shape_path_specs_expr(&nested_shapes, false);
    let nested_networks_expr = expand_nested_shape_network_specs_expr(&nested_shapes, false);
    let flatten_tag_exprs = flatten_shapes.iter().map(|ty| {
        quote! {
            __tags.extend(<#ty as ::agena_plugin_sdk::ToolInput>::input_tags());
        }
    });
    let nested_tag_exprs = nested_shapes.iter().map(|field| {
        let ty = &field.spec.inner_ty;
        quote! {
            __tags.extend(<#ty as ::agena_plugin_sdk::ToolInput>::input_tags());
        }
    });
    let input_paths_expr = if let Some(input_shape_ty) = spec.input_shape.as_ref() {
        quote! {{
            let mut __items = <#input_shape_ty as ::agena_plugin_sdk::ToolInput>::input_paths();
            __items.extend(#spec_input_paths_expr);
            __items
        }}
    } else if flatten_shapes.is_empty() && nested_shapes.is_empty() {
        spec_input_paths_expr
    } else {
        quote! {{
            let mut __items = #spec_input_paths_expr;
            #(
                __items.extend(<#flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_paths());
            )*
            #nested_paths_expr
            __items
        }}
    };
    let input_networks_expr = if let Some(input_shape_ty) = spec.input_shape.as_ref() {
        quote! {{
            let mut __items = <#input_shape_ty as ::agena_plugin_sdk::ToolInput>::input_networks();
            __items.extend(#spec_input_networks_expr);
            __items
        }}
    } else if flatten_shapes.is_empty() && nested_shapes.is_empty() {
        spec_input_networks_expr
    } else {
        quote! {{
            let mut __items = #spec_input_networks_expr;
            #(
                __items.extend(<#flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_networks());
            )*
            #nested_networks_expr
            __items
        }}
    };
    let tags_expr = if let Some(input_shape_ty) = spec.input_shape.as_ref() {
        quote! {{
            let mut __tags = #tags_expr;
            __tags.extend(#spec_input_tags_expr);
            __tags.extend(<#input_shape_ty as ::agena_plugin_sdk::ToolInput>::input_tags());
            ::agena_plugin_sdk::macro_support::dedupe_tool_tags(&mut __tags);
            __tags
        }}
    } else if flatten_shapes.is_empty() && nested_shapes.is_empty() {
        quote! {{
            let mut __tags = #tags_expr;
            __tags.extend(#spec_input_tags_expr);
            ::agena_plugin_sdk::macro_support::dedupe_tool_tags(&mut __tags);
            __tags
        }}
    } else {
        quote! {{
            let mut __tags = #tags_expr;
            __tags.extend(#spec_input_tags_expr);
            #(#flatten_tag_exprs)*
            #(#nested_tag_exprs)*
            ::agena_plugin_sdk::macro_support::dedupe_tool_tags(&mut __tags);
            __tags
        }}
    };
    let capabilities_expr = if spec.capabilities.is_empty() {
        quote! { ::std::vec::Vec::new() }
    } else {
        let capabilities = &spec.capabilities;
        quote! { vec![#(#capabilities),*] }
    };
    let streaming_expr = if spec.streaming {
        quote! { ::agena_plugin_sdk::ToolStreamingMode::Streaming }
    } else {
        quote! { ::agena_plugin_sdk::ToolStreamingMode::default() }
    };

    Ok(quote! {{
        let input_schema = #input_schema_expr;
        ::agena_plugin_sdk::ToolDefinition {
            name: #tool.to_string(),
            contract: ::agena_plugin_sdk::manifest::ToolContract {
                input_schema,
                output_schema: #output_schema_expr,
                strict: #strict,
            },
            model: ::agena_plugin_sdk::manifest::ToolModelSurface {
                examples: #examples_expr,
            },
            docs: ::agena_plugin_sdk::manifest::ToolDocs {
                before_help: #before_help_expr,
                after_help: #after_help_expr,
                summary: Some(#summary.to_string()),
                help: #help_expr,
            },
            runtime: ::agena_plugin_sdk::manifest::ToolRuntimePolicy {
                concurrency_safe: #concurrency_safe,
                streaming: #streaming_expr,
                result_policy: ::agena_plugin_sdk::ToolResultPolicy::default(),
            },
            permissions: ::agena_plugin_sdk::manifest::ToolPermissionContract {
                input_paths: #input_paths_expr,
                input_networks: #input_networks_expr,
                path_access: ::std::vec::Vec::new(),
                network_access: ::std::vec::Vec::new(),
                tags: #tags_expr,
            },
            display: ::agena_plugin_sdk::manifest::ToolDisplay {
                description_mode: #description_mode_expr,
                ui_display_mode: #ui_display_mode_expr,
            },
            capabilities: #capabilities_expr,
        }
    }})
}

pub fn expand_plugin_tool_input_schema(
    model: &PluginGeneratedToolInput,
) -> Result<proc_macro2::TokenStream> {
    let spec = &model.spec;
    let input_ty = &model.input_ty;
    let flatten_schema_calls = model
        .input_fields
        .iter()
        .enumerate()
        .filter(|(_, field)| field.flatten_shape)
        .map(|(index, field)| {
            let pointer = LitStr::new("", field.ident.span());
            let order = LitStr::new(format!("{index:06}").as_str(), field.ident.span());
            let ty = &field.ty;
            quote! {
                let mut overlay = <#ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                ::agena_plugin_sdk::macro_support::prefix_schema_order_metadata(
                    &mut overlay,
                    #order,
                );
                ::agena_plugin_sdk::macro_support::merge_flattened_schema_at_pointer(
                    schema,
                    #pointer,
                    &overlay,
                );
            }
        })
        .collect::<Vec<_>>();
    let alias_calls = model
        .input_fields
        .iter()
        .filter(|field| !field.flatten_shape && !field.aliases.is_empty())
        .map(|field| {
            let pointer = LitStr::new(
                format!("/properties/{}", field.wire_name.value()).as_str(),
                field.wire_name.span(),
            );
            let aliases = field.aliases.iter().collect::<Vec<_>>();
            quote! {
                ::agena_plugin_sdk::macro_support::set_schema_string_list_metadata(
                    schema,
                    #pointer,
                    "x-agena-aliases",
                    &[#(#aliases),*],
                );
            }
        })
        .collect::<Vec<_>>();
    let nested_schema_calls = model
        .input_fields
        .iter()
        .enumerate()
        .filter_map(|(index, field)| {
            let spec = field
                .nested_shape
                .then(|| nested_input_shape_spec_from_type(&field.ty))
                .flatten()?;
            let pointer = if spec.array {
                format!("/properties/{}/items", field.wire_name.value())
            } else {
                format!("/properties/{}", field.wire_name.value())
            };
            let pointer = LitStr::new(pointer.as_str(), field.wire_name.span());
            let order = LitStr::new(format!("{index:06}").as_str(), field.wire_name.span());
            let inner_ty = &spec.inner_ty;
            Some(quote! {
                let mut overlay = <#inner_ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                ::agena_plugin_sdk::macro_support::prefix_schema_order_metadata(
                    &mut overlay,
                    #order,
                );
                ::agena_plugin_sdk::macro_support::merge_schema_overlay_at_pointer(
                    schema,
                    #pointer,
                    &overlay,
                );
            })
        })
        .collect::<Vec<_>>();
    let metadata_calls = tool_spec_schema_metadata_calls(spec)?;
    let order_calls = model
        .input_fields
        .iter()
        .enumerate()
        .filter(|(_, field)| !field.flatten_shape)
        .map(|(index, field)| {
            let pointer = LitStr::new(
                format!("/properties/{}", field.wire_name.value()).as_str(),
                field.wire_name.span(),
            );
            let order = LitStr::new(format!("{index:06}").as_str(), field.wire_name.span());
            quote! {
                ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                    schema,
                    #pointer,
                    "x-agena-order",
                    #order,
                );
            }
        })
        .collect::<Vec<_>>();
    let default_calls = model
        .input_fields
        .iter()
        .filter(|field| !field.flatten_shape)
        .filter_map(|field| {
            let pointer = LitStr::new(
                format!("/properties/{}", field.wire_name.value()).as_str(),
                field.wire_name.span(),
            );
            let ty = &field.ty;
            if let Some(expr) = field.default_expr.as_ref() {
                Some(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_value_metadata(
                        schema,
                        #pointer,
                        "default",
                        ::agena_plugin_sdk::serde_json::json!(#expr),
                    );
                })
            } else if field.default {
                Some(quote! {
                    if let Ok(__default) = ::agena_plugin_sdk::serde_json::to_value(
                        <#ty as ::core::default::Default>::default(),
                    ) {
                        ::agena_plugin_sdk::macro_support::set_schema_value_metadata(
                            schema,
                            #pointer,
                            "default",
                            __default,
                        );
                    }
                })
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let schema_source = if let Some(input_shape_ty) = spec.input_shape.as_ref() {
        quote! { <#input_shape_ty as ::agena_plugin_sdk::ToolInput>::input_schema() }
    } else {
        quote! { ::agena_plugin_sdk::macro_support::json_schema_for::<#input_ty>() }
    };
    Ok(quote! {{
        let mut schema = #schema_source;
        {
            let schema = &mut schema;
            #(#default_calls)*
            #(#alias_calls)*
            #(#flatten_schema_calls)*
            #(#nested_schema_calls)*
            #(#metadata_calls)*
            #(#order_calls)*
        }
        schema
    }})
}

pub fn expand_plugin_tool_parse_input(
    model: &PluginGeneratedToolInput,
    input_expr: proc_macro2::TokenStream,
    cache_label: &Ident,
) -> Result<proc_macro2::TokenStream> {
    let spec = &model.spec;
    let input_ty = &model.input_ty;
    let flatten_shapes = generated_input_flatten_shape_types(&model.input_fields)?;
    let nested_shapes = generated_input_nested_shape_fields(&model.input_fields);
    let input_aliases = generated_input_alias_specs(&model.input_fields);
    let input_alias_normalize_expr = expand_input_alias_normalize_tokens(&input_aliases);
    let flatten_shape_input_normalize_expr =
        expand_flatten_shape_schema_normalize_expr(&flatten_shapes);
    let nested_shape_input_normalize_expr =
        expand_nested_shape_schema_normalize_expr(&nested_shapes);
    let nested_shape_post_parse_expr = expand_generated_input_post_parse_tokens(model);
    let built_in_normalize_expr = built_in_normalization_tokens(
        quote! { &mut input },
        &spec.trim,
        &spec.trim_suffix,
        &flatten_shapes,
        &nested_shapes,
    );
    let built_in_post_parse_normalize_expr = built_in_post_parse_normalization_tokens(
        &spec.trim,
        &spec.trim_suffix,
        &flatten_shapes,
        &nested_shapes,
    );
    let normalize_expr = spec
        .normalize
        .as_ref()
        .map(|path| quote! { #path(input)? })
        .unwrap_or_else(|| quote! { input });
    let validate_expr = spec
        .validate
        .as_ref()
        .map(|path| quote! { #path(&parsed)?; })
        .unwrap_or_default();
    let built_in_validate_expr = built_in_validation_tokens(
        quote! { parsed },
        &spec.non_empty,
        &spec.non_empty_if_present,
        &spec.minimums,
        &spec.maximums,
        &spec.exclusive_minimums,
        &spec.exclusive_maximums,
        &spec.exactly_one_of,
        &spec.at_least_one_of,
        &spec.requires,
        &spec.conflicts_with,
        &spec.required_unless_present,
        &spec.forbid_substrings,
        &spec.distinct_trimmed,
        &spec.distinct_trimmed_within,
        &spec.min_items,
        &spec.max_items,
        &spec.min_properties,
        &spec.max_properties,
        &spec.min_chars,
        &spec.max_chars,
        &spec.formats,
        &spec.patterns,
        &spec.choices,
        &flatten_shapes,
        &nested_shapes,
    );

    if let Some(input_shape_ty) = spec.input_shape.as_ref() {
        return Ok(quote! {{
            let mut input = #input_expr;
            #input_alias_normalize_expr
            #nested_shape_input_normalize_expr
            #flatten_shape_input_normalize_expr
            #built_in_normalize_expr
            let input = #normalize_expr;
            let parsed = <#input_shape_ty as ::agena_plugin_sdk::ToolInput>::parse_input(input)?;
            let parsed = #built_in_post_parse_normalize_expr;
            let parsed = #nested_shape_post_parse_expr;
            #built_in_validate_expr
            #validate_expr
            parsed
        }});
    }

    let cache_label = sanitize_generated_ident_label(&cache_label.to_string()).to_ascii_uppercase();
    let schema_static = format_ident!("__AGENA_TOOL_INPUT_SCHEMA_{}", cache_label);
    let input_schema_expr = expand_plugin_tool_input_schema(model)?;
    Ok(quote! {{
        static #schema_static: ::std::sync::OnceLock<::agena_plugin_sdk::serde_json::Value> =
            ::std::sync::OnceLock::new();
        let mut input = #input_expr;
        #input_alias_normalize_expr
        #nested_shape_input_normalize_expr
        #flatten_shape_input_normalize_expr
        #built_in_normalize_expr
        let input = #normalize_expr;
        let schema = #schema_static.get_or_init(|| #input_schema_expr);
        let parsed = ::agena_plugin_sdk::macro_support::parse_typed_json_value_with_field_suggestions::<#input_ty>(
            input,
            schema,
            "field",
        )?;
        let parsed = #built_in_post_parse_normalize_expr;
        let parsed = #nested_shape_post_parse_expr;
        #built_in_validate_expr
        #validate_expr
        parsed
    }})
}

fn option_lit_str_expr(value: Option<&LitStr>) -> proc_macro2::TokenStream {
    value
        .map(|value| quote! { Some(#value.to_string()) })
        .unwrap_or_else(|| quote! { None })
}

fn path_permission_kind_expr(kind: PluginPathPermissionKind) -> proc_macro2::TokenStream {
    match kind {
        PluginPathPermissionKind::Read => quote! { ::agena_plugin_sdk::PathKind::Read },
        PluginPathPermissionKind::Write => quote! { ::agena_plugin_sdk::PathKind::Write },
    }
}
