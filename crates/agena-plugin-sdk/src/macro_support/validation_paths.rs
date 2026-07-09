use std::borrow::ToOwned;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::net::{Ipv4Addr, Ipv6Addr};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::{PluginError, Result};

use super::{normalize_flattened_input_object, suggest_name_candidates, unknown_name_message};

pub fn normalize_trim_paths(input: &mut Value, paths: &[&str]) {
    for path in paths {
        let segments = parse_json_path(path);
        mutate_json_path_strings(input, &segments, &mut |text| {
            *text = text.trim().to_string();
        });
    }
}

pub fn normalize_trim_suffix_path(input: &mut Value, path: &str, suffix: &str) {
    let segments = parse_json_path(path);
    mutate_json_path_strings(input, &segments, &mut |text| {
        if let Some(stripped) = text.strip_suffix(suffix) {
            *text = stripped.to_string();
        }
    });
}

pub fn remove_json_path(root: &mut Value, path: &str) {
    let segments = parse_json_path(path);
    remove_json_path_matches(root, &segments);
}

pub fn normalize_nested_input_path(input: &mut Value, path: &str, schema: &Value) {
    let segments = parse_json_path(path);
    normalize_nested_input_matches(input, &segments, schema);
}

pub fn prefix_input_jsonpath(prefix: &str, jsonpath: &str) -> Option<String> {
    if jsonpath == "$" {
        return Some(prefix.to_string());
    }
    let suffix = jsonpath.strip_prefix("$.")?;
    Some(format!("{prefix}.{suffix}"))
}

pub fn validate_non_empty_paths<T>(value: &T, paths: &[&str]) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for path in paths {
        let matches = json_path_matches(&json, path);
        if matches.is_empty() || matches.iter().any(|value| !value_present(value)) {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must not be empty",
                display_path(path)
            )));
        }
    }
    Ok(())
}

pub fn validate_non_empty_if_present_paths<T>(value: &T, paths: &[&str]) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for path in paths {
        let matches = json_path_matches(&json, path);
        if matches.iter().any(|value| !value_present(value)) {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must not be empty when present",
                display_path(path)
            )));
        }
    }
    Ok(())
}

pub fn validate_exactly_one_of_paths<T>(value: &T, paths: &[&str]) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    let present = paths
        .iter()
        .filter(|path| json_path_present(&json, path))
        .count();
    if present != 1 {
        return Err(PluginError::invalid_params(format!(
            "exactly one of {} is required",
            human_join_paths(paths)
        )));
    }
    Ok(())
}

pub fn validate_at_least_one_of_paths<T>(value: &T, paths: &[&str]) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    if !paths.iter().any(|path| json_path_present(&json, path)) {
        return Err(PluginError::invalid_params(format!(
            "at least one of {} is required",
            human_join_paths(paths)
        )));
    }
    Ok(())
}

pub fn validate_min_items_path<T>(value: &T, path: &str, minimum: usize) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    let matches = json_path_matches(&json, path);
    if matches.is_empty() || matches.iter().any(|value| array_len(value) < minimum) {
        return Err(PluginError::invalid_params(format!(
            "field `{}` requires at least {minimum} item{}",
            display_path(path),
            if minimum == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

pub fn validate_max_items_path<T>(value: &T, path: &str, maximum: usize) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    if json_path_matches(&json, path)
        .iter()
        .any(|value| array_len(value) > maximum)
    {
        return Err(PluginError::invalid_params(format!(
            "field `{}` accepts at most {maximum} item{}",
            display_path(path),
            if maximum == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

pub fn validate_min_properties_path<T>(value: &T, path: &str, minimum: usize) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for candidate in json_path_matches(&json, path) {
        let Value::Object(object) = candidate else {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be an object",
                display_path(path)
            )));
        };
        if object.len() < minimum {
            return Err(PluginError::invalid_params(format!(
                "field `{}` requires at least {minimum} propert{}",
                display_path(path),
                if minimum == 1 { "y" } else { "ies" }
            )));
        }
    }
    Ok(())
}

pub fn validate_max_properties_path<T>(value: &T, path: &str, maximum: usize) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for candidate in json_path_matches(&json, path) {
        let Value::Object(object) = candidate else {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be an object",
                display_path(path)
            )));
        };
        if object.len() > maximum {
            return Err(PluginError::invalid_params(format!(
                "field `{}` accepts at most {maximum} propert{}",
                display_path(path),
                if maximum == 1 { "y" } else { "ies" }
            )));
        }
    }
    Ok(())
}

