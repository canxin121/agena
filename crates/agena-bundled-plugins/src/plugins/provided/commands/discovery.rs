//! Command discovery across the standard roots.
//!
//! This module is private to the `agena.commands` plugin. It is the only place
//! that reads command source from disk: a `SKILL.md` package under a skill
//! root, or a plain `.md` file under a command root. Everything it returns is
//! plain text; nothing here reaches the model unless a caller explicitly reads
//! a catalog entry.
//!
//! The frontmatter subset is deliberately tiny — `name`, `description` and
//! `aliases` — so the plugin needs no YAML dependency and a malformed document
//! is rejected with a usable diagnostic instead of a parser stack trace.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use sha2::{Digest, Sha256};

pub(crate) const MAX_COMMAND_DOCUMENT_BYTES: usize = 1_048_576;
const MAX_DISCOVERY_ENTRIES_PER_ROOT: usize = 10_000;
const MAX_DISCOVERY_DEPTH: usize = 8;

pub(crate) type DiscoveryResult<T> = Result<T, DiscoveryError>;

/// Why one discovered document could not be turned into a catalog entry.
#[derive(Debug)]
pub(crate) enum DiscoveryError {
    Io(std::io::Error),
    Malformed(String),
    InvalidResourcePath(String),
    ResourceTooLarge { path: String, limit: usize },
    ResourceNotText { path: String },
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Malformed(message) => formatter.write_str(message),
            Self::InvalidResourcePath(path) => {
                write!(formatter, "`{path}` is not a readable resource path")
            }
            Self::ResourceTooLarge { path, limit } => {
                write!(formatter, "`{path}` exceeds the {limit} byte limit")
            }
            Self::ResourceNotText { path } => {
                write!(formatter, "`{path}` is not valid UTF-8 text")
            }
        }
    }
}

impl std::error::Error for DiscoveryError {}

impl From<std::io::Error> for DiscoveryError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// The metadata a command document declares about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CommandFrontmatter {
    pub name: String,
    pub description: String,
    /// Alternate names a composer recognizes for the same command.
    pub aliases: Vec<String>,
}

/// One discovered command: its metadata, its instructions, and — when it came
/// from disk — the document and directory it was read from.
#[derive(Debug, Clone)]
pub(crate) struct DiscoveredCommand {
    pub frontmatter: CommandFrontmatter,
    pub body: String,
    /// Absolute path of the source document; `None` for a compiled-in command.
    pub source_path: Option<PathBuf>,
}

impl DiscoveredCommand {
    /// A command whose instructions are compiled into the executable.
    pub(crate) fn bundled(frontmatter: CommandFrontmatter, body: impl Into<String>) -> Self {
        Self {
            frontmatter,
            body: body.into(),
            source_path: None,
        }
    }

    /// Parse a `SKILL.md` package document from disk.
    pub(crate) fn from_package_path(path: impl AsRef<Path>) -> DiscoveryResult<Self> {
        let path = path.as_ref();
        let raw = read_command_document(path)?;
        let mut command = Self::from_raw(&raw)?;
        command.source_path = Some(path.to_path_buf());
        Ok(command)
    }

    /// Parse a bare command markdown file from disk. The file name is the
    /// default `name`, so `~/.agena/commands/standup.md` needs no frontmatter
    /// to become `/standup`.
    pub(crate) fn from_command_path(path: impl AsRef<Path>) -> DiscoveryResult<Self> {
        let path = path.as_ref();
        let raw = read_command_document(path)?;
        let default_name = path.file_stem().and_then(|name| name.to_str());
        let mut command = Self::from_raw_with_default_name(&raw, default_name)?;
        command.source_path = Some(path.to_path_buf());
        Ok(command)
    }

    /// Parse a document that must carry frontmatter.
    pub(crate) fn from_raw(raw: &str) -> DiscoveryResult<Self> {
        Self::from_raw_with_default_name(raw, None)
    }

