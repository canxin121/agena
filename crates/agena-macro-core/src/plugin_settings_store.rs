//! Expansion of the `PluginSettingsStore` derive macro.

use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{
    Attribute, Data, DeriveInput, Expr, Field, Ident, Index, Member, Meta, PathArguments, Result,
    Token, Type,
};

pub fn expand_plugin_settings_store(input: DeriveInput) -> Result<proc_macro2::TokenStream> {
    let name = input.ident;
    let config_field = find_plugin_settings_store_field(&input.data)?;
    let field_member = config_field.member;
    let config_ty = config_field.config_ty;
    let schema_expr = match config_field.default {
        PluginConfigDefault::None => {
            quote! { ::agena_plugin_sdk::macro_support::json_schema_for::<#config_ty>() }
        }
        PluginConfigDefault::Default => {
            quote! {
                ::agena_plugin_sdk::macro_support::json_schema_for_default(
                    <#config_ty as ::core::default::Default>::default(),
                )
            }
        }
        PluginConfigDefault::Expr(default) => {
            quote! { ::agena_plugin_sdk::macro_support::json_schema_for_default(#default) }
        }
    };
    let generics = input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics ::agena_plugin_sdk::plugin::PluginSettingsStoreAccess for #name #ty_generics #where_clause {
            fn plugin_settings_schema() -> ::agena_plugin_sdk::serde_json::Value {
                #schema_expr
            }

            fn set_plugin_settings_from_json(
                &self,
                input: ::agena_plugin_sdk::serde_json::Value,
                invalid: &str,
                already: ::std::string::String,
            ) -> ::agena_plugin_sdk::Result<()> {
                self.#field_member.set_from_json(input, invalid, already)
            }
        }
    })
}

struct PluginSettingsStoreField {
    member: Member,
    config_ty: Type,
    default: PluginConfigDefault,
}

enum PluginConfigDefault {
    None,
    Default,
    Expr(Box<Expr>),
}

fn find_plugin_settings_store_field(data: &Data) -> Result<PluginSettingsStoreField> {
    let Data::Struct(data_struct) = data else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "PluginSettingsStore can only be derived for structs",
        ));
    };

    let mut found = None;
    for (index, field) in data_struct.fields.iter().enumerate() {
        let config_attr = parse_plugin_settings_store_field_attrs(field)?;
        if config_attr.is_none() {
            continue;
        }
        if found.is_some() {
            return Err(syn::Error::new_spanned(
                field,
                "PluginSettingsStore supports exactly one #[settings] field",
            ));
        }
        let member = match &field.ident {
            Some(ident) => Member::Named(ident.clone()),
            None => Member::Unnamed(Index::from(index)),
        };
        let config_ty = plugin_config_inner_type(field)?;
        found = Some(PluginSettingsStoreField {
            member,
            config_ty,
            default: config_attr.expect("config attr checked as present").default,
        });
    }

    found.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "PluginSettingsStore requires one field marked #[settings] or #[plugin_settings]",
        )
    })
}

fn is_plugin_settings_store_attr(attr: &Attribute) -> bool {
    attr.path().is_ident("settings") || attr.path().is_ident("plugin_settings")
}

struct PluginConfigFieldAttr {
    default: PluginConfigDefault,
}

fn parse_plugin_settings_store_field_attrs(field: &Field) -> Result<Option<PluginConfigFieldAttr>> {
    let mut found = false;
    let mut default = PluginConfigDefault::None;
    for attr in &field.attrs {
        if !is_plugin_settings_store_attr(attr) {
            continue;
        }
        found = true;
        let attr_default = parse_plugin_settings_store_field_attr(attr)?;
        match (&default, attr_default) {
            (PluginConfigDefault::None, next) => default = next,
            (_, PluginConfigDefault::None) => {}
            (_, _) => {
                return Err(syn::Error::new_spanned(
                    attr,
                    "duplicate #[settings] default option",
                ));
            }
        }
    }

    Ok(found.then_some(PluginConfigFieldAttr { default }))
}

fn parse_plugin_settings_store_field_attr(attr: &Attribute) -> Result<PluginConfigDefault> {
    match &attr.meta {
        Meta::Path(_) => Ok(PluginConfigDefault::None),
        Meta::NameValue(_) => Err(syn::Error::new_spanned(
            attr,
            "#[settings] only supports `#[settings]`, `#[settings(default)]`, or `#[settings(default = expr)]`",
        )),
        Meta::List(_) => attr.parse_args::<PluginConfigDefault>(),
    }
}

impl Parse for PluginConfigDefault {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        if input.is_empty() {
            return Ok(Self::None);
        }
        let key = input.call(Ident::parse_any)?;
        if key != "default" {
            return Err(syn::Error::new_spanned(
                key,
                "unsupported #[settings] option; expected `default`",
            ));
        }
        if input.peek(Token![=]) {
            let _: Token![=] = input.parse()?;
            let default = input.parse::<Expr>()?;
            if !input.is_empty() {
                return Err(input.error("unexpected trailing tokens in #[settings]"));
            }
            return Ok(Self::Expr(Box::new(default)));
        }
        if !input.is_empty() {
            return Err(input.error("unexpected trailing tokens in #[settings]"));
        }
        Ok(Self::Default)
    }
}

fn plugin_config_inner_type(field: &Field) -> Result<Type> {
    let Type::Path(path) = &field.ty else {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "#[settings] fields must have type PluginSettings<T>",
        ));
    };
    let Some(segment) = path.path.segments.last() else {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "#[settings] fields must have type PluginSettings<T>",
        ));
    };
    if segment.ident != "PluginSettings" {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "#[settings] fields must have type PluginSettings<T>",
        ));
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "#[settings] fields must specify PluginSettings<T>",
        ));
    };
    let mut types = args.args.iter().filter_map(|arg| match arg {
        syn::GenericArgument::Type(ty) => Some(ty.clone()),
        _ => None,
    });
    let config_ty = types.next().ok_or_else(|| {
        syn::Error::new_spanned(
            &field.ty,
            "#[settings] fields must specify PluginSettings<T>",
        )
    })?;
    if types.next().is_some() {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "#[settings] fields must specify exactly one PluginSettings<T> type",
        ));
    }
    Ok(config_ty)
}
