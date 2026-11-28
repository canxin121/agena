use std::collections::HashMap;

/// Parse Bash's literal assignment forms without executing shell code.
pub(super) fn parse(input: &str) -> Option<HashMap<String, String>> {
    let mut assignments = HashMap::new();
    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let assignment = line
            .strip_prefix("export ")
            .or_else(|| line.strip_prefix("declare -x "))
            .map(str::trim_start)
            .unwrap_or(line);
        let (name, value) = assignment.split_once('=')?;
        let mut bytes = name.bytes();
        if !bytes
            .next()
            .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
            || !bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
        {
            return None;
        }
        assignments.insert(name.to_owned(), literal(value)?);
    }
    Some(assignments)
}

fn literal(input: &str) -> Option<String> {
    let mut chars = input.chars();
    let mut value = String::new();
    while let Some(character) = chars.next() {
        match character {
            '\'' => loop {
                match chars.next()? {
                    '\'' => break,
                    '\0' => return None,
                    character => value.push(character),
                }
            },
            '"' => loop {
                match chars.next()? {
                    '"' => break,
                    '$' | '`' | '\0' => return None,
                    '\\' => {
                        let escaped = chars.next()?;
                        match escaped {
                            '$' | '`' | '"' | '\\' => value.push(escaped),
                            '\0' => return None,
                            _ => {
                                value.push('\\');
                                value.push(escaped);
                            }
                        }
                    }
                    character => value.push(character),
                }
            },
            character if character.is_ascii_alphanumeric() || "_@%+=:,./-".contains(character) => {
                value.push(character)
            }
            _ => return None,
        }
    }
    Some(value)
}

#[cfg(test)]
#[test]
fn powershell_projection_accepts_literals_and_drops_the_file_on_shell_code() {
    let variables = parse(
        r#"
# session setup
export A="hello world"
declare -x B=plain' + '"\$literal"
C=one
C=two
EMPTY=
"#,
    )
    .unwrap();
    assert_eq!(variables["A"], "hello world");
    assert_eq!(variables["B"], "plain + $literal");
    assert_eq!(variables["C"], "two");
    assert!(variables["EMPTY"].is_empty());
    for script in [
        "export PATH=\"$PATH:tools\"",
        "source config.sh",
        "A=$(pwd)",
        "A='unclosed",
        "A=one echo two",
    ] {
        assert!(parse(&format!("A=works\n{script}")).is_none(), "{script}");
    }
}
