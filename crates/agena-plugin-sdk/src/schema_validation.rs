pub fn hostname_format_is_valid(text: &str) -> bool {
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
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
    })
}

pub fn email_format_is_valid(text: &str) -> bool {
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
    let local_valid = local.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || matches!(
                character,
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

pub fn format_is_valid(format: &str, text: &str) -> bool {
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

pub fn validate_regex_pattern(pattern: &str) -> Result<(), regex::Error> {
    regex::Regex::new(pattern).map(|_| ())
}

pub fn pattern_matches(pattern: &str, text: &str) -> Result<bool, regex::Error> {
    regex::Regex::new(pattern).map(|expression| expression.is_match(text))
}

#[cfg(test)]
mod tests {
    use super::{
        email_format_is_valid, format_is_valid, hostname_format_is_valid, pattern_matches,
    };

    #[test]
    fn validates_shared_schema_formats_and_patterns() {
        assert!(hostname_format_is_valid("example.com"));
        assert!(email_format_is_valid("person@example.com"));
        assert!(format_is_valid(
            "uuid",
            "00000000-0000-0000-0000-000000000000"
        ));
        assert!(pattern_matches("^item$", "item").unwrap());
    }
}
