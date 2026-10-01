//! Shared parsing helpers for proc-macro entrypoints.

use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Attribute, Expr, ExprLit, ExprPath, Ident, Lit, LitBool, LitStr, Meta, Result, Token};

use super::{
    PathPairConstraint, PathStringConstraint, PathStringsConstraint, PathUsizeConstraint,
    PathValueConstraint, PathValuesConstraint, append_constraint_path_suffix, validate_format_lit,
    validate_pattern_lit,
};

pub fn default_tool_name(ident: &Ident) -> String {
    let name = ident_to_snake_case(ident);
    ["invoke_", "dispatch_", "handle_"]
        .into_iter()
        .find_map(|prefix| name.strip_prefix(prefix).map(str::to_string))
        .unwrap_or(name)
}

pub fn default_command_id(ident: &Ident) -> String {
    let name = ident_to_snake_case(ident);
    ["command_", "cmd_", "invoke_", "dispatch_", "handle_"]
        .into_iter()
        .find_map(|prefix| name.strip_prefix(prefix).map(str::to_string))
        .unwrap_or(name)
}

pub fn command_title_from_id(id: &str) -> String {
    id.split(['.', '_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn ident_to_snake_case(ident: &Ident) -> String {
    let chars = ident.to_string().chars().collect::<Vec<_>>();
    let mut out = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            let prev = chars[index - 1];
            let next = chars.get(index + 1).copied();
            if prev.is_ascii_lowercase()
                || prev.is_ascii_digit()
                || next.is_some_and(|next| next.is_ascii_lowercase())
            {
                out.push('_');
            }
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

pub fn parse_path_lit_str_list_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathStringsConstraint> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut iter = items.iter();
    let Some(first) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires a path string followed by one or more strings"),
        ));
    };
    let path = expr_lit_str(first, attribute)?;
    let values = iter
        .map(|item| expr_lit_str(item, attribute))
        .collect::<Result<Vec<_>>>()?;
    if values.is_empty() {
        return Err(syn::Error::new_spanned(
            path,
            format!("{attribute} requires at least one string value"),
        ));
    }
    Ok(PathStringsConstraint { path, values })
}

pub fn parse_path_expr_list_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathValuesConstraint> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut iter = items.iter();
    let Some(first) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires a path string followed by one or more values"),
        ));
    };
    let path = expr_lit_str(first, attribute)?;
    let values = iter.cloned().collect::<Vec<_>>();
    if values.is_empty() {
        return Err(syn::Error::new_spanned(
            path,
            format!("{attribute} requires at least one value"),
        ));
    }
    Ok(PathValuesConstraint { path, values })
}

pub fn parse_path_expr_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathValueConstraint> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut iter = items.iter();
    let Some(first) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires a path string and one value"),
        ));
    };
    let Some(second) = iter.next() else {
        return Err(syn::Error::new_spanned(
            first,
            format!("{attribute} requires a path string and one value"),
        ));
    };
    if iter.next().is_some() {
        return Err(syn::Error::new_spanned(
            second,
            format!("{attribute} accepts exactly two arguments"),
        ));
    }
    Ok(PathValueConstraint {
        path: expr_lit_str(first, attribute)?,
        value: second.clone(),
    })
}

pub fn parse_path_lit_str_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathStringConstraint> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut iter = items.iter();
    let Some(first) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires a path string and one string value"),
        ));
    };
    let Some(second) = iter.next() else {
        return Err(syn::Error::new_spanned(
            first,
            format!("{attribute} requires a path string and one string value"),
        ));
    };
    if iter.next().is_some() {
        return Err(syn::Error::new_spanned(
            second,
            format!("{attribute} accepts exactly two string arguments"),
        ));
    }
    Ok(PathStringConstraint {
        path: expr_lit_str(first, attribute)?,
        value: expr_lit_str(second, attribute)?,
    })
}

pub fn parse_item_path_lit_str_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathStringConstraint> {
    let mut constraint = parse_path_lit_str_constraint(tokens, attribute)?;
    constraint.path = append_constraint_path_suffix(&constraint.path, "[]");
    Ok(constraint)
}

