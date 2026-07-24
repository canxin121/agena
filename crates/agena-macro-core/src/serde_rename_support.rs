use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Attribute, Field, Ident, LitStr, Meta, Result, Token};

use super::{expr_lit_str, ident_to_snake_case};

#[derive(Clone, Copy)]
pub enum SerdeRenameRule {
    Lower,
    Upper,
    Pascal,
    Camel,
    Snake,
    ScreamingSnake,
    Kebab,
    ScreamingKebab,
}

impl SerdeRenameRule {
    fn parse(value: &LitStr) -> Result<Self> {
        match value.value().as_str() {
            "lowercase" => Ok(Self::Lower),
            "UPPERCASE" => Ok(Self::Upper),
            "PascalCase" => Ok(Self::Pascal),
            "camelCase" => Ok(Self::Camel),
            "snake_case" => Ok(Self::Snake),
            "SCREAMING_SNAKE_CASE" => Ok(Self::ScreamingSnake),
            "kebab-case" => Ok(Self::Kebab),
            "SCREAMING-KEBAB-CASE" => Ok(Self::ScreamingKebab),
            other => Err(syn::Error::new_spanned(
                value,
                format!("unsupported serde rename_all rule '{other}'"),
            )),
        }
    }

    pub fn apply(self, ident: &Ident) -> String {
        let snake = ident_to_snake_case(ident);
        match self {
            Self::Lower => snake.replace('_', ""),
            Self::Upper => snake.replace('_', "").to_ascii_uppercase(),
            Self::Pascal => pascal_case_from_snake(&snake),
            Self::Camel => {
                let mut value = pascal_case_from_snake(&snake);
                if let Some(first) = value.get_mut(0..1) {
                    first.make_ascii_lowercase();
                }
                value
            }
            Self::Snake => snake,
            Self::ScreamingSnake => snake.to_ascii_uppercase(),
            Self::Kebab => snake.replace('_', "-"),
            Self::ScreamingKebab => snake.replace('_', "-").to_ascii_uppercase(),
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

pub fn serde_rename_all_rule(attrs: &[Attribute]) -> Result<Option<SerdeRenameRule>> {
    serde_rename_all_rule_for_key(attrs, "rename_all")
}

pub fn serde_rename_all_fields_rule(attrs: &[Attribute]) -> Result<Option<SerdeRenameRule>> {
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

pub fn field_schema_property_name_with_rule(
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

pub fn field_has_serde_default(field: &Field) -> Result<bool> {
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

pub fn field_schema_aliases(field: &Field) -> Result<Vec<String>> {
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
