use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use pulldown_cmark::{Event, Parser, Tag};
use std::{borrow::Cow, ops::Range, path::Path};

const MAX_EXPANDED_GLOBS: usize = 1000;
const MAX_EXPANDED_BYTES: usize = 4 * 1024 * 1024;

/// A partially read YAML header might contain a paths filter outside our
/// read budget. Defer that rule instead of accidentally loading it eagerly.
pub(super) fn rule_body(text: &str, truncated: bool) -> Option<(Option<Vec<String>>, &str)> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.split_inclusive('\n');
    let Some(opening) = lines.next().filter(|line| line.trim_end() == "---") else {
        return Some((None, text));
    };
    let mut offset = opening.len();
    for line in lines {
        if line.trim_end() == "---" {
            let yaml = serde_yaml::from_str::<serde_yaml::Value>(&text[opening.len()..offset]).ok();
            let paths = yaml
                .as_ref()
                .and_then(|yaml| yaml.get("paths"))
                .and_then(|paths| match paths {
                    serde_yaml::Value::Sequence(paths) => {
                        Some(paths.iter().filter_map(serde_yaml::Value::as_str).collect())
                    }
                    serde_yaml::Value::String(paths) => Some(split_globs(paths)),
                    _ => None,
                })
                .map(|paths| {
                    paths
                        .into_iter()
                        .map(str::trim)
                        .filter(|path| !path.is_empty())
                        .map(str::to_owned)
                        .collect()
                });
            return Some((paths, &text[offset + line.len()..]));
        }
        offset += line.len();
    }
    (!truncated).then_some((None, text))
}

