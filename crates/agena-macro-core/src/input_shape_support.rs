//! Shape computation for tool input types.

use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Attribute, Data, Field, Fields, Index, LitStr, Member, Meta, Result, Token, Variant};

use crate::serde_rename_support::{
    SerdeRenameRule, field_schema_property_name_with_rule, serde_rename_all_fields_rule,
    serde_rename_all_rule,
};

use super::{
    PluginGeneratedToolInput, nested_input_shape_field, nested_input_shape_spec,
    nested_input_shape_spec_from_type,
};

pub fn named_field_object_insert_tokens<'a, I>(
    fields: I,
    flatten_error: &str,
    rename_rule: Option<SerdeRenameRule>,
) -> Result<Vec<proc_macro2::TokenStream>>
where
    I: IntoIterator<Item = (&'a Field, &'a syn::Ident)>,
{
    fields
        .into_iter()
        .map(|(field, binding)| {
            if field_is_flatten(field)? {
                Ok(quote! {
                    match serde_json::to_value(#binding)
                        .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params_error(&err))? {
                        serde_json::Value::Object(flattened) => {
                            object.extend(flattened);
                        }
                        _ => {
                            return Err(::agena_plugin_sdk::PluginError::invalid_params(
                                #flatten_error,
                            ));
                        }
                    }
                })
            } else {
                let Some(name) = field_schema_property_name_with_rule(field, rename_rule)? else {
                    return Err(syn::Error::new_spanned(
                        field,
                        "named field is missing serializable property name",
                    ));
                };
                let name = LitStr::new(&name, field.span());
                Ok(quote! {
                    object.insert(
                        #name.to_string(),
                        serde_json::to_value(#binding).map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params_error(&err))?,
                    );
                })
            }
        })
        .collect()
}

pub fn flatten_shape_type(field: &Field) -> Result<Option<syn::Type>> {
    if !field_is_flatten(field)? {
        return Ok(None);
    }
    for attr in &field.attrs {
        if !attr.path().is_ident("input") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            if let Meta::Path(path) = meta
                && path.is_ident("flatten_shape")
            {
                return Ok(Some(field.ty.clone()));
            }
        }
    }
    Ok(None)
}

fn nested_input_shape_post_parse_expr(
    target: proc_macro2::TokenStream,
    spec: &super::NestedInputShapeSpec,
    schema_path: Option<&LitStr>,
) -> proc_macro2::TokenStream {
    let ty = &spec.inner_ty;
    match (spec.optional, spec.array) {
        (false, false) => {
            if let Some(schema_path) = schema_path {
                quote! {{
                    let __inner_schema = <#ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                    let __macro_parse_result = <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                        serde_json::to_value(&#target)
                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params_error(&err))?,
                    );
                    let __mappings =
                        ::agena_plugin_sdk::macro_support::prefixed_input_error_path_mappings(
                            &__inner_schema,
                            #schema_path,
                        );
                    ::agena_plugin_sdk::macro_support::remap_invalid_params_paths_owned(
                        __macro_parse_result,
                        __mappings.as_slice(),
                    )?
                }}
            } else {
                quote! {
                    <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                        serde_json::to_value(&#target)
                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params_error(&err))?,
                    )?
                }
            }
        }
        (true, false) => {
            if let Some(schema_path) = schema_path {
                quote! {
                    match &#target {
                        Some(value) => {
                            let __inner_schema = <#ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                            let __macro_parse_result =
                                <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                    serde_json::to_value(value).map_err(|err| {
                                        ::agena_plugin_sdk::PluginError::invalid_params_error(&err)
                                    })?,
                                );
                            let __mappings =
                                ::agena_plugin_sdk::macro_support::prefixed_input_error_path_mappings(
                                    &__inner_schema,
                                    #schema_path,
                                );
                            Some(
                                ::agena_plugin_sdk::macro_support::remap_invalid_params_paths_owned(
                                    __macro_parse_result,
                                    __mappings.as_slice(),
                                )?,
                            )
                        }
                        None => None,
                    }
                }
            } else {
                quote! {
                    match &#target {
                        Some(value) => Some(
                            <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                serde_json::to_value(value)
                                    .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params_error(&err))?,
                            )?,
                        ),
                        None => None,
                    }
                }
            }
        }
        (false, true) => {
            if let Some(schema_path) = schema_path {
                quote! {{
                    let __inner_schema = <#ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                    #target
                        .iter()
                        .enumerate()
                        .map(|(__index, value)| {
                            let __macro_parse_result =
                                <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                    serde_json::to_value(value).map_err(|err| {
                                        ::agena_plugin_sdk::PluginError::invalid_params_error(&err)
                                    })?,
                                );
                            let __prefix = format!("{}[{__index}]", #schema_path);
                            let __mappings =
                                ::agena_plugin_sdk::macro_support::prefixed_input_error_path_mappings(
                                    &__inner_schema,
                                    __prefix.as_str(),
                                );
                            ::agena_plugin_sdk::macro_support::remap_invalid_params_paths_owned(
                                __macro_parse_result,
                                __mappings.as_slice(),
                            )
                        })
                        .collect::<::agena_plugin_sdk::Result<Vec<_>>>()?
                }}
            } else {
                quote! {
                    #target
                        .iter()
                        .map(|value| {
                            <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                serde_json::to_value(value)
                                    .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params_error(&err))?,
                            )
                        })
                        .collect::<::agena_plugin_sdk::Result<Vec<_>>>()?
                }
            }
        }
        (true, true) => {
            if let Some(schema_path) = schema_path {
                quote! {
                    match &#target {
                        Some(values) => {
                            let __inner_schema = <#ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                            Some(
                                values
                                    .iter()
                                    .enumerate()
                                    .map(|(__index, value)| {
                                        let __macro_parse_result =
                                            <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                                serde_json::to_value(value).map_err(|err| {
                                                    ::agena_plugin_sdk::PluginError::invalid_params_error(&err)
                                                })?,
                                            );
                                        let __prefix = format!("{}[{__index}]", #schema_path);
                                        let __mappings =
                                            ::agena_plugin_sdk::macro_support::prefixed_input_error_path_mappings(
                                                &__inner_schema,
                                                __prefix.as_str(),
                                            );
                                        ::agena_plugin_sdk::macro_support::remap_invalid_params_paths_owned(
                                            __macro_parse_result,
                                            __mappings.as_slice(),
                                        )
                                    })
                                    .collect::<::agena_plugin_sdk::Result<Vec<_>>>()?,
                            )
                        }
                        None => None,
                    }
                }
            } else {
                quote! {
                    match &#target {
                        Some(values) => Some(
                            values
                                .iter()
                                .map(|value| {
                                    <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                        serde_json::to_value(value)
                                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params_error(&err))?,
                                    )
                                })
                                .collect::<::agena_plugin_sdk::Result<Vec<_>>>()?,
                        ),
                        None => None,
                    }
                }
            }
        }
    }
}