pub fn validate_min_chars_path<T>(value: &T, path: &str, minimum: usize) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    if json_path_matches(&json, path)
        .iter()
        .any(|value| string_char_count(value) < minimum)
    {
        return Err(PluginError::invalid_params(format!(
            "field `{}` must be at least {minimum} character{}",
            display_path(path),
            if minimum == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

pub fn validate_max_chars_path<T>(value: &T, path: &str, maximum: usize) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    if json_path_matches(&json, path)
        .iter()
        .any(|value| string_char_count(value) > maximum)
    {
        return Err(PluginError::invalid_params(format!(
            "field `{}` must be at most {maximum} character{}",
            display_path(path),
            if maximum == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

pub fn validate_minimum_path<T>(value: &T, path: &str, minimum: &Value) -> Result<()>
where
    T: Serialize,
{
    let Some(minimum_number) = minimum.as_number() else {
        return Err(PluginError::invalid_params(format!(
            "minimum for field `{}` must be numeric",
            display_path(path)
        )));
    };
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for candidate in json_path_matches(&json, path) {
        let Value::Number(candidate_number) = candidate else {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be a number",
                display_path(path)
            )));
        };
        if compare_json_numbers(candidate_number, minimum_number)
            .is_some_and(|ordering| ordering == Ordering::Less)
        {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be at least {}",
                display_path(path),
                minimum
            )));
        }
    }
    Ok(())
}

pub fn validate_maximum_path<T>(value: &T, path: &str, maximum: &Value) -> Result<()>
where
    T: Serialize,
{
    let Some(maximum_number) = maximum.as_number() else {
        return Err(PluginError::invalid_params(format!(
            "maximum for field `{}` must be numeric",
            display_path(path)
        )));
    };
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for candidate in json_path_matches(&json, path) {
        let Value::Number(candidate_number) = candidate else {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be a number",
                display_path(path)
            )));
        };
        if compare_json_numbers(candidate_number, maximum_number)
            .is_some_and(|ordering| ordering == Ordering::Greater)
        {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be at most {}",
                display_path(path),
                maximum
            )));
        }
    }
    Ok(())
}

pub fn validate_exclusive_minimum_path<T>(value: &T, path: &str, minimum: &Value) -> Result<()>
where
    T: Serialize,
{
    let Some(minimum_number) = minimum.as_number() else {
        return Err(PluginError::invalid_params(format!(
            "exclusive minimum for field `{}` must be numeric",
            display_path(path)
        )));
    };
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for candidate in json_path_matches(&json, path) {
        let Value::Number(candidate_number) = candidate else {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be a number",
                display_path(path)
            )));
        };
        if compare_json_numbers(candidate_number, minimum_number)
            .is_some_and(|ordering| ordering == Ordering::Less || ordering == Ordering::Equal)
        {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be greater than {}",
                display_path(path),
                minimum
            )));
        }
    }
    Ok(())
}

pub fn validate_exclusive_maximum_path<T>(value: &T, path: &str, maximum: &Value) -> Result<()>
where
    T: Serialize,
{
    let Some(maximum_number) = maximum.as_number() else {
        return Err(PluginError::invalid_params(format!(
            "exclusive maximum for field `{}` must be numeric",
            display_path(path)
        )));
    };
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for candidate in json_path_matches(&json, path) {
        let Value::Number(candidate_number) = candidate else {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be a number",
                display_path(path)
            )));
        };
        if compare_json_numbers(candidate_number, maximum_number)
            .is_some_and(|ordering| ordering == Ordering::Greater || ordering == Ordering::Equal)
        {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be less than {}",
                display_path(path),
                maximum
            )));
        }
    }
    Ok(())
}

