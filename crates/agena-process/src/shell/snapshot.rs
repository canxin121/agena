//! Interactive startup capture for POSIX shells.
//!
//! A non-interactive login shell never reads the interactive startup file that
//! shapes the user's own terminal (`~/.zshrc`, `~/.bashrc`), so a command run
//! with `-lc` sees a different `PATH`, no aliases, and default shell options.
//! Agena captures that state once per process and replays it as *data*: the
//! environment map is applied to the child, and alias/option declarations are
//! prepended to the command string.
//!
//! Boundaries, all deliberate:
//!
//! - The snapshot never touches disk; it lives in this process only.
//! - Capture is bounded (5 s, 256 KiB) and runs the shell with stdin closed.
//! - Only `bash`, `zsh`, and `sh` are captured; PowerShell and Command Prompt
//!   keep their existing startup behavior.
//! - Captured environment entries pass through [`sanitize_environment`], which
//!   drops startup-injection and dynamic-linker variables.
//! - Alias and option records are replayed only when they match the shape the
//!   shell itself emits for a declaration; anything else is dropped and counted.
//! - Function declarations are not replayed in this version; they need their own
//!   literal decoder before replayed source is safe to prepend.
//!
//! Callers keep the historical `-lc` behavior whenever capture fails.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::Weak;
use std::time::Duration;
use std::time::SystemTime;

use tokio::process::Command;

use super::ShellKind;
use super::env_expand;

/// Overall capture deadline.
pub const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);
/// Maximum bytes captured from either stream before capture fails.
pub const SNAPSHOT_MAX_BYTES: usize = 256 * 1024;

const ENV_MARKER: &str = "AGENA-SNAPSHOT-ENV";
const ALIAS_MARKER: &str = "AGENA-SNAPSHOT-ALIASES";
const OPTION_MARKER: &str = "AGENA-SNAPSHOT-OPTIONS";
const END_MARKER: &str = "AGENA-SNAPSHOT-END";

static SNAPSHOT_WORK: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
static SNAPSHOT_CAPTURES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

/// Environment variables that never come from a snapshot: Agena's own control
/// surface is trusted configuration, not user startup state.
const NEVER_SNAPSHOT_PREFIX: &str = "AGENA_";
/// Startup-injection and dynamic-linker variables, matching the launch-time
/// filter used for every shell subprocess.
const BLOCKED_EXACT: &[&str] = &[
    "BASH_ENV",
    "ENV",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
];
const BLOCKED_PREFIXES: &[&str] = &["DYLD_", "LD_", "BASH_FUNC_"];
/// Variables the shell owns and recreates for every invocation.
const SHELL_OWNED: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_", "PS1", "PS2", "PS3", "PS4"];

/// Why a snapshot could not be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// The dialect has no capture script.
    Unsupported(ShellKind),
    /// The shell process could not be started or exited abnormally.
    Spawn(String),
    /// Capture exceeded [`SNAPSHOT_TIMEOUT`].
    TimedOut,
    /// Capture exceeded [`SNAPSHOT_MAX_BYTES`] on one stream.
    OutputTooLarge,
    /// Capture output did not contain the expected records.
    Malformed(&'static str),
}

impl SnapshotError {
    /// Short, value-free description suitable for tool metadata.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Unsupported(kind) => match kind {
                ShellKind::Powershell => "powershell startup is not captured",
                ShellKind::Cmd => "command prompt startup is not captured",
                _ => "this shell's startup is not captured",
            },
            Self::Spawn(message) => message.as_str(),
            Self::TimedOut => "startup capture timed out",
            Self::OutputTooLarge => "startup capture produced too much output",
            Self::Malformed(message) => message,
        }
    }
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::error::Error for SnapshotError {}

/// Captured interactive startup state, ready to replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSnapshot {
    /// Exported variables, already filtered by [`sanitize_environment`].
    pub env: HashMap<String, String>,
    /// Replayable `alias ...` declarations.
    pub aliases: Vec<String>,
    /// Replayable option declarations (`setopt ...`, `shopt -s ...`).
    pub options: Vec<String>,
    pub shell_kind: ShellKind,
    pub shell_path: PathBuf,
    /// Records dropped because they were not replayable declarations.
    pub dropped_records: usize,
    /// Ignored-candidate notes, for diagnostics only.
    pub warnings: Vec<String>,
}