    fn from_raw_with_default_name(raw: &str, default_name: Option<&str>) -> DiscoveryResult<Self> {
        // A command file without frontmatter is still a command: its whole
        // text is the instruction body and its file name is its name. Only a
        // document that opens a frontmatter block must close it.
        let Some(stripped) = raw.strip_prefix("---\n") else {
            let name = default_name
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    DiscoveryError::Malformed(
                        "a command document without frontmatter needs a file name to be named after"
                            .into(),
                    )
                })?;
            return Ok(Self {
                frontmatter: CommandFrontmatter {
                    name: name.to_string(),
                    ..CommandFrontmatter::default()
                },
                body: raw.trim_start_matches('\n').to_string(),
                source_path: None,
            });
        };
        let end = stripped.find("\n---").ok_or_else(|| {
            DiscoveryError::Malformed("command frontmatter is missing its closing '---'".into())
        })?;
        let frontmatter = &stripped[..end];
        let body = stripped[end + 4..].trim_start_matches('\n').to_string();
        let mut frontmatter = parse_frontmatter(frontmatter)?;
        if frontmatter.name.trim().is_empty() {
            match default_name.map(str::trim).filter(|name| !name.is_empty()) {
                Some(default_name) => frontmatter.name = default_name.to_string(),
                None => {
                    return Err(DiscoveryError::Malformed(
                        "command frontmatter must declare `name`".into(),
                    ));
                }
            }
        }
        Ok(Self {
            frontmatter,
            body,
            source_path: None,
        })
    }

    /// Whether `query` names this command, by canonical name or alias.
    pub(crate) fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        self.frontmatter.name.to_ascii_lowercase() == query
            || self
                .frontmatter
                .aliases
                .iter()
                .any(|alias| alias.to_ascii_lowercase() == query)
    }

    /// Stable content identity for catalogs and stale-document checks.
    ///
    /// Paths are excluded on purpose, so moving an unchanged command does not
    /// change its identity.
    pub(crate) fn content_hash(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(self.frontmatter.name.as_bytes());
        digest.update([0]);
        digest.update(self.frontmatter.description.as_bytes());
        digest.update([0]);
        for alias in &self.frontmatter.aliases {
            digest.update(alias.as_bytes());
            digest.update([0]);
        }
        digest.update([0]);
        digest.update(self.body.as_bytes());
        hex::encode(digest.finalize())
    }

    /// The directory a packaged command may keep resources in.
    pub(crate) fn resource_root(&self) -> Option<&Path> {
        self.source_path.as_deref()?.parent()
    }

    /// Read one UTF-8 resource contained by the command's directory.
    ///
    /// Canonical path checks reject `..` and symlink escapes before any
    /// content is returned.
    pub(crate) fn read_text_resource(
        &self,
        relative: &Path,
        max_bytes: usize,
    ) -> DiscoveryResult<String> {
        if relative.as_os_str().is_empty() || relative.is_absolute() {
            return Err(DiscoveryError::InvalidResourcePath(
                relative.display().to_string(),
            ));
        }
        if relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        }) {
            return Err(DiscoveryError::InvalidResourcePath(
                relative.display().to_string(),
            ));
        }
        let root = self.resource_root().ok_or_else(|| {
            DiscoveryError::InvalidResourcePath(
                "compiled-in commands have no resource directory".into(),
            )
        })?;
        let canonical_root = root.canonicalize()?;
        let candidate = root.join(relative);
        let canonical_candidate = candidate.canonicalize()?;
        if !canonical_candidate.starts_with(&canonical_root) || !canonical_candidate.is_file() {
            return Err(DiscoveryError::InvalidResourcePath(
                relative.display().to_string(),
            ));
        }
        read_text_file_bounded(&canonical_candidate, max_bytes).map_err(|error| match error {
            DiscoveryError::ResourceTooLarge { limit, .. } => DiscoveryError::ResourceTooLarge {
                path: relative.display().to_string(),
                limit,
            },
            DiscoveryError::ResourceNotText { .. } => DiscoveryError::ResourceNotText {
                path: relative.display().to_string(),
            },
            other => other,
        })
    }
}

/// Read a command document, refusing anything over the size limit before the
/// whole file is materialized in memory.
pub(crate) fn read_command_document(path: impl AsRef<Path>) -> DiscoveryResult<String> {
    read_text_file_bounded(path.as_ref(), MAX_COMMAND_DOCUMENT_BYTES)
}

