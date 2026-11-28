//! A Claude permission-style condition limits which calls reach a hook.

use std::path::{Component, Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};
use regex::Regex;
use serde_json::Value;

#[derive(Clone, Debug)]
pub(super) struct ToolCondition {
    source: String,
    tool: String,
    argument: Argument,
}

#[derive(Clone, Debug)]
enum Argument {
    Any,
    Command(Regex),
    File { origin: Origin, glob: GlobMatcher },
    Domain(String),
    Agent(Regex),
}

#[derive(Clone, Copy, Debug)]
enum Origin {
    Workspace,
    Absolute,
    Home,
}

impl PartialEq for ToolCondition {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl ToolCondition {
    pub(super) fn parse(rule: &str) -> Result<Self, String> {
        let rule = rule.trim();
        if rule.len() > 4096 || rule.chars().any(char::is_control) {
            return Err("invalid hook condition".into());
        }
        let (tool, specifier) = match rule.split_once('(') {
            Some((tool, specifier)) => (
                tool,
                Some(
                    specifier
                        .strip_suffix(')')
                        .filter(|value| !value.is_empty())
                        .ok_or("hook condition requires Tool(argument)")?,
                ),
            ),
            None => (rule, None),
        };
        if tool.is_empty()
            || !tool
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".:_-".contains(&byte))
        {
            return Err("hook condition requires a tool name".into());
        }
        let argument = match (tool, specifier) {
            (_, None | Some("*")) => Argument::Any,
            ("Bash" | "PowerShell" | "agena.shell.exec" | "agena.shell.spawn", Some(pattern)) => {
                Argument::Command(command_glob(pattern)?)
            }
            (
                "Read" | "Edit" | "Write" | "MultiEdit" | "Glob" | "Grep" | "NotebookRead"
                | "ReadNotebook" | "NotebookEdit" | "NotebookEditCell",
                Some(pattern),
            ) => {
                let (origin, pattern) = if pattern.starts_with("//") {
                    (Origin::Absolute, &pattern[1..])
                } else if let Some(pattern) = pattern.strip_prefix("~/") {
                    (Origin::Home, pattern)
                } else {
                    (Origin::Workspace, pattern.trim_start_matches('/'))
                };
                let pattern = pattern.strip_prefix("./").unwrap_or(pattern);
                let glob = GlobBuilder::new(pattern)
                    .literal_separator(true)
                    .build()
                    .map_err(|error| format!("invalid hook file pattern: {error}"))?
                    .compile_matcher();
                Argument::File { origin, glob }
            }
            ("WebFetch", Some(specifier)) => {
                let host = specifier
                    .strip_prefix("domain:")
                    .filter(|value| !value.is_empty())
                    .ok_or("WebFetch hook conditions use domain:example.com")?;
                let host = url::Host::parse(host).map_err(|_| "invalid WebFetch hook domain")?;
                Argument::Domain(host.to_string())
            }
            ("Agent" | "Task", Some(pattern)) => Argument::Agent(command_glob(pattern)?),
            (_, Some(_)) => {
                return Err(format!(
                    "hook condition arguments for {tool} are unsupported"
                ));
            }
        };
        Ok(Self {
            source: rule.to_owned(),
            tool: tool.to_owned(),
            argument,
        })
    }

    pub(super) fn matches_tool(&self, name: &str, aliases: &[&str]) -> bool {
        self.tool == name || aliases.contains(&self.tool.as_str())
    }

    pub(super) fn matches_input(&self, input: &Value, cwd: Option<&str>) -> bool {
        match &self.argument {
            Argument::Any => true,
            Argument::Command(pattern) => input["command"]
                .as_str()
                .is_some_and(|command| shell_matches(pattern, command)),
            Argument::File { origin, glob } => input["file_path"]
                .as_str()
                .or_else(|| input["path"].as_str())
                .or_else(|| input["notebook_path"].as_str())
                .and_then(|path| rule_path(path, cwd, *origin))
                .is_some_and(|path| glob.is_match(path)),
            Argument::Domain(domain) => input["url"]
                .as_str()
                .and_then(|url| url::Url::parse(url).ok())
                .is_some_and(|url| url.host_str() == Some(domain)),
            Argument::Agent(pattern) => input["subagent_type"]
                .as_str()
                .or_else(|| input["profile"].as_str())
                .is_some_and(|profile| pattern.is_match(profile)),
        }
    }
}