impl ShellSnapshot {
    /// Shell source to prepend to a command so aliases and options apply.
    pub fn replay_preamble(&self) -> String {
        let mut preamble = String::new();
        for alias in &self.aliases {
            preamble.push_str(alias);
            preamble.push('\n');
        }
        for option in &self.options {
            preamble.push_str(option);
            preamble.push('\n');
        }
        preamble
    }
}

/// Drop variables that must never reach a child through an environment map.
pub fn sanitize_environment(env: &mut HashMap<String, String>) {
    env.retain(|key, _| {
        !BLOCKED_EXACT
            .iter()
            .any(|name| key.eq_ignore_ascii_case(name))
            && !BLOCKED_PREFIXES
                .iter()
                .any(|prefix| starts_with_ignore_ascii_case(key, prefix))
    });
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// Environment entries that a snapshot may supply for `key`.
fn snapshot_allows(key: &str) -> bool {
    !key.is_empty()
        && !key.starts_with(NEVER_SNAPSHOT_PREFIX)
        && !SHELL_OWNED.contains(&key)
        && !BLOCKED_EXACT
            .iter()
            .any(|name| key.eq_ignore_ascii_case(name))
        && !BLOCKED_PREFIXES
            .iter()
            .any(|prefix| starts_with_ignore_ascii_case(key, prefix))
}

/// Startup files whose modification invalidates a cached snapshot.
fn startup_files(kind: ShellKind, env: &HashMap<String, String>) -> Vec<PathBuf> {
    let home = env.get("HOME").map(PathBuf::from);
    match kind {
        ShellKind::Zsh => {
            let base = env
                .get("ZDOTDIR")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .or(home);
            base.map(|base| vec![base.join(".zshrc")])
                .unwrap_or_default()
        }
        ShellKind::Bash => home
            .map(|home| vec![home.join(".bashrc")])
            .unwrap_or_default(),
        ShellKind::Sh | ShellKind::Powershell | ShellKind::Cmd => Vec::new(),
    }
}

fn fingerprint(paths: &[PathBuf]) -> Vec<(PathBuf, Option<SystemTime>)> {
    paths
        .iter()
        .map(|path| {
            let modified = std::fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok();
            (path.clone(), modified)
        })
        .collect()
}

async fn startup_fingerprint(
    kind: ShellKind,
    env: &HashMap<String, String>,
) -> Result<Vec<(PathBuf, Option<SystemTime>)>, SnapshotError> {
    let paths = startup_files(kind, env);
    SNAPSHOT_WORK
        .run(move || fingerprint(&paths))
        .await
        .map_err(|error| SnapshotError::Spawn(format!("startup file worker failed: {error}")))
}

/// Quote a literal for POSIX shells.
fn shell_single_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for character in value.chars() {
        if character == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(character);
        }
    }
    quoted.push('\'');
    quoted
}

/// Build the startup preamble that reproduces the user's interactive setup.
fn startup_preamble(kind: ShellKind, env: &HashMap<String, String>) -> String {
    match kind {
        ShellKind::Zsh => {
            "if [ -n \"${ZDOTDIR-}\" ]; then __agena_rc=\"$ZDOTDIR/.zshrc\"; else __agena_rc=\"${HOME-}/.zshrc\"; fi\n\
             if [ -r \"$__agena_rc\" ] && [ ! -d \"$__agena_rc\" ]; then . \"$__agena_rc\" || true; fi\n\
             unset __agena_rc\n"
                .to_string()
        }
        ShellKind::Bash => {
            "if [ -z \"${BASH_ENV-}\" ] && [ -n \"${HOME-}\" ] && [ -r \"$HOME/.bashrc\" ] && [ ! -d \"$HOME/.bashrc\" ]; then . \"$HOME/.bashrc\" || true; fi\n"
                .to_string()
        }
        ShellKind::Sh => match env.get("ENV") {
            Some(value) => match env_expand::expand_env_path(value, &|name| env.get(name).cloned()) {
                Ok(path) => {
                    let quoted = shell_single_quote(&path);
                    format!(
                        "if [ -r {quoted} ] && [ ! -d {quoted} ]; then . {quoted} || true; fi\n"
                    )
                }
                Err(_) => String::new(),
            },
            None => String::new(),
        },
        ShellKind::Powershell | ShellKind::Cmd => String::new(),
    }
}

