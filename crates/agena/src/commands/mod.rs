//! Custom slash commands.
//!
//! Discovers Markdown files under `.agena/commands/` (project-local, walked
//! up from the current dir) and `~/.agena/commands/` (user-global) and
//! exposes them as [`CustomCommand`] values that a TUI / CLI dispatcher can
//! resolve when the user types `/<name> <args>`.
//!
//! A command file looks like:
//!
//! ```markdown
//! ---
//! description: "Quick smoke test"
//! argument-hint: "[target-dir]"
//! allowed_tools: ["bash", "read"]
//! model: "claude-haiku-4-5"
//! ---
//! Run a smoke test against $1 and report a one-line summary.
//! ```
//!
//! Project files win when the same name exists in both scopes — same rule
//! Claude Code uses for `~/.claude/commands` vs `.claude/commands`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandScope {
    Project,
    User,
    Builtin,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandFrontmatter {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(
        default,
        rename = "argument-hint",
        skip_serializing_if = "Option::is_none"
    )]
    pub argument_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CustomCommand {
    pub name: String,
    pub frontmatter: CommandFrontmatter,
    pub body: String,
    pub source_path: Option<PathBuf>,
    pub scope: CommandScope,
}

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed command: {0}")]
    Malformed(String),
    #[error("yaml frontmatter error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("unknown command: {0}")]
    UnknownCommand(String),
}

pub type CommandResult<T> = Result<T, CommandError>;

#[derive(Debug, Clone, Default)]
pub struct CustomCommandRegistry {
    inner: Arc<RwLock<CommandRegistryInner>>,
}

#[derive(Debug, Default)]
struct CommandRegistryInner {
    by_name: BTreeMap<String, CustomCommand>,
}

impl CustomCommandRegistry {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Discover commands rooted at `workspace_root` (walks up to find any
    /// `.agena/commands/` ancestors) and `user_root` (typically
    /// `~/.agena/commands`). Project entries win on name collisions.
    pub fn discover(workspace_root: &Path, user_root: Option<&Path>) -> Self {
        let registry = Self::default();
        registry.reload_disk(workspace_root, user_root);
        registry
    }

    pub fn reload_disk(&self, workspace_root: &Path, user_root: Option<&Path>) {
        let mut inner = self.inner.write();
        inner.by_name.clear();
        if let Some(user) = user_root {
            load_dir(&mut inner.by_name, user, CommandScope::User);
        }
        for dir in collect_project_command_dirs(workspace_root) {
            load_dir(&mut inner.by_name, &dir, CommandScope::Project);
        }
    }

    /// Register a runtime command (typically by a plugin). Uses
    /// [`CommandScope::Builtin`] when caller does not specify, but plugins
    /// may pass `Project`/`User` to mirror disk priority.
    pub fn register_runtime(&self, command: CustomCommand) {
        let mut inner = self.inner.write();
        let scope = command.scope;
        upsert(&mut inner.by_name, command, scope);
    }

    pub fn remove_runtime(&self, name: &str) -> bool {
        let mut inner = self.inner.write();
        let removed = inner.by_name.remove(name).is_some();
        // Also drop any aliases pointing at the same command.
        let to_drop: Vec<String> = inner
            .by_name
            .iter()
            .filter(|(_, cmd)| cmd.name == name)
            .map(|(k, _)| k.clone())
            .collect();
        for key in to_drop {
            inner.by_name.remove(&key);
        }
        removed
    }

    pub fn names(&self) -> Vec<String> {
        self.inner.read().by_name.keys().cloned().collect()
    }

    pub fn list(&self) -> Vec<CustomCommand> {
        let inner = self.inner.read();
        let mut seen: BTreeMap<String, CustomCommand> = BTreeMap::new();
        for cmd in inner.by_name.values() {
            seen.entry(cmd.name.clone()).or_insert_with(|| cmd.clone());
        }
        seen.into_values().collect()
    }

    pub fn get(&self, name: &str) -> Option<CustomCommand> {
        self.inner.read().by_name.get(name).cloned()
    }

    /// Render a command into the prompt the LLM should see.
    /// Substitutes `$1..$N` and `$ARGUMENTS`.
    pub fn render(&self, name: &str, raw_args: &str) -> CommandResult<RenderedCommand> {
        let cmd = self
            .get(name)
            .ok_or_else(|| CommandError::UnknownCommand(name.to_string()))?;
        let args = split_args(raw_args);
        let prompt = substitute(&cmd.body, &args, raw_args);
        Ok(RenderedCommand {
            name: cmd.name.clone(),
            prompt,
            allowed_tools: cmd.frontmatter.allowed_tools.clone(),
            model: cmd.frontmatter.model.clone(),
            scope: cmd.scope,
            source_path: cmd.source_path.clone(),
        })
    }
}

fn load_dir(by_name: &mut BTreeMap<String, CustomCommand>, dir: &Path, scope: CommandScope) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        match CustomCommand::from_path(&path, &stem, scope) {
            Ok(cmd) => {
                upsert(by_name, cmd, scope);
            }
            Err(err) => {
                tracing::warn!(
                    target: "agena::commands",
                    "skipping command `{}`: {err}",
                    path.display()
                );
            }
        }
    }
}