pub fn expand_flatten_shape_post_parse_tokens(
    attrs: &[Attribute],
    data: &Data,
) -> Result<proc_macro2::TokenStream> {
    match data {
        Data::Struct(data_struct) => {
            let rename_rule = serde_rename_all_rule(attrs)?;
            let updates = data_struct
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let flatten_shape_ty = flatten_shape_type(field)?;
                    let nested_shape = nested_input_shape_spec(field)?;
                    let nested_shape_field = nested_input_shape_field(field, rename_rule)?;
                    let member = field
                        .ident
                        .clone()
                        .map(Member::Named)
                        .unwrap_or_else(|| Member::Unnamed(Index::from(index)));
                    Ok(
                        flatten_shape_ty
                            .map(|ty| {
                                quote! {
                                    parsed.#member = <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                        serde_json::to_value(&parsed.#member)
                                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params_error(&err))?,
                                    )?;
                                }
                            })
                            .or_else(|| {
                                nested_shape.as_ref().map(|spec| {
                                    let expr = nested_input_shape_post_parse_expr(
                                        quote! { parsed.#member },
                                        spec,
                                        nested_shape_field.as_ref().map(|field| &field.schema_path),
                                    );
                                    quote! {
                                        parsed.#member = #expr;
                                    }
                                })
                            }),
                    )
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            if updates.is_empty() {
                Ok(quote! { parsed })
            } else {
                Ok(quote! {{
                    let mut parsed = parsed;
                    #(#updates)*
                    parsed
                }})
            }
        }
        Data::Enum(data_enum) => {
            let enum_field_rule = serde_rename_all_fields_rule(attrs)?;
            let arms = data_enum
                .variants
                .iter()
                .map(|variant| {
                    expand_flatten_shape_variant_post_parse_arm(variant, enum_field_rule)
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            if arms.is_empty() {
                Ok(quote! { parsed })
            } else {
                Ok(quote! {
                    match parsed {
                        #(#arms,)*
                        other => other,
                    }
                })
            }
        }
        Data::Union(_) => Ok(quote! { parsed }),
    }
}

pub fn expand_generated_input_post_parse_tokens(
    model: &PluginGeneratedToolInput,
) -> proc_macro2::TokenStream {
    let updates = model
        .input_fields
        .iter()
        .filter_map(|field| {
            let ident = &field.ident;
            if field.flatten_shape {
                let ty = &field.ty;
                return Some(quote! {
                    parsed.#ident = <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                        serde_json::to_value(&parsed.#ident)
                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params_error(&err))?,
                    )?;
                });
            }
            let spec = field
                .nested_shape
                .then(|| nested_input_shape_spec_from_type(&field.ty))
                .flatten()?;
            let expr = nested_input_shape_post_parse_expr(
                quote! { parsed.#ident },
                &spec,
                Some(&field.wire_name),
            );
            Some(quote! {
                parsed.#ident = #expr;
            })
        })
        .collect::<Vec<_>>();
    if updates.is_empty() {
        quote! { parsed }
    } else {
        quote! {{
            let mut parsed = parsed;
            #(#updates)*
            parsed
        }}
    }
}