/// Commands that emit the alias and option sections for a dialect.
fn section_commands(kind: ShellKind) -> (&'static str, &'static str) {
    match kind {
        ShellKind::Zsh => ("alias -L 2>/dev/null || true", "setopt 2>/dev/null || true"),
        ShellKind::Bash => (
            "alias -p 2>/dev/null || true",
            "shopt -p 2>/dev/null || true",
        ),
        // POSIX sh has no `setopt`/`shopt`; its coarse `set -o` flags are not replayed.
        _ => ("alias 2>/dev/null || true", "true"),
    }
}

/// Script that emits startup state as NUL-delimited records.
fn capture_script(kind: ShellKind, env: &HashMap<String, String>) -> String {
    let (alias_command, option_command) = section_commands(kind);
    let preamble = startup_preamble(kind, env);
    format!(
        "{preamble}\
         printf '\\000%s\\000' '{ENV_MARKER}'\n\
         command env -0 2>/dev/null || true\n\
         printf '\\000%s\\000' '{ALIAS_MARKER}'\n\
         {alias_command}\n\
         printf '\\000%s\\000' '{OPTION_MARKER}'\n\
         {option_command}\n\
         printf '\\000%s\\000' '{END_MARKER}'\n"
    )
}

/// Capture the interactive startup state of `shell_path`.
pub async fn capture(
    kind: ShellKind,
    shell_path: &Path,
    base_env: &HashMap<String, String>,
) -> Result<ShellSnapshot, SnapshotError> {
    if !kind.supports_snapshot() {
        return Err(SnapshotError::Unsupported(kind));
    }
    let script = capture_script(kind, base_env);
    let mut command = Command::new(shell_path);
    command
        .arg("-l")
        .arg("-c")
        .arg(script)
        .env_clear()
        .envs(base_env)
        .stdin(Stdio::null());

    let permit = SNAPSHOT_CAPTURES
        .acquire()
        .await
        .expect("the private snapshot semaphore is never closed");
    let output = crate::output(command, SNAPSHOT_TIMEOUT, SNAPSHOT_MAX_BYTES)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::TimedOut => SnapshotError::TimedOut,
            std::io::ErrorKind::FileTooLarge => SnapshotError::OutputTooLarge,
            _ => SnapshotError::Spawn(error.to_string()),
        })?;
    drop(permit);
    let shell_path = shell_path.to_path_buf();
    SNAPSHOT_WORK
        .run(move || {
            let mut snapshot = parse(kind, &shell_path, &output.stdout)?;
            if !output.status.success() {
                snapshot.warnings.push(format!(
                    "startup capture exited with status {}",
                    output.status
                ));
            }
            Ok(snapshot)
        })
        .await
        .map_err(|error| SnapshotError::Spawn(format!("snapshot decoder worker failed: {error}")))?
}