fn split_globs(value: &str) -> Vec<&str> {
    let mut globs = Vec::new();
    let mut start = 0;
    let mut braces = 0usize;
    for (index, ch) in glob_delimiters(value) {
        match ch {
            '{' if value[index + 1..].contains('}') => braces += 1,
            '}' => braces = braces.saturating_sub(1),
            ',' if braces == 0 => {
                globs.push(&value[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    globs.push(&value[start..]);
    globs
}

fn glob_delimiters(value: &str) -> impl Iterator<Item = (usize, char)> + '_ {
    let mut brackets = false;
    let mut escaped = false;
    value.char_indices().filter(move |&(index, ch)| {
        if std::mem::take(&mut escaped) {
            return false;
        }
        match ch {
            '\\' => escaped = true,
            '[' if value[index + 1..].contains(']') => brackets = true,
            ']' => brackets = false,
            '{' | '}' | ',' => return !brackets,
            _ => (),
        }
        false
    })
}

fn first_brace_group(value: &str) -> Result<Option<Range<usize>>, ()> {
    let mut open = Vec::new();
    let mut nested = None;
    for (index, ch) in glob_delimiters(value) {
        match ch {
            '{' => open.push((index, false)),
            ',' => {
                if let Some((_, has_alternatives)) = open.last_mut() {
                    *has_alternatives = true;
                }
            }
            '}' => {
                let (start, has_alternatives) = open.pop().ok_or(())?;
                if has_alternatives {
                    nested = Some(start..index + 1);
                }
                if open.is_empty() && nested.is_some() {
                    return Ok(nested);
                }
            }
            _ => (),
        }
    }
    if open.is_empty() { Ok(None) } else { Err(()) }
}

fn expand_braces(
    pattern: &str,
    globs_left: &mut usize,
    bytes_left: &mut usize,
) -> Option<Vec<String>> {
    if first_brace_group(pattern).ok()?.is_none() {
        return Some(vec![pattern.to_owned()]);
    }
    let mut pending = vec![pattern.to_owned()];
    let mut expanded = Vec::new();
    let mut live_bytes = pattern.len();
    while let Some(pattern) = pending.pop() {
        let Some(group) = first_brace_group(&pattern).ok()? else {
            expanded.push(pattern);
            continue;
        };
        live_bytes -= pattern.len();
        for part in split_globs(&pattern[group.start + 1..group.end - 1])
            .into_iter()
            .rev()
        {
            let alternative = format!("{}{part}{}", &pattern[..group.start], &pattern[group.end..]);
            live_bytes += alternative.len();
            if pending.len() + expanded.len() >= *globs_left || live_bytes > *bytes_left {
                return None;
            }
            pending.push(alternative);
        }
    }
    *globs_left -= expanded.len();
    *bytes_left -= live_bytes;
    Some(expanded)
}

struct RuleGlob {
    matcher: GlobSet,
    match_basename: bool,
}

impl RuleGlob {
    fn new(pattern: &str) -> Option<Self> {
        let anchored = pattern
            .strip_prefix('/')
            .or_else(|| pattern.strip_prefix("./"));
        // Any braces remaining after expansion are literal characters.
        let unanchored = anchored.unwrap_or(pattern);
        let mut escaped = String::new();
        let mut start = 0;
        for (index, ch) in glob_delimiters(unanchored) {
            if matches!(ch, '{' | '}') {
                escaped.push_str(&unanchored[start..index]);
                escaped.push('\\');
                start = index;
            }
        }
        escaped.push_str(&unanchored[start..]);
        let glob = GlobBuilder::new(&escaped)
            .literal_separator(true)
            .backslash_escape(true)
            .build()
            .ok()?;
        let matcher = GlobSetBuilder::new().add(glob).build().ok()?;
        Some(Self {
            matcher,
            match_basename: anchored.is_none() && !pattern.contains('/'),
        })
    }

    fn matches(&self, path: &Path) -> bool {
        self.matcher.is_match(path)
            || (self.match_basename
                && path
                    .file_name()
                    .is_some_and(|name| self.matcher.is_match(Path::new(name))))
    }
}

pub(super) fn matching_target<'a>(
    globs: &[String],
    base: &Path,
    targets: impl IntoIterator<Item = &'a Path>,
) -> Option<&'a Path> {
    let mut targets = targets.into_iter().peekable();
    targets.peek()?;
    let mut globs_left = MAX_EXPANDED_GLOBS;
    let mut bytes_left = MAX_EXPANDED_BYTES;
    let matchers = globs
        .iter()
        .filter_map(|pattern| expand_braces(pattern, &mut globs_left, &mut bytes_left))
        .flatten()
        .filter_map(|pattern| RuleGlob::new(&pattern))
        .collect::<Vec<_>>();
    if matchers.is_empty() {
        return None;
    }
    targets.find(|target| {
        target
            .strip_prefix(base)
            .is_ok_and(|relative| matchers.iter().any(|glob| glob.matches(relative)))
    })
}

pub(super) fn document(text: &str, truncated: bool) -> (Cow<'_, str>, Vec<String>) {
    let mut literals = Vec::new();
    let mut comments = Vec::new();
    for (event, range) in Parser::new(text).into_offset_iter() {
        match event {
            Event::Start(Tag::HtmlBlock) => {
                let raw = &text[range.clone()];
                let comment = raw.trim_start();
                if comment.starts_with("<!--")
                    && let Some(end) = comment.find("-->")
                {
                    let start = range.start + raw.len() - comment.len();
                    comments.push(start..start + end + 3);
                }
                literals.push(range);
            }
            Event::Start(Tag::CodeBlock(_)) | Event::Code(_) | Event::InlineHtml(_) => {
                literals.push(range);
            }
            _ => {}
        }
    }

    let mut literals = literals.into_iter().peekable();
    let mut imports = Vec::new();
    let mut next_start = 0;
    for (index, _) in text.match_indices('@') {
        while literals.peek().is_some_and(|range| range.end <= index) {
            literals.next();
        }
        if index < next_start
            || literals.peek().is_some_and(|range| range.contains(&index))
            || text[..index]
                .chars()
                .next_back()
                .is_some_and(|ch| !ch.is_whitespace() && !matches!(ch, '(' | '[' | '>' | '*' | '|'))
        {
            continue;
        }
        let mut chars = text[index + 1..].char_indices().peekable();
        let mut path = String::new();
        while let Some((offset, ch)) = chars.next() {
            if ch.is_whitespace()
                || matches!(
                    ch,
                    '"' | '\'' | '\u{60}' | '<' | '>' | '“' | '”' | '‘' | '’'
                )
            {
                break;
            }
            if ch == '\\' && chars.peek().is_some_and(|(_, next)| *next == ' ') {
                chars.next();
                path.push(' ');
                next_start = index + offset + 3;
            } else {
                path.push(ch);
                next_start = index + offset + 1 + ch.len_utf8();
            }
        }
        let path = path.trim_end_matches([')', ']', '*', ',', ';']);
        if !path.is_empty() && (!truncated || next_start < text.len()) {
            imports.push(path.to_owned());
        }
    }

    if comments.is_empty() {
        return (Cow::Borrowed(text), imports);
    }
    let mut contents = String::with_capacity(text.len());
    let mut offset = 0;
    for comment in comments {
        contents.push_str(&text[offset..comment.start]);
        offset = comment.end;
    }
    contents.push_str(&text[offset..]);
    (Cow::Owned(contents), imports)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_the_rule_base_and_ignore_individual_invalid_patterns() {
        let text = "---\r\npaths: 'src/**/*.{ts,tsx}, /*.md, photos [2024/**, *.rs'\r\n---\r\nbody";
        let (globs, body) = rule_body(text, false).unwrap();
        let globs = globs.unwrap();
        assert_eq!(body, "body");
        assert_eq!(
            globs,
            ["src/**/*.{ts,tsx}", "/*.md", "photos [2024/**", "*.rs"]
        );
        for (path, expected) in [
            ("src/api.ts", true),
            ("src/api/index.tsx", true),
            ("src/api/index.js", false),
            ("notes.md", true),
            ("src/notes.md", false),
            ("src/api/main.rs", true),
            ("photos [2024/a.jpg", false),
        ] {
            assert_eq!(
                matching_target(
                    &globs,
                    Path::new("project"),
                    [Path::new(&format!("project/{path}"))]
                )
                .is_some(),
                expected,
                "{path}",
            );
        }
        assert_eq!(
            rule_body("---\npaths: [\n---\nbody", false),
            Some((None, "body"))
        );
        assert!(rule_body("---\npaths:\n", true).is_none());
    }

    #[test]
    fn expansion_matches_each_alternative_in_its_own_scope() {
        let globs = [
            "{README.md,src/{api,ui}/*.ts}".into(),
            "{a,b}".repeat(11),
            "*.rs".into(),
        ];
        for (path, matches) in [
            ("a/b/README.md", true),
            ("src/ui/app.ts", true),
            ("src/api/app.ts", true),
            ("x/src/api/app.ts", false),
            ("aaaaaaaaaaa", false),
            ("src/main.rs", true),
        ] {
            assert_eq!(
                matching_target(
                    &globs,
                    Path::new("project"),
                    [Path::new(&format!("project/{path}"))]
                )
                .is_some(),
                matches,
                "{path}"
            );
        }
    }

    #[test]
    fn imports_handle_escaped_spaces_and_markdown_literals() {
        let (text, imports) = document(
            concat!(
                "## Guidance\n\nSee @README and @Design\\ Docs/api.md\n",
                "- \"@quoted.md\" or @'quoted.md' and user@example.com\n",
                "- use \u{60}\u{60}@literal.md\u{60}\u{60} for a literal\n",
                "> ~~~text\n> @fenced.md\n> ~~~\n\n",
                "<!-- private @hidden.md -->\n",
                "\n<!-- multiline\n@hidden.md\n-->\n",
                "\n@next.md\n",
            ),
            false,
        );
        assert_eq!(imports, ["README", "Design Docs/api.md", "next.md"]);
        assert!(!text.contains("private") && !text.contains("multiline"));
        assert!(text.contains("@fenced.md") && text.contains("@literal.md"));
        assert_eq!(document("@full.md @partial", true).1, ["full.md"]);
    }
}