pub fn validate_format_path<T>(value: &T, path: &str, format: &str) -> Result<()>
where
    T: Serialize,
{
    if !is_supported_string_format(format) {
        return Err(PluginError::invalid_params(format!(
            "unsupported format `{}` for field `{}`",
            format,
            display_path(path)
        )));
    }
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for candidate in json_path_matches(&json, path) {
        let Value::String(text) = candidate else {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be a string",
                display_path(path)
            )));
        };
        if !string_matches_format(text, format) {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must match format `{}`",
                display_path(path),
                format
            )));
        }
    }
    Ok(())
}

pub fn validate_pattern_path<T>(value: &T, path: &str, pattern: &str) -> Result<()>
where
    T: Serialize,
{
    let regex = regex::Regex::new(pattern).map_err(|err| {
        PluginError::invalid_params(format!(
            "invalid pattern for field `{}`: {err}",
            display_path(path)
        ))
    })?;
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for candidate in json_path_matches(&json, path) {
        let Value::String(text) = candidate else {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be a string",
                display_path(path)
            )));
        };
        if !regex.is_match(text) {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must match pattern `{}`",
                display_path(path),
                pattern
            )));
        }
    }
    Ok(())
}

fn is_supported_string_format(format: &str) -> bool {
    matches!(
        format,
        "uri" | "uuid" | "email" | "hostname" | "ipv4" | "ipv6"
    )
}

fn string_matches_format(text: &str, format: &str) -> bool {
    match format {
        "uri" => url::Url::parse(text).is_ok(),
        "uuid" => uuid::Uuid::parse_str(text).is_ok(),
        "email" => validate_email_text(text),
        "hostname" => validate_hostname_text(text),
        "ipv4" => text.parse::<Ipv4Addr>().is_ok(),
        "ipv6" => text.parse::<Ipv6Addr>().is_ok(),
        _ => false,
    }
}

fn validate_email_text(text: &str) -> bool {
    if text.is_empty() || text.len() > 254 || text.chars().any(char::is_whitespace) {
        return false;
    }
    let Some((local, domain)) = text.rsplit_once('@') else {
        return false;
    };
    if local.is_empty()
        || local.len() > 64
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
    {
        return false;
    }
    if !local.bytes().all(|byte| {
        matches!(byte,
            b'a'..=b'z'
            | b'A'..=b'Z'
            | b'0'..=b'9'
            | b'!'
            | b'#'
            | b'$'
            | b'%'
            | b'&'
            | b'\''
            | b'*'
            | b'+'
            | b'-'
            | b'/'
            | b'='
            | b'?'
            | b'^'
            | b'_'
            | b'`'
            | b'{'
            | b'|'
            | b'}'
            | b'~'
            | b'.')
    }) {
        return false;
    }
    if let Some(domain_literal) = domain
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        if let Some(ipv6) = domain_literal.strip_prefix("IPv6:") {
            return ipv6.parse::<Ipv6Addr>().is_ok();
        }
        return domain_literal.parse::<Ipv4Addr>().is_ok();
    }
    validate_hostname_text(domain)
}

fn validate_hostname_text(text: &str) -> bool {
    let hostname = text.strip_suffix('.').unwrap_or(text);
    if hostname.is_empty() || hostname.len() > 253 {
        return false;
    }
    hostname.split('.').all(validate_hostname_label)
}

fn validate_hostname_label(label: &str) -> bool {
    if label.is_empty() || label.len() > 63 {
        return false;
    }
    let bytes = label.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
}