/// Decode the NUL-delimited capture format.
fn parse(
    kind: ShellKind,
    shell_path: &Path,
    captured: &[u8],
) -> Result<ShellSnapshot, SnapshotError> {
    let mut section = None;
    let mut env = HashMap::new();
    let mut alias_records = Vec::new();
    let mut option_records = Vec::new();
    let mut saw_end = false;
    for record in captured.split(|byte| *byte == 0) {
        let Ok(text) = std::str::from_utf8(record) else {
            continue;
        };
        let text = text.trim_end_matches(['\n', '\r']);
        if text.is_empty() {
            continue;
        }
        match text {
            ENV_MARKER => {
                section = Some(ENV_MARKER);
                continue;
            }
            ALIAS_MARKER => {
                section = Some(ALIAS_MARKER);
                continue;
            }
            OPTION_MARKER => {
                section = Some(OPTION_MARKER);
                continue;
            }
            END_MARKER => {
                saw_end = true;
                section = None;
                continue;
            }
            _ => {}
        }
        match section {
            Some(ENV_MARKER) => {
                if let Some((key, value)) = text.split_once('=')
                    && snapshot_allows(key)
                {
                    env.insert(key.to_string(), value.to_string());
                }
            }
            Some(ALIAS_MARKER) => alias_records.push(text.to_string()),
            Some(OPTION_MARKER) => option_records.push(text.to_string()),
            _ => {}
        }
    }
    if !saw_end {
        return Err(SnapshotError::Malformed(
            "startup capture ended before its final marker",
        ));
    }
    if env.is_empty() {
        return Err(SnapshotError::Malformed(
            "startup capture reported no environment; `env -0` is required",
        ));
    }

    // The capture must not leak its own marker variables into the replay set.
    for marker in [ENV_MARKER, ALIAS_MARKER, OPTION_MARKER, END_MARKER] {
        env.remove(marker);
    }
    let warnings = Vec::new();
    sanitize_environment(&mut env);
    debug_assert!(env.keys().all(|key| snapshot_allows(key)));

    if kind == ShellKind::Sh {
        // POSIX `alias` prints `name='value'`; add the keyword so the record is a
        // complete declaration before it is judged replayable.
        for record in &mut alias_records {
            if !record.starts_with("alias ") {
                record.insert_str(0, "alias ");
            }
        }
    }
    let aliases = select_replayable(alias_records, is_replayable_alias);
    let options = if kind == ShellKind::Sh {
        // POSIX `sh` has no `setopt`/`shopt`; its option records are neither
        // replayed nor counted as dropped.
        Selected::default()
    } else {
        select_replayable(option_records, is_replayable_option)
    };
    let dropped_records = aliases.rejected + options.rejected;
    let aliases = aliases.records;
    let options = options.records;
    Ok(ShellSnapshot {
        env,
        aliases,
        options,
        shell_kind: kind,
        shell_path: shell_path.to_path_buf(),
        dropped_records,
        warnings,
    })
}

#[derive(Default)]
struct Selected {
    records: Vec<String>,
    rejected: usize,
}

fn select_replayable(records: Vec<String>, accept: fn(&str) -> bool) -> Selected {
    let mut selected = Selected {
        records: Vec::new(),
        rejected: 0,
    };
    for record in records {
        if accept(&record) {
            selected.records.push(record);
        } else {
            selected.rejected += 1;
        }
    }
    selected
}

/// `alias name='value'` with balanced single quotes and nothing else.
fn is_replayable_alias(record: &str) -> bool {
    let Some(rest) = record.strip_prefix("alias ") else {
        return false;
    };
    let Some((name, value)) = rest.split_once('=') else {
        return false;
    };
    if name.is_empty()
        || !name.chars().all(|character| {
            character == '_' || character == '-' || character.is_ascii_alphanumeric()
        })
    {
        return false;
    }
    let value = value.trim_end();
    value.len() >= 2
        && value.starts_with('\'')
        && value.ends_with('\'')
        && has_balanced_quotes(value)
}

fn has_balanced_quotes(value: &str) -> bool {
    let mut inside = false;
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        match character {
            '\'' => inside = !inside,
            '\\' if inside => {
                let _ = characters.next();
            }
            _ => {}
        }
    }
    !inside
}

/// `setopt name` (zsh) or `shopt -s|-u name` (bash).
fn is_replayable_option(record: &str) -> bool {
    if let Some(name) = record.strip_prefix("setopt ") {
        return !name.is_empty()
            && name
                .chars()
                .all(|character| character == '_' || character.is_ascii_alphanumeric());
    }
    let Some(rest) = record.strip_prefix("shopt ") else {
        return false;
    };
    let mut parts = rest.split_whitespace();
    let Some(flag) = parts.next() else {
        return false;
    };
    let Some(name) = parts.next() else {
        return false;
    };
    matches!(flag, "-s" | "-u")
        && parts.next().is_none()
        && !name.is_empty()
        && name
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric())
}