/// Parse the small frontmatter subset a command document may declare.
///
/// Only three keys are understood. An unknown key is an error rather than a
/// silent no-op, because a typo in `alias` or `descripton` would otherwise
/// quietly drop metadata the author believed they had declared.
fn parse_frontmatter(source: &str) -> DiscoveryResult<CommandFrontmatter> {
    let mut frontmatter = CommandFrontmatter::default();
    let mut aliases = Vec::new();
    let mut block_aliases = false;
    for (index, line) in source.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            if !block_aliases {
                return Err(DiscoveryError::Malformed(format!(
                    "frontmatter line {line_number}: a list item must follow `aliases:`"
                )));
            }
            aliases.push(unquote(item.trim()).to_string());
            continue;
        }
        block_aliases = false;
        let Some((key, value)) = trimmed.split_once(':') else {
            return Err(DiscoveryError::Malformed(format!(
                "frontmatter line {line_number}: expected `key: value`"
            )));
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "name" => frontmatter.name = unquote(value).to_string(),
            "description" => frontmatter.description = unquote(value).to_string(),
            "aliases" => {
                if value.is_empty() {
                    block_aliases = true;
                } else {
                    aliases.extend(parse_inline_aliases(value).map_err(|message| {
                        DiscoveryError::Malformed(format!(
                            "frontmatter line {line_number}: {message}"
                        ))
                    })?);
                }
            }
            other => {
                return Err(DiscoveryError::Malformed(format!(
                    "frontmatter line {line_number}: unknown key `{other}`"
                )));
            }
        }
    }
    frontmatter.aliases = aliases
        .into_iter()
        .map(|alias| alias.trim().to_string())
        .filter(|alias| !alias.is_empty())
        .collect();
    Ok(frontmatter)
}

/// Parse `[one, two]` or a bare `one` on the `aliases:` line.
fn parse_inline_aliases(value: &str) -> Result<Vec<String>, String> {
    let inner = value
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(value);
    Ok(inner
        .split(',')
        .map(|alias| unquote(alias.trim()).to_string())
        .collect())
}

/// Strip one layer of matching single or double quotes.
fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

/// One file that could not be turned into a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveryDiagnostic {
    pub path: PathBuf,
    pub diagnostic: String,
    pub failure: agena_failure::UserProblem,
}

/// The result of one discovery pass.
#[derive(Debug, Clone, Default)]
pub(crate) struct DiscoveryReport {
    pub commands: Vec<DiscoveredCommand>,
    pub diagnostics: Vec<DiscoveryDiagnostic>,
}

/// Standard skill-package roots, ordered from lower to higher precedence.
/// Later roots replace earlier commands with the same canonical name.
pub(crate) fn default_roots(workspace: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = agena_home_dir() {
        roots.push(home.join("skills"));
    }
    if let Some(workspace) = workspace {
        roots.push(workspace.join(".agena/skills"));
    }
    roots
}

/// Standard command-file roots, ordered from lower to higher precedence.
pub(crate) fn default_command_roots(workspace: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = agena_home_dir() {
        roots.push(home.join("commands"));
    }
    if let Some(workspace) = workspace {
        roots.push(workspace.join(".agena/commands"));
    }
    roots
}

fn agena_home_dir() -> Option<PathBuf> {
    Some(
        agena_storage_layout::StorageLayout::current()
            .root()
            .to_path_buf(),
    )
}

/// Scan skill packages: every `SKILL.md` under the given roots.
pub(crate) fn scan_packages_filtered(
    roots: &[PathBuf],
    access: Option<agena_tool::path_filter::PathPredicate>,
) -> DiscoveryReport {
    scan_matching(
        roots,
        |path| path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md"),
        |path| DiscoveredCommand::from_package_path(path),
        access,
    )
}

