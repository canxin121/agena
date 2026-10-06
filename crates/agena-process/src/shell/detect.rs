//! Platform shell discovery: login shell, PATH lookup, and fallbacks.
//!
//! Model-authored command strings are POSIX-shaped, so discovery only ever
//! returns a shell that can run them: `bash`, `zsh`, or `sh` on Unix and the
//! configured command interpreter on Windows. Anything else (fish, tcsh, nu,
//! ...) is reported as a diagnostic and replaced by `sh`, never executed with a
//! POSIX command string.

use std::path::Path;
use std::path::PathBuf;

use super::ShellPreference;
use super::ShellSettings;

/// Shell dialect used to build command arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShellKind {
    Bash,
    Zsh,
    Sh,
    Powershell,
    Cmd,
}

impl ShellKind {
    /// Stable name used in metadata, diagnostics, and PATH lookup.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Sh => "sh",
            Self::Powershell => "powershell",
            Self::Cmd => "cmd",
        }
    }

    /// Dialect of a shell executable path, including Windows `.exe` names.
    pub fn from_path(path: &Path) -> Option<Self> {
        // Both separators are accepted so a Windows path resolves identically on
        // any host, which keeps the precedence tests platform-independent.
        let raw = path.to_str()?;
        let name = raw.rsplit(['/', '\\']).next()?;
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name);
        match name {
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "sh" | "dash" | "ksh" => Some(Self::Sh),
            "pwsh" | "powershell" => Some(Self::Powershell),
            "cmd" => Some(Self::Cmd),
            _ => None,
        }
    }

    /// True for shells that read POSIX-style startup files and take `-c`.
    pub const fn is_posix(self) -> bool {
        matches!(self, Self::Bash | Self::Zsh | Self::Sh)
    }

    /// True for shells whose interactive startup state can be captured.
    pub const fn supports_snapshot(self) -> bool {
        self.is_posix()
    }
}

/// Where the resolved shell program came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellSource {
    /// `AGENA_SHELL` named this shell explicitly.
    Override,
    /// The user's login shell (Unix) or the platform default was usable.
    Detected,
    /// The preferred shell was unusable and a documented fallback was used.
    Fallback,
}

impl ShellSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Override => "override",
            Self::Detected => "detected",
            Self::Fallback => "fallback",
        }
    }
}

/// A shell program that can run Agena's POSIX-shaped command strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedShell {
    pub kind: ShellKind,
    pub path: PathBuf,
    pub source: ShellSource,
    /// Human-readable notes about replacements or unavailable candidates.
    pub notes: Vec<String>,
}

impl DetectedShell {
    fn new(kind: ShellKind, path: PathBuf, source: ShellSource) -> Self {
        Self {
            kind,
            path,
            source,
            notes: Vec::new(),
        }
    }
}

/// Host queries used by discovery. Injected so precedence rules stay testable.
pub(crate) trait ShellHost {
    fn var(&self, key: &str) -> Option<String>;
    /// Unix login shell from the password database; `None` elsewhere.
    fn login_shell(&self) -> Option<PathBuf>;
    fn is_executable_file(&self, path: &Path) -> bool;
}

pub(crate) struct RealShellHost;

impl ShellHost for RealShellHost {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|value| !value.is_empty())
    }

    fn login_shell(&self) -> Option<PathBuf> {
        read_login_shell()
    }

    fn is_executable_file(&self, path: &Path) -> bool {
        let Ok(metadata) = std::fs::metadata(path) else {
            return false;
        };
        if !metadata.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            metadata.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    }
}

/// Absolute candidates for a dialect, most specific first.
fn candidate_paths(kind: ShellKind) -> &'static [&'static str] {
    match kind {
        ShellKind::Bash => &["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"],
        ShellKind::Zsh => &["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh"],
        ShellKind::Sh => &["/bin/sh", "/usr/bin/sh"],
        ShellKind::Powershell => &["/usr/local/bin/pwsh", "/usr/bin/pwsh"],
        ShellKind::Cmd => &[],
    }
}