pub fn validate_allowed_values_path<T>(value: &T, path: &str, allowed: &[Value]) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for candidate in json_path_matches(&json, path) {
        if allowed.iter().any(|value| value == candidate) {
            continue;
        }
        let allowed_json = serde_json::to_string(&Value::Array(allowed.to_vec()))
            .unwrap_or_else(|_| "[]".to_string());
        return Err(PluginError::invalid_params(format!(
            "field `{}` must be one of {}",
            display_path(path),
            allowed_json
        )));
    }
    Ok(())
}

pub fn validate_forbid_substrings_path<T>(value: &T, path: &str, forbidden: &[&str]) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for candidate in json_path_matches(&json, path) {
        let Value::String(text) = candidate else {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be a string",
                display_path(path)
            )));
        };
        if let Some(found) = forbidden.iter().find(|needle| text.contains(**needle)) {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must not contain `{}`",
                display_path(path),
                found
            )));
        }
    }
    Ok(())
}

pub fn validate_distinct_trimmed_path<T>(value: &T, path: &str) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    validate_distinct_trimmed_matches(json_path_matches(&json, path), path, None)
}

pub fn validate_distinct_trimmed_within_path<T>(
    value: &T,
    path: &str,
    scope_path: &str,
) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    let path_segments = parse_json_path(path);
    let scope_segments = parse_json_path(scope_path);
    let Some(suffix) = path_segments.strip_prefix(scope_segments.as_slice()) else {
        return Err(PluginError::invalid_params(format!(
            "field `{}` must be nested within `{}`",
            display_path(path),
            display_path(scope_path)
        )));
    };
    for scope_root in json_path_matches_segments(&json, &scope_segments) {
        validate_distinct_trimmed_matches(
            json_path_matches_segments(scope_root, suffix),
            path,
            Some(scope_path),
        )?;
    }
    Ok(())
}

pub fn validate_requires_path<T>(value: &T, path: &str, required_path: &str) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    let relation = parsed_relation_paths(path, required_path);
    for subroot in relation_subroots(&json, &relation) {
        if path_present_segments(subroot, &relation.left_suffix)
            && !path_present_segments(subroot, &relation.right_suffix)
        {
            return Err(PluginError::invalid_params(format!(
                "field `{}` requires `{}`",
                display_path(path),
                display_path(required_path)
            )));
        }
    }
    Ok(())
}

pub fn validate_conflicts_with_path<T>(value: &T, path: &str, other_path: &str) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    let relation = parsed_relation_paths(path, other_path);
    for subroot in relation_subroots(&json, &relation) {
        if path_present_segments(subroot, &relation.left_suffix)
            && path_present_segments(subroot, &relation.right_suffix)
        {
            return Err(PluginError::invalid_params(format!(
                "field `{}` conflicts with `{}`",
                display_path(path),
                display_path(other_path)
            )));
        }
    }
    Ok(())
}

pub fn validate_required_unless_present_path<T>(
    value: &T,
    path: &str,
    unless_path: &str,
) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    let relation = parsed_relation_paths(path, unless_path);
    for subroot in relation_subroots(&json, &relation) {
        if !path_present_segments(subroot, &relation.right_suffix)
            && !path_present_segments(subroot, &relation.left_suffix)
        {
            return Err(PluginError::invalid_params(format!(
                "field `{}` is required unless `{}` is present",
                display_path(path),
                display_path(unless_path)
            )));
        }
    }
    Ok(())
}

pub(crate) fn invalid_json_syntax_error(err: serde_json::Error, source: &str) -> PluginError {
    let detail = err.to_string();
    let message = format!("invalid JSON {source}: {detail}");
    PluginError::invalid_params_with_data(message, json_error_data(err, source, None))
}

pub(crate) fn invalid_json_data_error(
    err: serde_json::Error,
    source: &str,
    path: Option<String>,
) -> PluginError {
    let detail = err.to_string();
    let message = match path.as_deref().filter(|value| !value.is_empty()) {
        Some(path) => format!("invalid JSON {source} at `{path}`: {detail}"),
        None => format!("invalid JSON {source}: {detail}"),
    };
    PluginError::invalid_params_with_data(message, json_error_data(err, source, path))
}