pub fn parse_path_pattern_constraint(
    tokens: proc_macro2::TokenStream,
) -> Result<PathStringConstraint> {
    let constraint = parse_path_lit_str_constraint(tokens, "pattern")?;
    validate_pattern_lit(&constraint.value)?;
    Ok(constraint)
}

pub fn parse_path_format_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathStringConstraint> {
    let mut constraint = parse_path_lit_str_constraint(tokens, attribute)?;
    constraint.value = validate_format_lit(&constraint.value)?;
    Ok(constraint)
}

pub fn parse_item_path_usize_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathUsizeConstraint> {
    let mut constraint = parse_path_usize_constraint(tokens, attribute)?;
    constraint.path = append_constraint_path_suffix(&constraint.path, "[]");
    Ok(constraint)
}

pub fn parse_item_path_expr_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathValueConstraint> {
    let mut constraint = parse_path_expr_constraint(tokens, attribute)?;
    constraint.path = append_constraint_path_suffix(&constraint.path, "[]");
    Ok(constraint)
}

pub fn parse_item_path_pattern_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathStringConstraint> {
    let mut constraint = parse_path_lit_str_constraint(tokens, attribute)?;
    validate_pattern_lit(&constraint.value)?;
    constraint.path = append_constraint_path_suffix(&constraint.path, "[]");
    Ok(constraint)
}

pub fn parse_item_path_format_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathStringConstraint> {
    let mut constraint = parse_path_lit_str_constraint(tokens, attribute)?;
    constraint.value = validate_format_lit(&constraint.value)?;
    constraint.path = append_constraint_path_suffix(&constraint.path, "[]");
    Ok(constraint)
}

pub fn parse_item_path_expr_list_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathValuesConstraint> {
    let mut constraint = parse_path_expr_list_constraint(tokens, attribute)?;
    constraint.path = append_constraint_path_suffix(&constraint.path, "[]");
    Ok(constraint)
}

pub fn parse_expr_list(tokens: proc_macro2::TokenStream) -> Result<Vec<Expr>> {
    Punctuated::<Expr, Token![,]>::parse_terminated
        .parse2(tokens)
        .map(|items| items.into_iter().collect())
}

pub fn parse_lit_str_list(tokens: proc_macro2::TokenStream) -> Result<Vec<LitStr>> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    items
        .iter()
        .map(|expr| expr_lit_str(expr, "path"))
        .collect()
}

pub fn parse_item_lit_str_list(tokens: proc_macro2::TokenStream) -> Result<Vec<LitStr>> {
    parse_lit_str_list(tokens).map(|items| {
        items
            .into_iter()
            .map(|path| append_constraint_path_suffix(&path, "[]"))
            .collect()
    })
}

pub fn parse_path_usize_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathUsizeConstraint> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut iter = items.into_iter();
    let Some(path_expr) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires a path string and usize value"),
        ));
    };
    let Some(value_expr) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires a path string and usize value"),
        ));
    };
    if iter.next().is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} accepts exactly two arguments"),
        ));
    }
    Ok(PathUsizeConstraint {
        path: expr_lit_str(&path_expr, attribute)?,
        value: expr_lit_usize(&value_expr, attribute)?,
    })
}

pub fn parse_path_pair_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathPairConstraint> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut iter = items.into_iter();
    let Some(left_expr) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires exactly two path strings"),
        ));
    };
    let Some(right_expr) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires exactly two path strings"),
        ));
    };
    if iter.next().is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires exactly two path strings"),
        ));
    }
    Ok(PathPairConstraint {
        left: expr_lit_str(&left_expr, attribute)?,
        right: expr_lit_str(&right_expr, attribute)?,
    })
}

pub fn expr_lit_str(expr: &Expr, field: &str) -> Result<LitStr> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) => Ok(value.clone()),
        _ => Err(syn::Error::new_spanned(
            expr,
            format!("{field} must be a string literal"),
        )),
    }
}

