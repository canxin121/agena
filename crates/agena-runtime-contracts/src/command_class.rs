//! Shell command classes, as evaluated by the compiled tool policy.
//!
//! These three classes are what the automatic-approval fast path and shell
//! heuristics used to decide before they became configuration keys
//! (`tools.no_op_commands`, `tools.routine_commands`,
//! `tools.dangerous_commands`):
//!
//! - [`CommandClass::NoOp`] — the exact no-op commands `true`, `:`, `false`.
//! - [`CommandClass::Routine`] — routine development commands (`git status`,
//!   `cargo test`, `ls`, …).
//! - [`CommandClass::Dangerous`] — commands that destroy data or run
//!   untrusted code (`rm -rf /`, `curl … | sh`, `mkfs`, `dd`, `chmod 777`, …).
//!
//! Detection works on *command positions*, not raw substrings: a substring test
//! over the whole line is both false-negative and false-positive prone — it
//! denies `grep mkfs Cargo.toml` because the line contains `mkfs`, while happily
//! allowing `sh -c 'mkfs /dev/sda'` because the fragment is preceded by a quote.
//! Splitting the line into the segments a shell would actually run, and
//! inspecting the leading word of each, fixes both directions.

/// A command class a class default applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandClass {
    NoOp,
    Routine,
    Dangerous,
}

/// Classify a shell command into the classes it matches, most specific
/// first. An empty result means no class default applies and the command falls through
/// to the configured tool/shell policy.
pub fn command_classes(command: &str) -> Option<CommandClass> {
    let normalized = normalize_command(command);
    if is_dangerous_command(&normalized) {
        return Some(CommandClass::Dangerous);
    }
    if is_exact_noop_command(&normalized) {
        return Some(CommandClass::NoOp);
    }
    if is_routine_command(&normalized) {
        return Some(CommandClass::Routine);
    }
    None
}

/// Exact no-op shell commands: they cannot change anything.
pub fn is_exact_noop_command(command: &str) -> bool {
    matches!(command.trim(), "true" | ":" | "false")
}