pub(crate) fn json_error_data(
    err: serde_json::Error,
    source: &str,
    path: Option<String>,
) -> serde_json::Value {
    let category = match err.classify() {
        serde_json::error::Category::Io => "io",
        serde_json::error::Category::Syntax => "syntax",
        serde_json::error::Category::Data => "data",
        serde_json::error::Category::Eof => "eof",
    };
    let mut data = Map::new();
    data.insert("kind".into(), Value::String("json_input".into()));
    data.insert("source".into(), Value::String(source.to_string()));
    data.insert("category".into(), Value::String(category.to_string()));
    data.insert("detail".into(), Value::String(err.to_string()));
    if let Some(path) = path.filter(|value| !value.is_empty()) {
        data.insert("path".into(), Value::String(path));
    }
    if err.line() > 0 {
        data.insert("line".into(), Value::from(err.line() as u64));
    }
    if err.column() > 0 {
        data.insert("column".into(), Value::from(err.column() as u64));
    }
    Value::Object(data)
}

pub fn json_path_present(root: &Value, path: &str) -> bool {
    let segments = parse_json_path(path);
    path_present_segments(root, &segments)
}

fn value_present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(_) | Value::Number(_) => true,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(object) => !object.is_empty(),
    }
}

fn array_len(value: &Value) -> usize {
    match value {
        Value::Array(items) => items.len(),
        _ => 0,
    }
}

fn string_char_count(value: &Value) -> usize {
    match value {
        Value::String(text) => text.chars().count(),
        _ => 0,
    }
}

pub(crate) fn compare_json_numbers(
    left: &serde_json::Number,
    right: &serde_json::Number,
) -> Option<Ordering> {
    match (left.as_i64(), left.as_u64(), right.as_i64(), right.as_u64()) {
        (Some(left), _, Some(right), _) => return Some(left.cmp(&right)),
        (_, Some(left), _, Some(right)) => return Some(left.cmp(&right)),
        (Some(left), _, _, Some(right)) => {
            return Some(if left < 0 {
                Ordering::Less
            } else {
                (left as u64).cmp(&right)
            });
        }
        (_, Some(left), Some(right), _) => {
            return Some(if right < 0 {
                Ordering::Greater
            } else {
                left.cmp(&(right as u64))
            });
        }
        _ => {}
    }
    left.as_f64()?.partial_cmp(&right.as_f64()?)
}

fn json_path_matches<'a>(root: &'a Value, path: &str) -> Vec<&'a Value> {
    let segments = parse_json_path(path);
    json_path_matches_segments(root, &segments)
}

fn json_path_matches_segments<'a>(root: &'a Value, segments: &[JsonPathSegment]) -> Vec<&'a Value> {
    let mut matches = Vec::new();
    collect_json_path_matches(root, segments, &mut matches);
    matches
}

fn path_present_segments(root: &Value, segments: &[JsonPathSegment]) -> bool {
    json_path_matches_segments(root, segments)
        .iter()
        .any(|value| value_present(value))
}

fn relation_subroots<'a>(root: &'a Value, relation: &ParsedRelationPaths) -> Vec<&'a Value> {
    let prefix = relation.common_prefix.as_slice();
    if prefix.is_empty() {
        vec![root]
    } else {
        json_path_matches_segments(root, prefix)
    }
}

fn parsed_relation_paths(left: &str, right: &str) -> ParsedRelationPaths {
    let left_segments = parse_json_path(left);
    let right_segments = parse_json_path(right);
    let prefix_len = common_prefix_len(&left_segments, &right_segments);
    ParsedRelationPaths {
        common_prefix: left_segments[..prefix_len].to_vec(),
        left_suffix: left_segments[prefix_len..].to_vec(),
        right_suffix: right_segments[prefix_len..].to_vec(),
    }
}