pub fn expr_string_like(expr: &Expr, field: &str) -> Result<LitStr> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) => Ok(value.clone()),
        Expr::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => Ok(
            LitStr::new(&path.path.segments[0].ident.to_string(), path.span()),
        ),
        _ => Err(syn::Error::new_spanned(
            expr,
            format!("{field} must be a string literal or bare identifier"),
        )),
    }
}

pub fn expr_lit_bool(expr: &Expr, field: &str) -> Result<bool> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Bool(LitBool { value, .. }),
            ..
        }) => Ok(*value),
        _ => Err(syn::Error::new_spanned(
            expr,
            format!("{field} must be a bool literal"),
        )),
    }
}

pub fn expr_lit_usize(expr: &Expr, field: &str) -> Result<usize> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(value),
            ..
        }) => value.base10_parse::<usize>().map_err(|err| {
            syn::Error::new_spanned(expr, format!("{field} must be a usize literal: {err}"))
        }),
        _ => Err(syn::Error::new_spanned(
            expr,
            format!("{field} must be a usize literal"),
        )),
    }
}

pub fn expr_array_values(expr: &Expr, field: &str) -> Result<Vec<Expr>> {
    let Expr::Array(array) = expr else {
        return Err(syn::Error::new_spanned(
            expr,
            format!("{field} must be an array literal"),
        ));
    };
    if array.elems.is_empty() {
        return Err(syn::Error::new_spanned(
            expr,
            format!("{field} must include at least one value"),
        ));
    }
    Ok(array.elems.iter().cloned().collect())
}

pub fn expr_array_lit_strs(expr: &Expr, field: &str) -> Result<Vec<LitStr>> {
    expr_array_values(expr, field)?
        .iter()
        .map(|item| expr_lit_str(item, field))
        .collect()
}

pub fn expr_lit_i32(expr: &Expr, field: &str) -> Result<i32> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(value),
            ..
        }) => value.base10_parse::<i32>().map_err(|err| {
            syn::Error::new_spanned(expr, format!("{field} must be an i32 literal: {err}"))
        }),
        Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => {
            let Expr::Lit(ExprLit {
                lit: Lit::Int(value),
                ..
            }) = unary.expr.as_ref()
            else {
                return Err(syn::Error::new_spanned(
                    expr,
                    format!("{field} must be an i32 literal"),
                ));
            };
            let value = value.base10_parse::<i32>().map_err(|err| {
                syn::Error::new_spanned(expr, format!("{field} must be an i32 literal: {err}"))
            })?;
            value
                .checked_neg()
                .ok_or_else(|| syn::Error::new_spanned(expr, format!("{field} is below i32::MIN")))
        }
        _ => Err(syn::Error::new_spanned(
            expr,
            format!("{field} must be an i32 literal"),
        )),
    }
}

pub fn expr_path(expr: &Expr, field: &str) -> Result<syn::Path> {
    match expr {
        Expr::Path(ExprPath { path, .. }) => Ok(path.clone()),
        _ => Err(syn::Error::new_spanned(
            expr,
            format!("{field} must be a path"),
        )),
    }
}

pub fn doc_text(attrs: &[Attribute]) -> Option<String> {
    let mut lines = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        let Meta::NameValue(value) = &attr.meta else {
            continue;
        };
        if let Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) = &value.value
        {
            lines.push(value.value().trim().to_string());
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(normalize_doc_lines(&lines))
}

fn normalize_doc_lines(lines: &[String]) -> String {
    let mut output = String::new();
    let mut previous_blank = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !output.is_empty() && !previous_blank {
                output.push('\n');
            }
            previous_blank = true;
            continue;
        }
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(trimmed);
        previous_blank = false;
    }
    output.trim().to_string()
}

pub fn doc_summary(doc: Option<&str>) -> Option<String> {
    let doc = doc?.trim();
    if doc.is_empty() {
        return None;
    }
    let first_paragraph = doc.split("\n\n").next()?.trim();
    if first_paragraph.is_empty() {
        return None;
    }
    Some(
        first_paragraph
            .lines()
            .map(str::trim)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

pub fn lit_str_from_text(text: Option<&str>) -> Option<LitStr> {
    text.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| LitStr::new(value, proc_macro2::Span::call_site()))
}