fn normalize_command(command: &str) -> String {
    command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

/// Split a command line into the segments a shell would run as separate
/// commands (on `;`, `&&`, `||`, `|`, `&`, newlines, and subshell parens).
///
/// Quotes are *not* split on: their contents belong to the segment that
/// introduces them, which is what keeps `sh -c '…'` analyzable as a whole.
fn shell_segments(command: &str) -> Vec<&str> {
    command
        .split([';', '|', '&', '\n', '(', ')', '`'])
        .collect()
}

/// Words that precede the real command in a segment without changing what runs.
/// Stripped so `sudo rm -rf /` and `env FOO=1 rm -rf /` still lead with `rm`.
const COMMAND_PREFIXES: &[&str] = &[
    "sudo", "doas", "env", "nohup", "time", "command", "exec", "builtin", "nice", "ionice",
    "stdbuf", "xargs",
];

/// Every fragment of the line that the shell would run as a command: the
/// segments between separators, the contents of each quoted string (which is
/// how `sh -c '…'` and `bash -c "…"` carry their payload), and each
/// parenthesized/substituted part. Fragments are inspected rather than the raw
/// line, so a danger word used as an *argument* is never mistaken for a
/// command.
fn command_fragments(command: &str) -> Vec<&str> {
    let mut fragments = Vec::new();
    for segment in shell_segments(command) {
        fragments.push(segment);
        for (index, quoted) in segment.split(['"', '\'']).enumerate() {
            if index % 2 == 1 && !quoted.trim().is_empty() {
                fragments.push(quoted);
            }
        }
    }
    fragments
}

/// Read one fragment as a command: skip leading prefixes, environment
/// assignments, and flags, then report the program name and whether it runs
/// with elevated privilege.
fn fragment_command(fragment: &str) -> Option<(&str, bool)> {
    let mut privileged = false;
    for word in fragment.split_whitespace() {
        if COMMAND_PREFIXES.contains(&word) {
            privileged |= matches!(word, "sudo" | "doas");
            continue;
        }
        if word.starts_with('-') || (word.contains('=') && !word.starts_with('-')) {
            continue;
        }
        return Some((word, privileged));
    }
    None
}

/// The command each fragment starts with, paired with its privilege. A
/// dangerous program is detected wherever it actually runs — behind `sh -c`,
/// behind `xargs`, in a substitution, or after `&&`.
fn command_positions(command: &str) -> Vec<(&str, bool)> {
    command_fragments(command)
        .into_iter()
        .filter_map(fragment_command)
        .collect()
}

/// `rm` invocations in the line, paired with their first non-flag argument and
/// whether the deletion runs with elevated privilege.
fn rm_targets(command: &str) -> Vec<(&str, bool)> {
    let mut targets = Vec::new();
    for fragment in command_fragments(command) {
        let mut words = fragment.split_whitespace().peekable();
        let mut privileged = false;
        // A word that is neither a prefix nor a flag before the `rm` means this
        // fragment's `rm` is an argument to something else (`grep rm notes.txt`)
        // and deletes nothing.
        let mut is_rm = false;
        for word in words.by_ref() {
            if COMMAND_PREFIXES.contains(&word) {
                privileged |= matches!(word, "sudo" | "doas");
                continue;
            }
            if word.starts_with('-') || (word.contains('=') && !word.starts_with('-')) {
                continue;
            }
            is_rm = word == "rm";
            break;
        }
        if !is_rm {
            continue;
        }
        if let Some(target) = words.find(|word| !word.starts_with('-')) {
            targets.push((target, privileged));
        }
    }
    targets
}

fn is_dangerous_command(command: &str) -> bool {
    let positions = command_positions(command);
    for (name, privileged) in &positions {
        // Raw device / filesystem destruction. `dd` only counts with an
        // explicit input or output: `dd if=…` / `dd of=…` write somewhere,
        // bare `dd` reads stdin.
        if name.starts_with("mkfs") {
            return true;
        }
        if *name == "dd" && (command.contains("if=") || command.contains("of=")) {
            return true;
        }
        // System power control is destructive when asked for directly, and
        // doubly so with privilege.
        if matches!(*name, "shutdown" | "reboot" | "poweroff" | "halt") && *privileged {
            return true;
        }
        // Reverse shells: the interpreter is only dangerous with `-e`.
        if matches!(*name, "nc" | "ncat") && command.contains(" -e") {
            return true;
        }
        if *name == "socat" {
            return true;
        }
    }
    // System power control invoked bare (the common accidental shape).
    if positions
        .iter()
        .any(|(name, _)| matches!(*name, "shutdown" | "reboot" | "poweroff" | "halt"))
    {
        return true;
    }
    // Fork bomb.
    if command.contains(":(){") || command.contains(":() {") {
        return true;
    }
    // Remote code execution: something fetched or decoded is piped *into* an
    // interpreter. Only a real pipe counts — `curl -O x && bash y` downloads and
    // then runs an unrelated local script, which is not the same act.
    for (upstream, downstream) in command.split('|').zip(command.split('|').skip(1)) {
        let produces_payload = upstream.contains("base64 -d")
            || upstream.contains("base64 --decode")
            || upstream
                .split_whitespace()
                .any(|word| matches!(word, "curl" | "wget"));
        let consumes_payload = fragment_command(downstream)
            .is_some_and(|(name, _)| is_shell_name(name) || is_interpreter_name(name));
        if produces_payload && consumes_payload {
            return true;
        }
    }
    // System-file overwrite by redirection or a tool's output flag.
    const SYSTEM_TARGETS: &[&str] = &[
        "> /etc/",
        ">> /etc/",
        "-o /etc/",
        "> /dev/sd",
        "-o /dev/sd",
        "> /boot/",
        "-o /boot/",
    ];
    if SYSTEM_TARGETS
        .iter()
        .any(|fragment| command.contains(fragment))
    {
        return true;
    }
    // World-writable permissions.
    if positions.iter().any(|(name, _)| *name == "chmod")
        && command.split_whitespace().any(|word| word.contains("777"))
    {
        return true;
    }
    // `rm` on a path that is absolute, home-relative, or the current directory.
    // A target inside a system temp directory is scratch space, not data — the
    // temp-path default is configured separately, and denying a delete there
    // would be inconsistent with it — so it is left to the approval model,
    // which can see whether the user asked for it. Privileged deletion is
    // never exempt.
    if rm_targets(command).iter().any(|(target, privileged)| {
        (target.starts_with('/') || target.starts_with('~') || matches!(*target, "." | "./"))
            && (*privileged || !is_within_temp_dir(target))
    }) {
        return true;
    }
    false
}

fn is_shell_name(name: &str) -> bool {
    matches!(name, "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish")
}

fn is_interpreter_name(name: &str) -> bool {
    matches!(name, "python" | "python3" | "node" | "perl" | "ruby")
}

fn is_routine_command(command: &str) -> bool {
    let words = command.split_whitespace().collect::<Vec<_>>();
    let Some(first) = words.first() else {
        return false;
    };
    if matches!(*first, "sudo" | "doas" | "su") || command_has_shell_metacharacters(command) {
        return false;
    }
    match *first {
        "git" => words.get(1).is_some_and(|verb| {
            matches!(
                *verb,
                "status"
                    | "diff"
                    | "log"
                    | "show"
                    | "branch"
                    | "remote"
                    | "ls-files"
                    | "config"
                    | "add"
                    | "commit"
                    | "checkout"
                    | "switch"
                    | "stash"
                    | "tag"
                    | "rev-parse"
                    | "shortlog"
                    | "blame"
                    | "describe"
                    | "help"
                    | "version"
            )
        }),
        "cargo" => words.get(1).is_some_and(|verb| {
            matches!(
                *verb,
                "build"
                    | "check"
                    | "test"
                    | "fmt"
                    | "clippy"
                    | "doc"
                    | "metadata"
                    | "tree"
                    | "search"
                    | "info"
                    | "version"
                    | "help"
            )
        }),
        "npm" | "pnpm" | "yarn" | "bun" => words.get(1).is_some_and(|verb| {
            matches!(
                *verb,
                "run" | "test" | "build" | "ls" | "list" | "outdated" | "why" | "version" | "help"
            )
        }),
        "ls" | "pwd" | "cat" | "head" | "tail" | "grep" | "wc" | "echo" | "whoami" | "env"
        | "date" | "uname" | "which" | "type" | "printenv" | "history" | "jobs" | "ps"
        | "uptime" | "du" | "df" | "tree" | "file" | "stat" | "true" | "false" | ":" => true,
        _ => false,
    }
}

fn command_has_shell_metacharacters(command: &str) -> bool {
    command.chars().any(|character| {
        matches!(
            character,
            '|' | '>'
                | '<'
                | '&'
                | ';'
                | '$'
                | '`'
                | '('
                | ')'
                | '{'
                | '}'
                | '*'
                | '?'
                | '['
                | ']'
                | '~'
                | '!'
                | '\\'
        )
    })
}

/// Well-known system temporary directories: scratch space. The platform's
/// configured temp dir (TMPDIR / TMP / TEMP) is checked separately because it
/// varies per user (macOS: /var/folders/.../T, Windows: %LOCALAPPDATA%\Temp).
const SYSTEM_TEMP_DIR_ROOTS: &[&str] = &[
    "/tmp",
    "/private/tmp",
    "/var/tmp",
    "/private/var/tmp",
    "C:/Windows/Temp",
];

/// True when `target` is inside one of the system's temporary directories.
pub fn is_within_temp_dir(target: &str) -> bool {
    let target = target.replace('\\', "/");
    if SYSTEM_TEMP_DIR_ROOTS
        .iter()
        .any(|root| path_is_within_root(&target, root))
    {
        return true;
    }
    let env_root = std::env::temp_dir().to_string_lossy().replace('\\', "/");
    !env_root.is_empty() && path_is_within_root(&target, &env_root)
}

fn path_is_within_root(target: &str, root: &str) -> bool {
    let mut target = target.replace('\\', "/");
    let mut root = root.replace('\\', "/");
    if cfg!(windows) {
        target.make_ascii_lowercase();
        root.make_ascii_lowercase();
    }
    let root = root.trim_end_matches('/');
    if root.is_empty() {
        return false;
    }
    if target == root {
        return true;
    }
    let Some(prefix) = target.strip_prefix(&format!("{root}/")) else {
        return false;
    };
    !prefix.split('/').any(|segment| segment == "..")
}