/// Resolve `name` (bare name or path) through `PATH`.
fn search_path(name: &str, host: &dyn ShellHost) -> Option<PathBuf> {
    let path = host.var("PATH")?;
    let separator = if cfg!(windows) { ';' } else { ':' };
    for entry in path.split(separator) {
        if entry.is_empty() {
            continue;
        }
        let directory = Path::new(entry);
        for candidate_name in candidate_names(name) {
            let candidate = directory.join(candidate_name);
            if host.is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn candidate_names(name: &str) -> Vec<String> {
    if cfg!(windows) && !name.ends_with(".exe") {
        vec![name.to_string(), format!("{name}.exe")]
    } else {
        vec![name.to_string()]
    }
}

/// Find a usable program for one dialect: absolute candidates, then `PATH`.
fn lookup(kind: ShellKind, host: &dyn ShellHost) -> Option<PathBuf> {
    for candidate in candidate_paths(kind) {
        let candidate = PathBuf::from(candidate);
        if host.is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    search_path(kind.name(), host)
}

fn resolve_override(raw: &str, host: &dyn ShellHost) -> Result<DetectedShell, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(format!(
            "{} is empty; ignoring the override",
            super::SHELL_ENV
        ));
    }
    let named = Path::new(raw);
    let path = if named.components().count() > 1 || named.is_absolute() {
        named.to_path_buf()
    } else {
        search_path(raw, host).ok_or_else(|| {
            format!(
                "{} names `{raw}`, which is not executable or not on PATH; ignoring the override",
                super::SHELL_ENV
            )
        })?
    };
    let kind = ShellKind::from_path(&path).ok_or_else(|| {
        format!(
            "{} names `{raw}`, which is not a supported shell for POSIX command strings; ignoring the override",
            super::SHELL_ENV
        )
    })?;
    if !host.is_executable_file(&path) {
        return Err(format!(
            "{} names `{raw}`, which is not an executable file; ignoring the override",
            super::SHELL_ENV
        ));
    }
    Ok(DetectedShell::new(kind, path, ShellSource::Override))
}

fn resolve_powershell(host: &dyn ShellHost, notes: &mut Vec<String>) -> DetectedShell {
    match lookup(ShellKind::Powershell, host) {
        Some(path) => DetectedShell::new(ShellKind::Powershell, path, ShellSource::Detected),
        None => {
            notes.push(
                "no pwsh executable was found; falling back to powershell.exe from PATH"
                    .to_string(),
            );
            DetectedShell::new(
                ShellKind::Powershell,
                PathBuf::from("powershell.exe"),
                ShellSource::Fallback,
            )
        }
    }
}

fn resolve_windows_posix(host: &dyn ShellHost, notes: &mut Vec<String>) -> DetectedShell {
    if let Some(command) = host.var("COMSPEC")
        && host.is_executable_file(Path::new(&command))
        && ShellKind::from_path(Path::new(&command)) == Some(ShellKind::Cmd)
    {
        return DetectedShell::new(
            ShellKind::Cmd,
            PathBuf::from(command),
            ShellSource::Detected,
        );
    }
    if let Some(found) = lookup(ShellKind::Cmd, host) {
        return DetectedShell::new(ShellKind::Cmd, found, ShellSource::Detected);
    }
    notes.push(
        "no cmd.exe was found on PATH; using the bare name for the OS to resolve".to_string(),
    );
    DetectedShell::new(
        ShellKind::Cmd,
        PathBuf::from("cmd.exe"),
        ShellSource::Fallback,
    )
}

fn resolve_login_shell(host: &dyn ShellHost, notes: &mut Vec<String>) -> Option<DetectedShell> {
    let configured = host.login_shell()?;
    let Some(kind) = ShellKind::from_path(&configured) else {
        notes.push(format!(
            "login shell `{}` is not a supported POSIX shell; using the fallback shell",
            configured.display()
        ));
        return None;
    };
    if host.is_executable_file(&configured) {
        return Some(DetectedShell::new(kind, configured, ShellSource::Detected));
    }
    match lookup(kind, host) {
        Some(found) => {
            notes.push(format!(
                "login shell `{}` is missing; using `{}` from PATH",
                configured.display(),
                found.display()
            ));
            Some(DetectedShell::new(kind, found, ShellSource::Detected))
        }
        None => {
            notes.push(format!(
                "login shell `{}` is missing and no replacement was found; using the fallback shell",
                configured.display()
            ));
            None
        }
    }
}

/// Last-resort POSIX shell, preserving Agena's historical `/bin/sh` behavior.
fn resolve_posix_fallback(host: &dyn ShellHost, notes: &mut Vec<String>) -> DetectedShell {
    for kind in [ShellKind::Sh, ShellKind::Bash, ShellKind::Zsh] {
        if let Some(path) = lookup(kind, host) {
            return DetectedShell::new(kind, path, ShellSource::Fallback);
        }
    }
    notes.push(
        "no sh, bash, or zsh executable was found; using bare `sh` for the OS to resolve"
            .to_string(),
    );
    DetectedShell::new(ShellKind::Sh, PathBuf::from("sh"), ShellSource::Fallback)
}

pub(crate) fn resolve_with(
    settings: &ShellSettings,
    preference: ShellPreference,
    host: &dyn ShellHost,
) -> DetectedShell {
    let mut notes = Vec::new();
    if let Some(raw) = settings.shell_override() {
        match resolve_override(raw, host) {
            Ok(shell) => return shell,
            Err(note) => notes.push(note),
        }
    }

    let mut resolved = match preference {
        ShellPreference::Powershell => resolve_powershell(host, &mut notes),
        ShellPreference::Posix => {
            if cfg!(windows) {
                resolve_windows_posix(host, &mut notes)
            } else {
                resolve_login_shell(host, &mut notes)
                    .unwrap_or_else(|| resolve_posix_fallback(host, &mut notes))
            }
        }
    };
    resolved.notes = notes;
    resolved
}

/// Read the login shell from the password database.
///
/// `getpwuid` returns pointers into libc-managed storage that is not safe to
/// read concurrently on every target, so the reentrant form with a
/// caller-owned buffer is used instead.
#[cfg(unix)]
fn read_login_shell() -> Option<PathBuf> {
    use std::ffi::CStr;
    use std::mem::MaybeUninit;
    use std::ptr;

    let uid = unsafe { libc::getuid() };
    let suggested = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let mut buffer_len = usize::try_from(suggested)
        .ok()
        .filter(|len| *len > 0)
        .unwrap_or(1024);

    loop {
        let mut passwd = MaybeUninit::<libc::passwd>::uninit();
        let mut buffer = vec![0_u8; buffer_len];
        let mut result = ptr::null_mut();
        // SAFETY: `passwd` and `buffer` are caller-owned and live across the call.
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                passwd.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if status != 0 {
            if status == libc::ERANGE && buffer_len < 1024 * 1024 {
                buffer_len *= 2;
                continue;
            }
            return None;
        }
        if result.is_null() {
            return None;
        }
        // SAFETY: the call reported success and filled the caller-owned entry.
        let passwd = unsafe { passwd.assume_init_ref() };
        if passwd.pw_shell.is_null() {
            return None;
        }
        // SAFETY: the password database guarantees a NUL-terminated shell path.
        let shell = unsafe { CStr::from_ptr(passwd.pw_shell) }
            .to_string_lossy()
            .into_owned();
        let shell = shell.trim();
        if shell.is_empty() {
            return None;
        }
        return Some(PathBuf::from(shell));
    }
}

#[cfg(not(unix))]
fn read_login_shell() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeHost {
        vars: HashMap<String, String>,
        login_shell: Option<PathBuf>,
        existing: Vec<PathBuf>,
    }

    impl FakeHost {
        fn new() -> Self {
            Self {
                vars: HashMap::new(),
                login_shell: None,
                existing: Vec::new(),
            }
        }

        fn with_var(mut self, key: &str, value: &str) -> Self {
            self.vars.insert(key.to_string(), value.to_string());
            self
        }

        fn with_login_shell(mut self, path: &str) -> Self {
            self.login_shell = Some(PathBuf::from(path));
            self
        }

        fn with_file(mut self, path: &str) -> Self {
            self.existing.push(PathBuf::from(path));
            self
        }
    }

    impl ShellHost for FakeHost {
        fn var(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }

        fn login_shell(&self) -> Option<PathBuf> {
            self.login_shell.clone()
        }

        fn is_executable_file(&self, path: &Path) -> bool {
            self.existing.iter().any(|candidate| candidate == path)
        }
    }

    fn settings(override_value: Option<&str>) -> ShellSettings {
        match override_value {
            Some(value) => ShellSettings::from_lookup(&|key| {
                (key == super::super::SHELL_ENV).then(|| value.to_string())
            }),
            None => ShellSettings::default(),
        }
    }

    #[test]
    fn login_shell_kind_wins_when_supported() {
        let host = FakeHost::new()
            .with_login_shell("/bin/zsh")
            .with_file("/bin/zsh");
        let shell = resolve_with(&settings(None), ShellPreference::Posix, &host);
        assert_eq!(shell.kind, ShellKind::Zsh);
        assert_eq!(shell.path, PathBuf::from("/bin/zsh"));
        assert_eq!(shell.source, ShellSource::Detected);
        assert!(shell.notes.is_empty());
    }

    #[test]
    fn unsupported_login_shell_keeps_historical_sh_fallback() {
        let host = FakeHost::new()
            .with_login_shell("/usr/local/bin/fish")
            .with_file("/bin/sh");
        let shell = resolve_with(&settings(None), ShellPreference::Posix, &host);
        assert_eq!(shell.kind, ShellKind::Sh);
        assert_eq!(shell.source, ShellSource::Fallback);
        assert_eq!(shell.notes.len(), 1);
        assert!(shell.notes[0].contains("fish"));
    }

    #[test]
    fn missing_login_shell_prefers_path_lookup_then_fallback() {
        let host = FakeHost::new()
            .with_login_shell("/opt/homebrew/bin/zsh")
            .with_var("PATH", "/usr/bin")
            .with_file("/usr/bin/zsh")
            .with_file("/bin/sh");
        let shell = resolve_with(&settings(None), ShellPreference::Posix, &host);
        assert_eq!(shell.kind, ShellKind::Zsh);
        assert_eq!(shell.path, PathBuf::from("/usr/bin/zsh"));
        assert_eq!(shell.source, ShellSource::Detected);
        assert!(shell.notes.iter().any(|note| note.contains("missing")));
    }

    #[test]
    fn no_login_shell_uses_documented_fallback_order() {
        let host = FakeHost::new()
            .with_var("PATH", "/usr/bin")
            .with_file("/usr/bin/bash");
        let shell = resolve_with(&settings(None), ShellPreference::Posix, &host);
        assert_eq!(shell.kind, ShellKind::Bash);
        assert_eq!(shell.source, ShellSource::Fallback);
    }

    #[test]
    fn override_wins_and_records_its_source() {
        let host = FakeHost::new()
            .with_login_shell("/bin/zsh")
            .with_file("/opt/homebrew/bin/bash");
        let shell = resolve_with(
            &settings(Some("/opt/homebrew/bin/bash")),
            ShellPreference::Posix,
            &host,
        );
        assert_eq!(shell.kind, ShellKind::Bash);
        assert_eq!(shell.path, PathBuf::from("/opt/homebrew/bin/bash"));
        assert_eq!(shell.source, ShellSource::Override);
    }

    #[test]
    fn unsupported_or_missing_override_is_ignored_with_a_note() {
        let host = FakeHost::new()
            .with_login_shell("/bin/zsh")
            .with_file("/bin/zsh")
            .with_file("/opt/homebrew/bin/fish");
        let shell = resolve_with(
            &settings(Some("/opt/homebrew/bin/fish")),
            ShellPreference::Posix,
            &host,
        );
        assert_eq!(shell.kind, ShellKind::Zsh);
        assert_eq!(shell.source, ShellSource::Detected);
        assert!(
            shell
                .notes
                .iter()
                .any(|note| note.contains("not a supported shell"))
        );

        let missing = FakeHost::new()
            .with_login_shell("/bin/sh")
            .with_file("/bin/sh");
        let shell = resolve_with(
            &settings(Some("/nowhere/bash")),
            ShellPreference::Posix,
            &missing,
        );
        assert_eq!(shell.source, ShellSource::Detected);
        assert!(
            shell
                .notes
                .iter()
                .any(|note| note.contains("not an executable file"))
        );
    }

    #[test]
    fn bare_override_name_resolves_through_path() {
        let host = FakeHost::new()
            .with_var("PATH", "/opt/homebrew/bin")
            .with_file("/opt/homebrew/bin/zsh");
        let shell = resolve_with(&settings(Some("zsh")), ShellPreference::Posix, &host);
        assert_eq!(shell.kind, ShellKind::Zsh);
        assert_eq!(shell.path, PathBuf::from("/opt/homebrew/bin/zsh"));
    }

    #[test]
    fn shell_kind_from_path_covers_exe_and_unsupported_names() {
        assert_eq!(
            ShellKind::from_path(Path::new("C:\\Windows\\System32\\cmd.exe")),
            Some(ShellKind::Cmd)
        );
        assert_eq!(
            ShellKind::from_path(Path::new("/bin/bash")),
            Some(ShellKind::Bash)
        );
        assert_eq!(
            ShellKind::from_path(Path::new("/usr/local/bin/pwsh")),
            Some(ShellKind::Powershell)
        );
        assert_eq!(
            ShellKind::from_path(Path::new("/usr/bin/ksh")),
            Some(ShellKind::Sh)
        );
        assert_eq!(ShellKind::from_path(Path::new("/usr/bin/fish")), None);
    }

    #[test]
    fn real_host_reads_the_current_login_shell_when_available() {
        let host = RealShellHost;
        let shell = resolve_with(&ShellSettings::default(), ShellPreference::Posix, &host);
        assert!(
            shell.kind.is_posix(),
            "unexpected shell kind: {:?}",
            shell.kind
        );
        assert!(!shell.path.as_os_str().is_empty());
    }
}
