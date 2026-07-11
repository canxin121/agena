use super::super::{JsonValue, Regex};

pub(in crate::app) fn hostname_format_is_valid(text: &str) -> bool {
    let text = text.trim_end_matches('.');
    if text.is_empty()
        || text.len() > 253
        || text.contains('/')
        || text.chars().any(char::is_whitespace)
    {
        return false;
    }
    text.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|char| char.is_ascii_alphanumeric() || char == '-')
    })
}

pub(in crate::app) fn email_format_is_valid(text: &str) -> bool {
    let Some((local, domain)) = text.split_once('@') else {
        return false;
    };
    if local.is_empty()
        || domain.is_empty()
        || text.chars().any(char::is_whitespace)
        || text.matches('@').count() != 1
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
    {
        return false;
    }
    let local_valid = local.chars().all(|char| {
        char.is_ascii_alphanumeric()
            || matches!(
                char,
                '!' | '#'
                    | '$'
                    | '%'
                    | '&'
                    | '\''
                    | '*'
                    | '+'
                    | '-'
                    | '/'
                    | '='
                    | '?'
                    | '^'
                    | '_'
                    | '`'
                    | '{'
                    | '|'
                    | '}'
                    | '~'
                    | '.'
            )
    });
    local_valid && (hostname_format_is_valid(domain) || domain.eq_ignore_ascii_case("localhost"))
}

pub(in crate::app) fn format_is_valid(format: &str, text: &str) -> bool {
    match format {
        "uri" | "url" => url::Url::parse(text).is_ok(),
        "email" => email_format_is_valid(text),
        "hostname" => hostname_format_is_valid(text),
        "ipv4" => text.parse::<std::net::Ipv4Addr>().is_ok(),
        "ipv6" => text.parse::<std::net::Ipv6Addr>().is_ok(),
        "uuid" => uuid::Uuid::parse_str(text).is_ok(),
        _ => true,
    }
}

pub(in crate::app) fn validate_regex_pattern(pattern: &str) -> Result<(), regex::Error> {
    Regex::new(pattern).map(|_| ())
}

pub(in crate::app) fn pattern_matches(pattern: &str, text: &str) -> Result<bool, regex::Error> {
    Regex::new(pattern).map(|regex| regex.is_match(text))
}

pub(in crate::app) fn merge_multi_enum_selection(
    current: &[JsonValue],
    selected: &[JsonValue],
) -> Vec<JsonValue> {
    let mut values = current
        .iter()
        .filter(|value| {
            selected
                .iter()
                .any(|selected_value| selected_value == *value)
        })
        .cloned()
        .collect::<Vec<_>>();
    for selected_value in selected {
        if !values.iter().any(|value| value == selected_value) {
            values.push(selected_value.clone());
        }
    }
    values
}
