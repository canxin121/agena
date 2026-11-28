use super::{commands, is_separator};

/// Attribute a simple shell preview to a single literal file operand.
/// This is for context discovery after a successful read, independent of
/// shell authorization and the general command classifier.
pub fn single_file_read(command: &str) -> Option<String> {
    let command = command.trim();
    if commands::has_uninspected_syntax(command, true) {
        return None;
    }
    let tokens = commands::tokens(command);
    if tokens.iter().any(|token| is_separator(token)) {
        return None;
    }
    let (program, args) = tokens.split_first()?;
    let program = commands::executable_name(program);
    let file = match program.as_str() {
        "cat" => {
            let parsed = operands(
                args,
                "AbEnsTtuv",
                "",
                &[
                    "show-all",
                    "number-nonblank",
                    "show-ends",
                    "number",
                    "squeeze-blank",
                    "show-tabs",
                    "show-nonprinting",
                ],
                &[],
            )?;
            one_file(&parsed.files)?
        }
        "nl" => {
            let parsed = operands(
                args,
                "p",
                "bdfhilnsvw",
                &["no-renumber"],
                &[
                    "body-numbering",
                    "section-delimiter",
                    "footer-numbering",
                    "header-numbering",
                    "page-increment",
                    "join-blank-lines",
                    "number-format",
                    "number-separator",
                    "starting-line-number",
                    "number-width",
                ],
            )?;
            one_file(&parsed.files)?
        }
        "head" | "tail" => {
            // Both the modern -n form and the traditional -10 form are common.
            let parsed = operands(
                args,
                "qv",
                "nc",
                &["quiet", "silent", "verbose"],
                &["lines", "bytes"],
            )?;
            one_file(&parsed.files)?
        }
        "bat" | "batcat" => {
            let parsed = operands(
                args,
                "pPAn",
                "r",
                &["plain", "number", "show-all"],
                &[
                    "line-range",
                    "color",
                    "paging",
                    "style",
                    "language",
                    "highlight-line",
                ],
            )?;
            one_file(&parsed.files)?
        }
        "sed" => {
            let parsed = operands(
                args,
                "nEr",
                "",
                &["quiet", "silent", "regexp-extended"],
                &[],
            )?;
            let [script, path] = parsed.files.as_slice() else {
                return None;
            };
            if !parsed
                .options
                .iter()
                .any(|option| matches!(*option, "n" | "quiet" | "silent"))
                || !script.strip_suffix('p').is_some_and(line_range)
            {
                return None;
            }
            *path
        }
        "grep" | "egrep" | "fgrep" | "rg" => {
            let flags = if program == "rg" {
                "EinHFbowsxFsSUaP"
            } else {
                "EinHFbhowvxsFaP"
            };
            let parsed = operands(
                args,
                flags,
                "eABCm",
                &[
                    "extended-regexp",
                    "fixed-strings",
                    "ignore-case",
                    "line-number",
                    "with-filename",
                    "no-filename",
                    "only-matching",
                    "word-regexp",
                    "line-regexp",
                    "invert-match",
                    "byte-offset",
                    "text",
                    "no-heading",
                    "heading",
                    "smart-case",
                    "pcre2",
                    "case-sensitive",
                    "multiline",
                    "multiline-dotall",
                    "no-config",
                ],
                &[
                    "regexp",
                    "after-context",
                    "before-context",
                    "context",
                    "max-count",
                    "color",
                ],
            )?;
            if parsed
                .options
                .iter()
                .any(|option| matches!(*option, "e" | "regexp"))
            {
                one_file(&parsed.files)?
            } else {
                let [_pattern, path] = parsed.files.as_slice() else {
                    return None;
                };
                *path
            }
        }
        _ => return None,
    };
    (!file.is_empty() && file != "-").then(|| file.to_owned())
}

fn one_file<'a>(files: &[&'a str]) -> Option<&'a str> {
    match files {
        [file] => Some(*file),
        _ => None,
    }
}

fn line_range(script: &str) -> bool {
    fn line(value: &str) -> bool {
        value == "$" || (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
    }
    match script.split_once(',') {
        Some((start, end)) => line(start) && line(end),
        None => line(script),
    }
}

#[derive(Default)]
struct Operands<'a> {
    files: Vec<&'a str>,
    options: Vec<&'a str>,
}

fn operands<'a>(
    args: &'a [String],
    short: &str,
    short_value: &str,
    long: &[&str],
    long_value: &[&str],
) -> Option<Operands<'a>> {
    let mut parsed = Operands::default();
    let mut args = args.iter().map(String::as_str);
    while let Some(arg) = args.next() {
        if arg == "--" {
            parsed.files.extend(args);
            break;
        }
        if let Some(arg) = arg.strip_prefix("--") {
            let name = arg.split_once('=').map_or(arg, |(name, _)| name);
            if long_value.contains(&name) {
                if arg == name {
                    args.next()?;
                }
            } else if arg != name || !long.contains(&name) {
                return None;
            }
            parsed.options.push(name);
        } else if let Some(flags) = arg.strip_prefix('-').filter(|flags| !flags.is_empty()) {
            if short_value == "nc" && flags.bytes().all(|byte| byte.is_ascii_digit()) {
                continue;
            }
            for (index, flag) in flags.char_indices() {
                let end = index + flag.len_utf8();
                parsed.options.push(&flags[index..end]);
                if short_value.contains(flag) {
                    if end == flags.len() {
                        args.next()?;
                    }
                    break;
                }
                if !short.contains(flag) {
                    return None;
                }
            }
        } else {
            parsed.files.push(arg);
        }
    }
    Some(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_attributes_single_file_text_previews() {
        for (command, path) in [
            ("cat -n -- './some [dir]/a b.txt'", "./some [dir]/a b.txt"),
            ("/bin/nl -ba 'a b.txt'", "a b.txt"),
            ("batcat --line-range=2:4 a.rs", "a.rs"),
            ("head -10 example.md", "example.md"),
            ("tail -n2 -- example.md", "example.md"),
            ("sed -n '2,4p' example.md", "example.md"),
            ("grep -m 1 -n -- pattern a.txt", "a.txt"),
            ("rg --regexp='a pattern' -- a.txt", "a.txt"),
        ] {
            assert_eq!(
                single_file_read(command).as_deref(),
                Some(path),
                "{command}"
            );
        }
        for command in [
            "cat a.txt b.txt",
            "cat a.txt >/dev/null",
            "cat <a.txt",
            "cat a.txt | head",
            "cat *.txt",
            "cat \"$FILE\"",
            "cat ~/a.txt",
            "cat a.txt; cat b.txt",
            "cat -",
            "sed -i '2,4p' a.txt",
            "rg --files a.txt",
            "grep -l pattern a.txt",
            "env -C sub cat a.txt",
        ] {
            assert_eq!(single_file_read(command), None, "{command}");
        }
    }
}