/// Scan command files: every `.md` under the given roots, except the
/// `SKILL.md` of a package.
///
/// The two scans are separate because a package keeps resources beside its
/// document and a bare command file does not, so a root that is listed for both
/// (or nested inside the other) must not report the same document twice.
pub(crate) fn scan_commands_filtered(
    roots: &[PathBuf],
    access: Option<agena_tool::path_filter::PathPredicate>,
) -> DiscoveryReport {
    scan_matching(
        roots,
        |path| {
            path.file_name().and_then(|name| name.to_str()) != Some("SKILL.md")
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        },
        |path| DiscoveredCommand::from_command_path(path),
        access,
    )
}

fn scan_matching(
    roots: &[PathBuf],
    matches_file: impl Fn(&Path) -> bool,
    load: impl Fn(&Path) -> DiscoveryResult<DiscoveredCommand>,
    access: Option<agena_tool::path_filter::PathPredicate>,
) -> DiscoveryReport {
    let mut commands = Vec::new();
    let mut diagnostics = Vec::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let mut builder = WalkBuilder::new(root);
        builder
            .follow_links(false)
            .max_depth(Some(MAX_DISCOVERY_DEPTH))
            .hidden(false)
            .ignore(false)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .parents(false)
            .threads(1);
        if let Some(access) = access.clone() {
            builder.filter_entry(move |entry| {
                !entry.file_type().is_some_and(|kind| kind.is_dir()) || access.allows(entry.path())
            });
        }
        for (entry_index, entry) in builder.build().enumerate() {
            if entry_index >= MAX_DISCOVERY_ENTRIES_PER_ROOT {
                diagnostics.push(discovery_diagnostic(
                    root.clone(),
                    format!(
                        "command discovery stopped after {MAX_DISCOVERY_ENTRIES_PER_ROOT} entries"
                    ),
                ));
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    diagnostics.push(discovery_diagnostic(root.clone(), error.to_string()));
                    continue;
                }
            };
            let path = entry.path();
            if !entry.file_type().is_some_and(|kind| kind.is_file()) || !matches_file(path) {
                continue;
            }
            if access.as_ref().is_some_and(|access| !access.allows(path)) {
                continue;
            }
            match load(path) {
                Ok(command) => commands.push(command),
                Err(error) => {
                    diagnostics.push(discovery_diagnostic(path.to_path_buf(), error.to_string()));
                }
            }
        }
    }
    DiscoveryReport {
        commands,
        diagnostics,
    }
}

fn discovery_diagnostic(path: PathBuf, diagnostic: String) -> DiscoveryDiagnostic {
    use agena_failure::{
        Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility,
        RecoveryDirective, RetryDirective, UserPresentation,
    };

    let failure = Failure::new(
        FailureCode::new("commands.discovery_item_failed"),
        FailureCategory::InvalidInput,
        FailureResponsibility::Caller,
        RetryDirective::CorrectInput,
        RecoveryDirective::OpenSettings,
        FailureImpact::PartialSuccess,
        UserPresentation::new(
            "commands-discovery-item-failed",
            "A command could not be loaded. Review its definition.",
        ),
    );
    tracing::warn!(
        target: "agena_commands::discovery",
        failure_id = %failure.id,
        path = %path.display(),
        diagnostic = %diagnostic,
        "skipping invalid or unreadable discovered command"
    );
    DiscoveryDiagnostic {
        path,
        diagnostic,
        failure: failure.into(),
    }
}

fn read_text_file_bounded(path: &Path, max_bytes: usize) -> DiscoveryResult<String> {
    let file = agena_tool::file_io::open_regular_file(path)?;
    let metadata = file.metadata()?;
    let capacity = usize::try_from(metadata.len().min(max_bytes as u64)).unwrap_or(max_bytes);
    let mut bytes = Vec::with_capacity(capacity);
    (&file)
        .take((max_bytes as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(DiscoveryError::ResourceTooLarge {
            path: path.display().to_string(),
            limit: max_bytes,
        });
    }
    let after = file.metadata()?;
    if after.len() != bytes.len() as u64
        || metadata.len() != after.len()
        || metadata.modified().ok() != after.modified().ok()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "command resource changed while reading",
        )
        .into());
    }
    String::from_utf8(bytes).map_err(|_| DiscoveryError::ResourceNotText {
        path: path.display().to_string(),
    })
}
