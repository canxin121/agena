use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Attribute, Field, Ident, LitStr, Meta, Result, Token};

use super::{expr_lit_str, ident_to_snake_case};

#[derive(Clone, Copy)]
pub(crate) enum SerdeRenameRule {
    LowerCase,
    UpperCase,
    PascalCase,
    CamelCase,
    SnakeCase,
    ScreamingSnakeCase,
    KebabCase,
    ScreamingKebabCase,
}

impl SerdeRenameRule {
    fn parse(value: &LitStr) -> Result<Self> {
        match value.value().as_str() {
            "lowercase" => Ok(Self::LowerCase),
            "UPPERCASE" => Ok(Self::UpperCase),
            "PascalCase" => Ok(Self::PascalCase),
            "camelCase" => Ok(Self::CamelCase),
            "snake_case" => Ok(Self::SnakeCase),
            "SCREAMING_SNAKE_CASE" => Ok(Self::ScreamingSnakeCase),
            "kebab-case" => Ok(Self::KebabCase),
            "SCREAMING-KEBAB-CASE" => Ok(Self::ScreamingKebabCase),
            other => Err(syn::Error::new_spanned(
                value,
                format!("unsupported serde rename_all rule '{other}'"),
            )),
        }
    }

    pub(crate) fn apply(self, ident: &Ident) -> String {
        let snake = ident_to_snake_case(ident);
        match self {
            Self::LowerCase => snake.replace('_', ""),
            Self::UpperCase => snake.replace('_', "").to_ascii_uppercase(),
            Self::PascalCase => pascal_case_from_snake(&snake),
            Self::CamelCase => {
                let mut value = pascal_case_from_snake(&snake);
                if let Some(first) = value.get_mut(0..1) {
                    first.make_ascii_lowercase();
                }
                value
            }
            Self::SnakeCase => snake,
            Self::ScreamingSnakeCase => snake.to_ascii_uppercase(),
            Self::KebabCase => snake.replace('_', "-"),
            Self::ScreamingKebabCase => snake.replace('_', "-").to_ascii_uppercase(),
        }
    }
}

fn pascal_case_from_snake(snake: &str) -> String {
    let mut output = String::new();
    for segment in snake.split('_').filter(|segment| !segment.is_empty()) {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            output.extend(first.to_uppercase());
            output.extend(chars);
        }
    }
    output
}

pub(crate) fn serde_rename_all_rule(attrs: &[Attribute]) -> Result<Option<SerdeRenameRule>> {
    serde_rename_all_rule_for_key(attrs, "rename_all")
}

pub(crate) fn serde_rename_all_fields_rule(attrs: &[Attribute]) -> Result<Option<SerdeRenameRule>> {
    serde_rename_all_rule_for_key(attrs, "rename_all_fields")
}

fn serde_rename_all_rule_for_key(
    attrs: &[Attribute],
    key: &str,
) -> Result<Option<SerdeRenameRule>> {
    let mut rule = None;
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            match meta {
                Meta::NameValue(value) if value.path.is_ident(key) => {
                    rule = Some(SerdeRenameRule::parse(&expr_lit_str(&value.value, key)?)?);
                }
                Meta::List(list) if list.path.is_ident(key) => {
                    if let Some(value) =
                        serde_rename_rule_list_value(list.tokens.clone(), key, "deserialize")?
                            .or(serde_rename_rule_list_value(list.tokens, key, "serialize")?)
                    {
                        rule = Some(SerdeRenameRule::parse(&value)?);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(rule)
}

fn serde_rename_rule_list_value(
    tokens: proc_macro2::TokenStream,
    list_name: &str,
    key: &str,
) -> Result<Option<LitStr>> {
    let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(tokens)?;
    for meta in metas {
        if let Meta::NameValue(value) = meta
            && value.path.is_ident(key)
        {
            return Ok(Some(expr_lit_str(&value.value, list_name)?));
        }
    }
    Ok(None)
}

pub(crate) fn field_schema_property_name_with_rule(
    field: &Field,
    rename_rule: Option<SerdeRenameRule>,
) -> Result<Option<String>> {
    let Some(ident) = field.ident.as_ref() else {
        return Ok(None);
    };
    let mut serde_rename = None;
    let mut schema_rename = None;
    for attr in &field.attrs {
        if !attr.path().is_ident("serde") && !attr.path().is_ident("schemars") {
            continue;
        }
        let is_serde = attr.path().is_ident("serde");
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            match meta {
                Meta::Path(path)
                    if is_serde
                        && (path.is_ident("skip") || path.is_ident("skip_deserializing")) =>
                {
                    return Ok(None);
                }
                Meta::Path(path) if !is_serde && path.is_ident("skip") => return Ok(None),
                Meta::Path(path) if path.is_ident("flatten") => return Ok(None),
                Meta::NameValue(value) => {
                    if value.path.is_ident("rename") {
                        let rename = expr_lit_str(&value.value, "rename")?.value();
                        if is_serde {
                            serde_rename = Some(rename);
                        } else {
                            schema_rename = Some(rename);
                        }
                    }
                }
                Meta::List(list) if list.path.is_ident("rename") => {
                    if let Some(rename) =
                        serde_rename_rule_list_value(list.tokens.clone(), "rename", "deserialize")?
                            .or(serde_rename_rule_list_value(
                                list.tokens,
                                "rename",
                                "serialize",
                            )?)
                    {
                        if is_serde {
                            serde_rename = Some(rename.value());
                        } else {
                            schema_rename = Some(rename.value());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(Some(serde_rename.or(schema_rename).unwrap_or_else(|| {
        rename_rule.map_or_else(|| ident.to_string(), |rule| rule.apply(ident))
    })))
}

pub(crate) fn field_has_serde_default(field: &Field) -> Result<bool> {
    for attr in &field.attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            match meta {
                Meta::Path(path) if path.is_ident("default") => return Ok(true),
                Meta::NameValue(value) if value.path.is_ident("default") => return Ok(true),
                _ => {}
            }
        }
    }
    Ok(false)
}

pub(crate) fn field_schema_aliases(field: &Field) -> Result<Vec<String>> {
    let mut aliases = Vec::new();
    for attr in &field.attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            if let Meta::NameValue(value) = meta
                && value.path.is_ident("alias")
            {
                aliases.push(expr_lit_str(&value.value, "alias")?.value());
            }
        }
    }
    Ok(aliases)
}