fn command_glob(pattern: &str) -> Result<Regex, String> {
    // The old :* spelling and a trailing space + * both include the command
    // without arguments. An unqualified * can match any number of characters.
    let pattern = pattern
        .strip_suffix(":*")
        .map_or_else(|| pattern.to_owned(), |prefix| format!("{prefix} *"));
    let (pattern, optional_arguments) = pattern
        .strip_suffix(" *")
        .map_or((pattern.as_str(), false), |prefix| (prefix, true));
    let mut regex = String::from(r"\A");
    for (index, literal) in pattern.split('*').enumerate() {
        if index != 0 {
            regex.push_str(".*");
        }
        regex.push_str(&regex::escape(literal));
    }
    if optional_arguments {
        regex.push_str("(?: .*)?");
    }
    regex.push_str(r"\z");
    regex::RegexBuilder::new(&regex)
        .dot_matches_new_line(true)
        .size_limit(256 * 1024)
        .build()
        .map_err(|_| "hook command pattern is too complex".into())
}

fn shell_matches(pattern: &Regex, command: &str) -> bool {
    let matches = |part: &str| {
        let Some(words) = shlex::split(part) else {
            return true;
        };
        let start = words
            .iter()
            .position(|word| !shell_assignment(word))
            .unwrap_or(words.len());
        !words[start..].is_empty() && pattern.is_match(&words[start..].join(" "))
    };
    let mut quote = None;
    let mut escaped = false;
    let mut start = 0;
    for (index, character) in command.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(current) = quote {
            if current == character {
                quote = None;
            } else if current == '"' && matches!(character, '$' | '\x60') {
                return true;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            ';' | '|' | '&' | '\n' => {
                if matches(&command[start..index]) {
                    return true;
                }
                start = index + character.len_utf8();
            }
            // Expansion and compound syntax may introduce additional commands.
            '$' | '\x60' | '(' | ')' | '<' | '>' | '{' | '}' => return true,
            _ => {}
        }
    }
    quote.is_some() || escaped || matches(&command[start..])
}

fn shell_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    })
}

fn rule_path(path: &str, cwd: Option<&str>, origin: Origin) -> Option<PathBuf> {
    let path = Path::new(path);
    let path = if path.is_absolute() {
        normalize(path)
    } else if let Some(cwd) = cwd {
        normalize(&Path::new(cwd).join(path))
    } else if matches!(origin, Origin::Workspace) {
        return Some(normalize(path));
    } else {
        return None;
    };
    match origin {
        Origin::Workspace => path
            .strip_prefix(normalize(Path::new(cwd?)))
            .ok()
            .map(Path::to_owned),
        Origin::Absolute => Some(path),
        Origin::Home => path
            .strip_prefix(normalize(&std::env::home_dir()?))
            .ok()
            .map(Path::to_owned),
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir if normalized.file_name().is_some_and(|name| name != "..") => {
                normalized.pop();
            }
            Component::ParentDir if normalized.has_root() => {}
            part => normalized.push(part),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ToolCondition;

    #[test]
    fn shell_condition_checks_subcommands_assignments_and_quotes() {
        let condition = ToolCondition::parse("Bash(git *)").unwrap();
        assert!(condition.matches_tool("Bash", &["agena.shell.exec"]));
        assert!(!condition.matches_tool("Read", &["agena.fs.read"]));
        let matches = |command| condition.matches_input(&json!({"command":command}), None);
        assert!(matches("echo 'a; b' && COLOR=always git status"));
        assert!(matches("git"));
        assert!(!matches("cargo test && echo 'git status'"));
        assert!(!matches("gitk"));
        assert!(matches("echo $(git status)"));
    }

    #[test]
    fn file_condition_respects_workspace_relative_and_recursive_patterns() {
        let condition = ToolCondition::parse("Edit(src/**)").unwrap();
        let matches = |path| condition.matches_input(&json!({"file_path":path}), Some("/work"));
        assert!(matches("/work/src/lib/mod.rs"));
        assert!(matches("src/lib.rs"));
        assert!(!matches("/work/dependency/src/lib.rs"));
        assert!(!matches("src/../script/main.rs"));
        let any_depth = ToolCondition::parse("Edit(**/src/**)").unwrap();
        assert!(any_depth.matches_input(
            &json!({"file_path":"dependency/src/main.rs"}),
            Some("/work")
        ));
        let absolute = ToolCondition::parse("Read(//work/src/**)").unwrap();
        assert!(absolute.matches_input(&json!({"file_path":"src/lib.rs"}), Some("/work")));
    }
}