fn expand_flatten_shape_variant_post_parse_arm(
    variant: &Variant,
    enum_field_rule: Option<SerdeRenameRule>,
) -> Result<Option<proc_macro2::TokenStream>> {
    let variant_field_rule = serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule);
    match &variant.fields {
        Fields::Named(fields_named) => {
            let bindings = fields_named
                .named
                .iter()
                .map(|field| {
                    let ident = field
                        .ident
                        .clone()
                        .expect("named fields should have identifiers");
                    let flatten_shape_ty = flatten_shape_type(field)?;
                    let nested_shape = nested_input_shape_spec(field)?;
                    let nested_shape_field = nested_input_shape_field(field, variant_field_rule)?;
                    Ok((ident, flatten_shape_ty, nested_shape, nested_shape_field))
                })
                .collect::<Result<Vec<_>>>()?;
            if !bindings
                .iter()
                .any(|(_, flatten_shape_ty, nested_shape, _)| {
                    flatten_shape_ty.is_some() || nested_shape.is_some()
                })
            {
                return Ok(None);
            }
            let pattern_fields = bindings.iter().map(|(ident, _, _, _)| quote! { #ident });
            let normalize_bindings = bindings
                .iter()
                .filter_map(|(ident, flatten_shape_ty, nested_shape, nested_shape_field)| {
                    flatten_shape_ty
                        .as_ref()
                        .map(|ty| {
                            quote! {
                                let #ident = <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                    serde_json::to_value(&#ident)
                                        .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params_error(&err))?,
                                )?;
                            }
                        })
                        .or_else(|| {
                            nested_shape.as_ref().map(|spec| {
                                let expr = nested_input_shape_post_parse_expr(
                                    quote! { #ident },
                                    spec,
                                    nested_shape_field.as_ref().map(|field| &field.schema_path),
                                );
                                quote! {
                                    let #ident = #expr;
                                }
                            })
                        })
                })
                .collect::<Vec<_>>();
            let rebuild_fields = bindings.iter().map(|(ident, _, _, _)| quote! { #ident });
            let variant_ident = &variant.ident;
            Ok(Some(quote! {
                Self::#variant_ident { #(#pattern_fields),* } => {
                    #(#normalize_bindings)*
                    Self::#variant_ident { #(#rebuild_fields),* }
                }
            }))
        }
        Fields::Unnamed(fields_unnamed) => {
            let bindings = fields_unnamed
                .unnamed
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let binding = format_ident!("__flatten_field_{index}");
                    let flatten_shape_ty = flatten_shape_type(field)?;
                    let nested_shape = nested_input_shape_spec(field)?;
                    let nested_shape_field = nested_input_shape_field(field, variant_field_rule)?;
                    Ok((binding, flatten_shape_ty, nested_shape, nested_shape_field))
                })
                .collect::<Result<Vec<_>>>()?;
            if !bindings
                .iter()
                .any(|(_, flatten_shape_ty, nested_shape, _)| {
                    flatten_shape_ty.is_some() || nested_shape.is_some()
                })
            {
                return Ok(None);
            }
            let pattern_fields = bindings
                .iter()
                .map(|(binding, _, _, _)| quote! { #binding });
            let normalize_bindings = bindings
                .iter()
                .filter_map(|(binding, flatten_shape_ty, nested_shape, nested_shape_field)| {
                    flatten_shape_ty
                        .as_ref()
                        .map(|ty| {
                            quote! {
                                let #binding = <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                    serde_json::to_value(&#binding)
                                        .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params_error(&err))?,
                                )?;
                            }
                        })
                        .or_else(|| {
                            nested_shape.as_ref().map(|spec| {
                                let expr = nested_input_shape_post_parse_expr(
                                    quote! { #binding },
                                    spec,
                                    nested_shape_field.as_ref().map(|field| &field.schema_path),
                                );
                                quote! {
                                    let #binding = #expr;
                                }
                            })
                        })
                })
                .collect::<Vec<_>>();
            let rebuild_fields = bindings
                .iter()
                .map(|(binding, _, _, _)| quote! { #binding });
            let variant_ident = &variant.ident;
            Ok(Some(quote! {
                Self::#variant_ident(#(#pattern_fields),*) => {
                    #(#normalize_bindings)*
                    Self::#variant_ident(#(#rebuild_fields),*)
                }
            }))
        }
        Fields::Unit => Ok(None),
    }
}

pub fn field_is_flatten(field: &Field) -> Result<bool> {
    for attr in &field.attrs {
        if !attr.path().is_ident("serde") && !attr.path().is_ident("schemars") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            if let Meta::Path(path) = meta
                && path.is_ident("flatten")
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