fn collect_json_path_matches<'a>(
    current: &'a Value,
    segments: &[JsonPathSegment],
    matches: &mut Vec<&'a Value>,
) {
    if segments.is_empty() {
        matches.push(current);
        return;
    }

    match &segments[0] {
        JsonPathSegment::Key(key) => {
            if let Value::Object(object) = current
                && let Some(next) = object.get(key)
            {
                collect_json_path_matches(next, &segments[1..], matches);
            }
        }
        JsonPathSegment::AllItems => {
            if let Value::Array(items) = current {
                for item in items {
                    collect_json_path_matches(item, &segments[1..], matches);
                }
            }
        }
    }
}

fn remove_json_path_matches(current: &mut Value, segments: &[JsonPathSegment]) {
    let Some((head, tail)) = segments.split_first() else {
        return;
    };

    match head {
        JsonPathSegment::Key(key) => {
            let Value::Object(object) = current else {
                return;
            };
            if tail.is_empty() {
                object.remove(key);
            } else if let Some(next) = object.get_mut(key) {
                remove_json_path_matches(next, tail);
            }
        }
        JsonPathSegment::AllItems => {
            let Value::Array(items) = current else {
                return;
            };
            if tail.is_empty() {
                items.clear();
            } else {
                for item in items {
                    remove_json_path_matches(item, tail);
                }
            }
        }
    }
}

fn normalize_nested_input_matches(
    current: &mut Value,
    segments: &[JsonPathSegment],
    schema: &Value,
) {
    if segments.is_empty() {
        normalize_flattened_input_object(current, schema);
        return;
    }

    match &segments[0] {
        JsonPathSegment::Key(key) => {
            if let Value::Object(object) = current
                && let Some(next) = object.get_mut(key)
            {
                normalize_nested_input_matches(next, &segments[1..], schema);
            }
        }
        JsonPathSegment::AllItems => {
            if let Value::Array(items) = current {
                for item in items {
                    normalize_nested_input_matches(item, &segments[1..], schema);
                }
            }
        }
    }
}

pub(crate) fn normalized_name_distance(left: &str, right: &str) -> usize {
    let left = left.trim().to_ascii_lowercase();
    let right = right.trim().to_ascii_lowercase();
    if left == right {
        return 0;
    }
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut prev = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut curr = vec![0; right_chars.len() + 1];
    for (i, left_ch) in left_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, right_ch) in right_chars.iter().enumerate() {
            let replace = prev[j] + usize::from(left_ch != right_ch);
            let insert = curr[j] + 1;
            let delete = prev[j + 1] + 1;
            curr[j + 1] = replace.min(insert.min(delete));
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[right_chars.len()]
}

pub(crate) fn unknown_field_from_error_detail(detail: &str) -> Option<String> {
    let prefix = "unknown field `";
    let start = detail.find(prefix)? + prefix.len();
    let rest = &detail[start..];
    let end = rest.find('`')?;
    Some(rest[..end].to_string())
}

pub(crate) fn schema_field_candidates(schema: &Value) -> Vec<String> {
    let mut candidates = Vec::new();
    collect_schema_field_candidates(schema, &mut candidates);
    candidates.sort();
    candidates.dedup();
    candidates
}

pub(crate) fn reject_unknown_object_fields(
    input: &Value,
    schema: &Value,
    kind: &str,
) -> Result<()> {
    let Some(object) = input.as_object() else {
        return Ok(());
    };
    if !schema_denies_unknown_properties(schema) {
        return Ok(());
    }
    let candidates = schema_field_candidates(schema);
    if candidates.is_empty() {
        return Ok(());
    }
    let candidate_set = candidates.iter().collect::<HashSet<_>>();
    for key in object.keys() {
        if candidate_set.contains(key) {
            continue;
        }
        let suggestions = suggest_name_candidates(key, candidates.iter(), 1);
        let message = if suggestions.is_empty() {
            format!("unknown {kind} '{key}'")
        } else {
            unknown_name_message(kind, key, &suggestions)
        };
        return Err(PluginError::invalid_params(message));
    }
    Ok(())
}

