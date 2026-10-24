//! Non-evaluating expansion of POSIX `ENV` startup paths.
//!
//! An interactive POSIX `sh` locates its startup file through `ENV`, whose value
//! is ordinary shell text. Agena never evaluates that text: this module accepts
//! only the literal forms a human actually writes, and everything else is
//! refused so the capture script can decide not to source a file instead of
//! executing something unexpected.
//!
//! Accepted forms: an absolute path, `~`/`~/...`, `$NAME`, `${NAME}`,
//! `${NAME:-default}`, each optionally followed by `/...` path segments.
//! Refused: command substitution, backticks, quotes, globs, redirections,
//! whitespace-separated lists, `~user`, `..` traversal, and any other form of
//! parameter expansion.

use std::path::Path;

/// Why an `ENV` value was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvPathError {
    /// The value uses shell syntax this module refuses to interpret.
    Unsupported,
    /// The value needs an environment variable that is not set.
    MissingVariable(String),
    /// The value contains whitespace and therefore names more than one token.
    MultipleTokens,
    /// The expanded path is not absolute.
    NotAbsolute,
}

impl EnvPathError {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported shell expression",
            Self::MissingVariable(_) => "references an environment variable that is not set",
            Self::MultipleTokens => "names more than one token",
            Self::NotAbsolute => "does not expand to an absolute path",
        }
    }
}

/// Expand one `ENV` value without evaluating any shell code.
///
/// `lookup` reads environment variables (injected for tests). The returned path
/// is textually expanded but not canonicalized; callers still verify that it is
/// a readable regular file before sourcing it.
pub fn expand_env_path(
    value: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<String, EnvPathError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(EnvPathError::Unsupported);
    }
    if value.chars().any(|character| character.is_whitespace()) {
        return Err(EnvPathError::MultipleTokens);
    }

    let (head, rest) = split_head(value)?;

    let expanded_head = if head == "~" {
        lookup("HOME").ok_or_else(|| EnvPathError::MissingVariable("HOME".to_string()))?
    } else if let Some(name) = head.strip_prefix('$') {
        expand_variable(name, lookup)?
    } else {
        head.to_string()
    };

    let joined = match rest {
        Some(rest) => format!("{expanded_head}/{rest}"),
        None => expanded_head,
    };
    validate_expanded_path(&joined)?;
    Ok(joined)
}

/// Split a value into its expandable head and an optional `/`-separated tail.
///
/// A `${...}` head is closed by its brace, so defaults containing `/` stay part
/// of the head; a bare `$NAME` head ends at the first `/`.
fn split_head(value: &str) -> Result<(&str, Option<&str>), EnvPathError> {
    if let Some(body) = value.strip_prefix("${") {
        let close = body.find('}').ok_or(EnvPathError::Unsupported)?;
        let head_len = 2 + close + 1;
        let head = &value[..head_len];
        let after = &value[head_len..];
        if after.is_empty() {
            return Ok((head, None));
        }
        let rest = after.strip_prefix('/').ok_or(EnvPathError::Unsupported)?;
        return Ok((head, Some(rest)));
    }
    if let Some(body) = value.strip_prefix('$') {
        let name_len = body.find('/').unwrap_or(body.len());
        let head_len = 1 + name_len;
        let head = &value[..head_len];
        let after = &value[head_len..];
        if after.is_empty() {
            return Ok((head, None));
        }
        let rest = after.strip_prefix('/').ok_or(EnvPathError::Unsupported)?;
        return Ok((head, Some(rest)));
    }
    match value.split_once('/') {
        Some((head, rest)) => Ok((head, Some(rest))),
        None => Ok((value, None)),
    }
}

/// Expand `$NAME`, `${NAME}`, or `${NAME:-default}` at the start of a value.
fn expand_variable(
    name: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<String, EnvPathError> {
    if let Some(body) = name.strip_prefix('{') {
        let body = body.strip_suffix('}').ok_or(EnvPathError::Unsupported)?;
        let (name, default) = match body.split_once(":-") {
            Some((name, default)) => (name, Some(default)),
            None => (body, None),
        };
        validate_name(name)?;
        match lookup(name) {
            Some(value) if !value.is_empty() => Ok(value),
            _ => match default {
                Some(default) => {
                    validate_literal(default)?;
                    Ok(default.to_string())
                }
                None if lookup(name).is_some() => Ok(String::new()),
                None => Err(EnvPathError::MissingVariable(name.to_string())),
            },
        }
    } else {
        validate_name(name)?;
        // A bare `$NAME` takes no default, so an unset name is an error.
        lookup(name)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| EnvPathError::MissingVariable(name.to_string()))
    }
}

fn validate_name(name: &str) -> Result<(), EnvPathError> {
    let mut characters = name.chars();
    let start_ok = characters
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic());
    let rest_ok = characters.all(|character| character == '_' || character.is_ascii_alphanumeric());
    if start_ok && rest_ok {
        Ok(())
    } else {
        Err(EnvPathError::Unsupported)
    }
}

/// Accept only literal text: no further expansion, substitution, or quoting.
fn validate_literal(value: &str) -> Result<(), EnvPathError> {
    if value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | '+' | '/')
    }) {
        Ok(())
    } else {
        Err(EnvPathError::Unsupported)
    }
}

fn validate_expanded_path(path: &str) -> Result<(), EnvPathError> {
    if !path.starts_with('/') {
        return Err(EnvPathError::NotAbsolute);
    }
    if Path::new(path)
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(EnvPathError::Unsupported);
    }
    validate_literal(path)
}