fn upsert(
    by_name: &mut BTreeMap<String, CustomCommand>,
    command: CustomCommand,
    scope: CommandScope,
) {
    let candidate_keys = std::iter::once(command.name.clone())
        .chain(command.frontmatter.aliases.iter().cloned())
        .collect::<Vec<_>>();
    for key in candidate_keys {
        match by_name.get(&key) {
            Some(existing) => {
                if scope_priority(scope) >= scope_priority(existing.scope) {
                    by_name.insert(key, command.clone());
                }
            }
            None => {
                by_name.insert(key, command.clone());
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct RenderedCommand {
    pub name: String,
    pub prompt: String,
    pub allowed_tools: Vec<String>,
    pub model: Option<String>,
    pub scope: CommandScope,
    pub source_path: Option<PathBuf>,
}

impl CustomCommand {
    pub fn from_path(path: &Path, stem: &str, scope: CommandScope) -> CommandResult<Self> {
        let raw = std::fs::read_to_string(path)?;
        let mut cmd = Self::from_raw(&raw, stem, scope)?;
        cmd.source_path = Some(path.to_path_buf());
        Ok(cmd)
    }

    pub fn from_raw(raw: &str, default_name: &str, scope: CommandScope) -> CommandResult<Self> {
        let (frontmatter, body) = parse_frontmatter(raw)?;
        let name = if default_name.is_empty() {
            return Err(CommandError::Malformed(
                "command file name (stem) must not be empty".into(),
            ));
        } else {
            default_name.to_string()
        };
        Ok(Self {
            name,
            frontmatter,
            body,
            source_path: None,
            scope,
        })
    }
}

fn parse_frontmatter(raw: &str) -> CommandResult<(CommandFrontmatter, String)> {
    let normalized = raw.replace("\r\n", "\n");
    let Some(stripped) = normalized.strip_prefix("---\n") else {
        return Ok((CommandFrontmatter::default(), normalized.trim().to_string()));
    };
    let Some(end) = stripped.find("\n---") else {
        return Err(CommandError::Malformed(
            "frontmatter missing closing '---'".into(),
        ));
    };
    let yaml = &stripped[..end];
    let body = stripped[end + 4..].trim_start_matches('\n').to_string();
    let frontmatter: CommandFrontmatter = if yaml.trim().is_empty() {
        CommandFrontmatter::default()
    } else {
        serde_yaml::from_str(yaml)?
    };
    Ok((frontmatter, body))
}

fn collect_project_command_dirs(workspace_root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut current = Some(workspace_root.to_path_buf());
    while let Some(dir) = current {
        let candidate = dir.join(".agena").join("commands");
        if candidate.is_dir() {
            dirs.push(candidate);
        }
        current = dir.parent().map(Path::to_path_buf);
    }
    dirs.reverse();
    dirs
}

fn split_args(raw: &str) -> Vec<String> {
    raw.split_whitespace().map(str::to_string).collect()
}

fn substitute(body: &str, args: &[String], raw: &str) -> String {
    let mut out = body.replace("$ARGUMENTS", raw.trim());
    for (idx, arg) in args.iter().enumerate() {
        let placeholder = format!("${}", idx + 1);
        out = out.replace(&placeholder, arg);
    }
    out
}

fn scope_priority(scope: CommandScope) -> u8 {
    match scope {
        CommandScope::Builtin => 0,
        CommandScope::User => 1,
        CommandScope::Project => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("agena-cmd-{label}-{suffix}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn parses_frontmatter_and_body() {
        let raw = "---\ndescription: \"smoke\"\nargument-hint: \"[dir]\"\nallowed_tools:\n  - bash\nmodel: gpt-5\n---\nRun a smoke check on $1\n";
        let cmd = CustomCommand::from_raw(raw, "smoke", CommandScope::Project).unwrap();
        assert_eq!(cmd.name, "smoke");
        assert_eq!(cmd.frontmatter.description, "smoke");
        assert_eq!(cmd.frontmatter.argument_hint.as_deref(), Some("[dir]"));
        assert_eq!(cmd.frontmatter.allowed_tools, vec!["bash"]);
        assert_eq!(cmd.frontmatter.model.as_deref(), Some("gpt-5"));
        assert_eq!(cmd.body.trim(), "Run a smoke check on $1");
    }

    #[test]
    fn missing_frontmatter_treats_whole_file_as_body() {
        let cmd = CustomCommand::from_raw("just the body", "plain", CommandScope::User).unwrap();
        assert_eq!(cmd.body, "just the body");
        assert!(cmd.frontmatter.description.is_empty());
    }

    #[test]
    fn frontmatter_without_closing_marker_errors() {
        let raw = "---\ndescription: oops\nbody without closing marker";
        let err = CustomCommand::from_raw(raw, "x", CommandScope::User).unwrap_err();
        assert!(matches!(err, CommandError::Malformed(_)));
    }

    #[test]
    fn project_overrides_user_with_same_name() {
        let work = temp_dir("project-wins");
        let user = temp_dir("user-base");
        let project_cmds = work.join(".agena").join("commands");
        let user_cmds = user.join("commands");
        fs::create_dir_all(&project_cmds).unwrap();
        fs::create_dir_all(&user_cmds).unwrap();
        fs::write(
            project_cmds.join("smoke.md"),
            "---\ndescription: project\n---\nproject body",
        )
        .unwrap();
        fs::write(
            user_cmds.join("smoke.md"),
            "---\ndescription: user\n---\nuser body",
        )
        .unwrap();

        let registry = CustomCommandRegistry::discover(&work, Some(&user_cmds));
        let cmd = registry.get("smoke").expect("command present");
        assert_eq!(cmd.scope, CommandScope::Project);
        assert_eq!(cmd.frontmatter.description, "project");
    }

    #[test]
    fn render_substitutes_positional_args_and_arguments() {
        let work = temp_dir("render");
        let dir = work.join(".agena").join("commands");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("greet.md"),
            "---\ndescription: greet\n---\nHello $1, full args: $ARGUMENTS",
        )
        .unwrap();
        let registry = CustomCommandRegistry::discover(&work, None);
        let rendered = registry.render("greet", "world from agena").unwrap();
        assert!(rendered.prompt.contains("Hello world,"));
        assert!(rendered.prompt.contains("full args: world from agena"));
    }

    #[test]
    fn aliases_resolve_to_the_same_command() {
        let work = temp_dir("aliases");
        let dir = work.join(".agena").join("commands");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("review.md"),
            "---\ndescription: review\naliases: [\"r\", \"rv\"]\n---\ndo a review",
        )
        .unwrap();
        let registry = CustomCommandRegistry::discover(&work, None);
        assert_eq!(registry.get("review").unwrap().name, "review");
        assert_eq!(registry.get("r").unwrap().name, "review");
        assert_eq!(registry.get("rv").unwrap().name, "review");
        // list() deduplicates aliases.
        assert_eq!(registry.list().len(), 1);
    }

    #[test]
    fn unknown_command_renders_error() {
        let registry = CustomCommandRegistry::empty();
        let err = registry.render("nope", "").unwrap_err();
        assert!(matches!(err, CommandError::UnknownCommand(_)));
    }

    #[test]
    fn discovery_walks_up_to_find_command_dir() {
        let outer = temp_dir("walk-up");
        fs::create_dir_all(outer.join(".agena").join("commands")).unwrap();
        fs::write(
            outer.join(".agena").join("commands").join("note.md"),
            "---\ndescription: note\n---\nnote body",
        )
        .unwrap();
        let nested = outer.join("nested").join("deep");
        fs::create_dir_all(&nested).unwrap();
        let registry = CustomCommandRegistry::discover(&nested, None);
        assert!(registry.get("note").is_some());
    }
}