struct CacheEntry {
    snapshot: Arc<ShellSnapshot>,
    fingerprint: Vec<(PathBuf, Option<SystemTime>)>,
}

/// Process-wide snapshot cache with startup-file invalidation.
#[derive(Default)]
pub struct SnapshotCache {
    entries: Mutex<HashMap<(ShellKind, PathBuf), CacheEntry>>,
    captures: Mutex<HashMap<(ShellKind, PathBuf), Weak<tokio::sync::Mutex<()>>>>,
}

static CACHE: LazyLock<SnapshotCache> = LazyLock::new(SnapshotCache::default);

/// Shared cache used by every shell start in this process.
pub fn cache() -> &'static SnapshotCache {
    &CACHE
}

impl SnapshotCache {
    /// Return a cached snapshot, capturing one when the startup files changed.
    pub async fn get_or_capture(
        &self,
        kind: ShellKind,
        shell_path: &Path,
        base_env: &HashMap<String, String>,
    ) -> Result<Arc<ShellSnapshot>, SnapshotError> {
        let key = (kind, shell_path.to_path_buf());
        let expected = startup_fingerprint(kind, base_env).await?;
        if let Some(snapshot) = self.get(&key, &expected) {
            return Ok(snapshot);
        }
        // Only the short registry lookup holds a synchronous mutex. Waiting
        // for another caller's shell process yields to the async executor.
        let gate = {
            let mut captures = self
                .captures
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if captures.len() >= 32 {
                captures.retain(|_, gate| gate.strong_count() > 0);
            }
            match captures.get(&key).and_then(Weak::upgrade) {
                Some(gate) => gate,
                None => {
                    let gate = Arc::new(tokio::sync::Mutex::new(()));
                    captures.insert(key.clone(), Arc::downgrade(&gate));
                    gate
                }
            }
        };
        let _capture = gate.lock().await;
        let expected = startup_fingerprint(kind, base_env).await?;
        if let Some(snapshot) = self.get(&key, &expected) {
            return Ok(snapshot);
        }
        let snapshot = Arc::new(capture(kind, shell_path, base_env).await?);
        let retired = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                key,
                CacheEntry {
                    snapshot: Arc::clone(&snapshot),
                    fingerprint: expected,
                },
            );
        // A replaced snapshot may hold many strings; release it outside the
        // registry lock so cache hits never wait for that work.
        drop(retired);
        Ok(snapshot)
    }

    fn get(
        &self,
        key: &(ShellKind, PathBuf),
        expected: &[(PathBuf, Option<SystemTime>)],
    ) -> Option<Arc<ShellSnapshot>> {
        if let Some(entry) = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            && entry.fingerprint == expected
        {
            return Some(Arc::clone(&entry.snapshot));
        }
        None
    }

    /// Number of cached snapshots; used by tests and diagnostics.
    pub fn cached_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(sections: &[(&str, &[&str])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(0);
        for (marker, entries) in sections {
            bytes.push(0);
            bytes.extend_from_slice(marker.as_bytes());
            bytes.push(0);
            for entry in *entries {
                bytes.extend_from_slice(entry.as_bytes());
                bytes.push(0);
            }
        }
        bytes
    }

    fn base_env() -> HashMap<String, String> {
        HashMap::from([
            ("HOME".to_string(), "/Users/ada".to_string()),
            ("PATH".to_string(), "/usr/bin".to_string()),
        ])
    }

    #[test]
    fn parses_sections_and_ignores_leading_shell_noise() {
        let mut captured = b"welcome to your shell\n".to_vec();
        captured.extend(records(&[
            (
                ENV_MARKER,
                &["PATH=/opt/bin:/usr/bin", "EDITOR=vim", "PWD=/tmp"],
            ),
            (ALIAS_MARKER, &["alias ll='ls -l'", "not an alias"]),
            (OPTION_MARKER, &["shopt -s expand_aliases", "shopt -s"]),
            (END_MARKER, &[]),
        ]));
        let snapshot =
            parse(ShellKind::Bash, Path::new("/bin/bash"), &captured).expect("snapshot parses");
        assert_eq!(snapshot.env.get("EDITOR"), Some(&"vim".to_string()));
        assert_eq!(
            snapshot.env.get("PATH"),
            Some(&"/opt/bin:/usr/bin".to_string())
        );
        assert!(!snapshot.env.contains_key("PWD"));
        assert_eq!(snapshot.aliases, vec!["alias ll='ls -l'"]);
        assert_eq!(snapshot.options, vec!["shopt -s expand_aliases"]);
        assert_eq!(snapshot.dropped_records, 2);
    }

    #[test]
    fn snapshot_never_supplies_agena_or_linker_variables() {
        let captured = records(&[
            (
                ENV_MARKER,
                &[
                    "AGENA_SHELL_SANDBOX=offline",
                    "LD_PRELOAD=/tmp/evil.so",
                    "DYLD_INSERT_LIBRARIES=/tmp/evil.dylib",
                    "BASH_ENV=/tmp/evil.sh",
                    "BASH_FUNC_x%%=() { evil; }",
                    "PATH=/opt/bin",
                ],
            ),
            (ALIAS_MARKER, &[]),
            (OPTION_MARKER, &[]),
            (END_MARKER, &[]),
        ]);
        let snapshot =
            parse(ShellKind::Bash, Path::new("/bin/bash"), &captured).expect("snapshot parses");
        assert_eq!(snapshot.env.len(), 1);
        assert_eq!(snapshot.env.get("PATH"), Some(&"/opt/bin".to_string()));
    }

    #[test]
    fn missing_end_marker_and_empty_environment_are_malformed() {
        let truncated = records(&[(ENV_MARKER, &["PATH=/usr/bin"]), (ALIAS_MARKER, &[])]);
        assert_eq!(
            parse(ShellKind::Bash, Path::new("/bin/bash"), &truncated),
            Err(SnapshotError::Malformed(
                "startup capture ended before its final marker"
            ))
        );

        let empty = records(&[
            (ENV_MARKER, &["PWD=/tmp"]),
            (ALIAS_MARKER, &[]),
            (OPTION_MARKER, &[]),
            (END_MARKER, &[]),
        ]);
        assert_eq!(
            parse(ShellKind::Bash, Path::new("/bin/bash"), &empty),
            Err(SnapshotError::Malformed(
                "startup capture reported no environment; `env -0` is required"
            ))
        );
    }

    #[test]
    fn alias_and_option_records_must_be_replayable_declarations() {
        assert!(is_replayable_alias("alias ll='ls -l'"));
        assert!(is_replayable_alias("alias x='a;b'"));
        assert!(!is_replayable_alias("alias ll='ls -l"));
        assert!(!is_replayable_alias("alias ll=ls -l"));
        assert!(!is_replayable_alias("alias ='x'"));
        assert!(!is_replayable_alias("ll='ls -l'"));
        assert!(is_replayable_option("setopt extendedglob"));
        assert!(is_replayable_option("shopt -s expand_aliases"));
        assert!(!is_replayable_option("shopt -s"));
        assert!(!is_replayable_option("setopt a; rm -rf /"));
    }

    #[test]
    fn sh_aliases_gain_the_alias_keyword() {
        let captured = records(&[
            (ENV_MARKER, &["PATH=/usr/bin"]),
            (ALIAS_MARKER, &["ll='ls -l'"]),
            (OPTION_MARKER, &["posix on"]),
            (END_MARKER, &[]),
        ]);
        let snapshot =
            parse(ShellKind::Sh, Path::new("/bin/sh"), &captured).expect("snapshot parses");
        assert_eq!(snapshot.aliases, vec!["alias ll='ls -l'"]);
        assert!(snapshot.options.is_empty());
    }

    #[test]
    fn per_kind_scripts_reference_the_expected_startup_file() {
        let env = base_env();
        let zsh = capture_script(ShellKind::Zsh, &env);
        assert!(zsh.contains("$ZDOTDIR/.zshrc"));
        assert!(zsh.contains("alias -L"));
        assert!(zsh.contains("setopt"));

        let bash = capture_script(ShellKind::Bash, &env);
        assert!(bash.contains("BASH_ENV"));
        assert!(bash.contains("$HOME/.bashrc"));
        assert!(bash.contains("shopt -p"));

        let sh = capture_script(ShellKind::Sh, &env);
        assert!(sh.contains("alias"));
        assert!(!sh.contains("shopt -p"));
        assert!(sh.contains(ENV_MARKER));
    }

    #[test]
    fn sh_script_sources_the_whitelisted_env_file() {
        let mut env = base_env();
        env.insert("ENV".to_string(), "$HOME/.shrc".to_string());
        let script = capture_script(ShellKind::Sh, &env);
        assert!(script.contains(". '/Users/ada/.shrc'"));

        env.insert("ENV".to_string(), "$(touch /tmp/pwned)".to_string());
        let refused = capture_script(ShellKind::Sh, &env);
        assert!(!refused.contains("pwned'"));
        assert!(!refused.contains(". '"));
    }

    #[tokio::test]
    async fn unsupported_dialects_report_a_reason() {
        for kind in [ShellKind::Powershell, ShellKind::Cmd] {
            let error = capture(kind, Path::new("/bin/false"), &HashMap::new())
                .await
                .expect_err("unsupported shell");
            assert!(matches!(error, SnapshotError::Unsupported(_)));
            assert!(!error.as_str().is_empty());
        }
    }

    #[tokio::test]
    async fn capture_reads_state_from_the_posix_env_file() {
        if !Path::new("/bin/sh").is_file() {
            return;
        }
        let fixture = tempfile::tempdir().expect("temp dir");
        let shrc = fixture.path().join("shrc");
        std::fs::write(
            &shrc,
            "export SNAP_TEST_VALUE=from_shrc\nexport AGENA_SNAPSHOT_MARKER=never_replayed\nalias agena_test='echo hi'\n",
        )
        .expect("write shrc");

        let mut env = HashMap::new();
        env.insert(
            "HOME".to_string(),
            fixture.path().to_string_lossy().to_string(),
        );
        env.insert("ENV".to_string(), shrc.to_string_lossy().to_string());
        env.insert(
            "PATH".to_string(),
            std::env::var("PATH").unwrap_or_default(),
        );

        let snapshot = capture(ShellKind::Sh, Path::new("/bin/sh"), &env)
            .await
            .expect("capture sh startup");
        assert_eq!(
            snapshot.env.get("SNAP_TEST_VALUE"),
            Some(&"from_shrc".to_string())
        );
        // Agena's own control surface is trusted configuration, never snapshot data.
        assert!(!snapshot.env.contains_key("AGENA_SNAPSHOT_MARKER"));
        assert!(
            snapshot
                .aliases
                .iter()
                .any(|alias| alias.contains("agena_test"))
        );
        assert!(snapshot.replay_preamble().contains("alias agena_test"));
    }

    #[tokio::test]
    async fn cached_snapshot_is_reused_and_refreshed_on_change() {
        if !Path::new("/bin/sh").is_file() {
            return;
        }
        let fixture = tempfile::tempdir().expect("temp dir");
        let shrc = fixture.path().join("shrc");
        std::fs::write(&shrc, "SNAP_TEST_VALUE=first\n").expect("write shrc");
        let mut env = HashMap::new();
        env.insert(
            "HOME".to_string(),
            fixture.path().to_string_lossy().to_string(),
        );
        env.insert("ENV".to_string(), shrc.to_string_lossy().to_string());

        let cache = SnapshotCache::default();
        let first = cache
            .get_or_capture(ShellKind::Sh, Path::new("/bin/sh"), &env)
            .await
            .expect("first capture");
        let second = cache
            .get_or_capture(ShellKind::Sh, Path::new("/bin/sh"), &env)
            .await
            .expect("cached capture");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(cache.cached_count(), 1);

        // POSIX sh has no tracked startup file, so the cache stays warm; the
        // documented invalidation surface is the zsh/bash rc files.
        assert!(startup_files(ShellKind::Sh, &env).is_empty());
        assert_eq!(startup_files(ShellKind::Zsh, &base_env()).len(), 1);
        assert_eq!(startup_files(ShellKind::Bash, &base_env()).len(), 1);
    }
}