fn schema_denies_unknown_properties(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    if object
        .get("additionalProperties")
        .is_some_and(|value| matches!(value, Value::Bool(false)))
    {
        return true;
    }
    if object
        .get("unevaluatedProperties")
        .is_some_and(|value| matches!(value, Value::Bool(false)))
    {
        return true;
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(Value::Array(items)) = object.get(key)
            && !items.is_empty()
            && items.iter().all(schema_denies_unknown_properties)
        {
            return true;
        }
    }
    false
}

fn collect_schema_field_candidates(schema: &Value, candidates: &mut Vec<String>) {
    let Some(object) = schema.as_object() else {
        return;
    };
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, property_schema) in properties {
            candidates.push(name.clone());
            if let Some(Value::Array(aliases)) = property_schema.get("x-agena-aliases") {
                candidates.extend(
                    aliases
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned),
                );
            }
            collect_schema_field_candidates(property_schema, candidates);
        }
    }
    if let Some(Value::Array(aliases)) = object.get("x-agena-aliases") {
        candidates.extend(
            aliases
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned),
        );
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(Value::Array(items)) = object.get(key) {
            for item in items {
                collect_schema_field_candidates(item, candidates);
            }
        }
    }
    if let Some(items) = object.get("items") {
        collect_schema_field_candidates(items, candidates);
    }
}

fn mutate_json_path_strings<F>(current: &mut Value, segments: &[JsonPathSegment], f: &mut F)
where
    F: FnMut(&mut String),
{
    if segments.is_empty() {
        if let Value::String(text) = current {
            f(text);
        }
        return;
    }

    match &segments[0] {
        JsonPathSegment::Key(key) => {
            if let Value::Object(object) = current
                && let Some(next) = object.get_mut(key)
            {
                mutate_json_path_strings(next, &segments[1..], f);
            }
        }
        JsonPathSegment::AllItems => {
            if let Value::Array(items) = current {
                for item in items {
                    mutate_json_path_strings(item, &segments[1..], f);
                }
            }
        }
    }
}

fn validate_distinct_trimmed_matches(
    matches: Vec<&Value>,
    path: &str,
    scope_path: Option<&str>,
) -> Result<()> {
    let mut seen = HashSet::new();
    for candidate in matches {
        let Value::String(text) = candidate else {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be a string",
                display_path(path)
            )));
        };
        let trimmed = text.trim();
        if !seen.insert(trimmed.to_string()) {
            let message = match scope_path {
                Some(scope) => format!(
                    "field `{}` must not contain duplicate values within `{}`",
                    display_path(path),
                    display_path(scope)
                ),
                None => format!(
                    "field `{}` must not contain duplicate values",
                    display_path(path)
                ),
            };
            return Err(PluginError::invalid_params(message));
        }
    }
    Ok(())
}

fn parse_json_path(path: &str) -> Vec<JsonPathSegment> {
    let mut segments = Vec::new();
    for segment in path.split('.') {
        if let Some(key) = segment.strip_suffix("[]") {
            if !key.is_empty() {
                segments.push(JsonPathSegment::Key(key.to_string()));
            }
            segments.push(JsonPathSegment::AllItems);
        } else if !segment.is_empty() {
            segments.push(JsonPathSegment::Key(segment.to_string()));
        }
    }
    segments
}

#[derive(Clone, PartialEq, Eq)]
enum JsonPathSegment {
    Key(String),
    AllItems,
}

struct ParsedRelationPaths {
    common_prefix: Vec<JsonPathSegment>,
    left_suffix: Vec<JsonPathSegment>,
    right_suffix: Vec<JsonPathSegment>,
}

fn common_prefix_len(left: &[JsonPathSegment], right: &[JsonPathSegment]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(a, b)| a == b)
        .count()
}

fn human_join_paths(paths: &[&str]) -> String {
    paths
        .iter()
        .map(|path| format!("`{}`", display_path(path)))
        .collect::<Vec<_>>()
        .join(" or ")
}

fn display_path(path: &str) -> &str {
    path.strip_prefix("args.").unwrap_or(path)
}
