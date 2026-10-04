//! `agena.commands` — Agena's command surface, and the bridge that projects
//! skill packages from other agent ecosystems into it.
//!
//! Two things live here, and they are the same thing seen from two sides:
//!
//! 1. **The built-in commands.** Every slash command a client offers locally
//!    (`/new`, `/settings`, …) is declared once, as an ordinary plugin command
//!    whose target is [`CommandTarget::Client`]. The host publishes the
//!    declaration through the plugin surface catalog, so the terminal UI and
//!    the web client render the same names, aliases, usage lines and
//!    descriptions without either one keeping its own hand-copied table.
//!
//! 2. **Skill packages, projected as commands.** Agena has no skills of its
//!    own: `skill` is a vocabulary other agents use, and this is the one place
//!    it is read. A skill package on disk — a `SKILL.md` under a skill root, or
//!    a plain `.md` under a command root, or a `PluginSkillDefinition` another
//!    plugin declared in its manifest — is registered as an ordinary command
//!    whose handler is [`HANDLER_RUN`]. Running it resolves the package's
//!    instructions and returns them as an [`CommandHostEffect::InsertPrompt`],
//!    so the composer receives the text the same way for every command alike.
//!
//! The declaration is the contract. A `Client` target names an action rather
//! than a handler, because the client that renders the command palette is the
//! only component that can run it — the server never executes those commands
//! and refuses to (`CommandStatus::Unavailable`) if an invocation ever reaches
//! it. Built-in text lives in each client's message catalog, so those
//! declarations carry `summary_key` rather than literal descriptions; clients
//! resolve the key and fall back to the title when a locale has not translated
//! it yet.
//!
//! Discovered packages are registered through the host's dynamic command
//! registry rather than declared in the manifest, because the manifest is
//! captured once, before `init`, and a filesystem scan is only truthful at the
//! moment it runs. The synchronization point is the `tool.definition` hook:
//! its callback context carries no session, so entries land at the same global
//! layer `resolve_command` reads, and it runs inside a real host callback
//! (with a valid authority token) — which is what the registry requires.

mod discovery;

use portable_atomic::AtomicU64;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use agena_plugin_host::sdk::host_api::{HostCommandRegisterRequest, HostCommandRemoveRequest};
use agena_plugin_host::sdk::{
    CommandDefinition, CommandDocs, CommandHostEffect, CommandInvokeInput, CommandResult,
    CommandTarget, HookSubscription, HostClient, InitContext, InitOutcome, Plugin, PluginError,
    PluginKey, PluginManifest, PluginSkillDefinition, Result as SdkResult, SettingsContract,
    SettingsNode, SettingsNodeKind, ToolContract, ToolDefinition, ToolDefinitionInput,
    ToolDefinitionPatch, ToolDocs, ToolInvokeInput, ToolInvokeOutput, ToolRuntimePolicy,
    ToolStreamingMode, ToolTag, macro_support,
};

use discovery::{
    DiscoveredCommand, DiscoveryDiagnostic, MAX_COMMAND_DOCUMENT_BYTES, default_command_roots,
    default_roots, scan_commands_with_diagnostics, scan_packages_with_diagnostics,
};

pub(crate) const COMMANDS_PLUGIN_ID: &str = "agena.commands";

/// The handler name every discovered package is registered under.
const HANDLER_RUN: &str = "run";

/// Group and category every discovered package is published with, so a
/// composer can tell external packages apart from the built-ins it implements
/// itself without knowing their names.
const DISCOVERED_GROUP: &str = "Commands";
const DISCOVERED_CATEGORY: &str = "Package";

/// One built-in command, in palette order.
///
/// `action` is the client-side vocabulary name; `slash` is what a user types.
/// `usage` is the argument shape shown in the palette (empty when the command
/// takes none), and `summary_key` is the Fluent / vue-i18n key for its one-line
/// description.
struct BuiltInCommand {
    action: &'static str,
    slash: &'static str,
    aliases: &'static [&'static str],
    usage: Option<&'static str>,
    summary_key: &'static str,
}

/// Palette order is part of the contract: both clients render this sequence
/// for a blank query, so a new built-in belongs at the position a user would
/// look for it rather than wherever it was appended.
const BUILT_IN_COMMANDS: &[BuiltInCommand] = &[
    BuiltInCommand {
        action: "help",
        slash: "/help",
        aliases: &["?"],
        usage: None,
        summary_key: "command-help-summary",
    },
    BuiltInCommand {
        action: "commands",
        slash: "/commands",
        aliases: &["palette"],
        usage: None,
        summary_key: "command-commands-summary",
    },
    BuiltInCommand {
        action: "new",
        slash: "/new",
        aliases: &["clear"],
        usage: None,
        summary_key: "command-new-summary",
    },
    BuiltInCommand {
        action: "sessions",
        slash: "/sessions",
        aliases: &[],
        usage: None,
        summary_key: "command-sessions-summary",
    },
    BuiltInCommand {
        action: "hub",
        slash: "/hub",
        aliases: &[],
        usage: None,
        summary_key: "command-hub-summary",
    },
    BuiltInCommand {
        action: "lineage",
        slash: "/lineage",
        aliases: &["branch-history", "branches"],
        usage: None,
        summary_key: "command-lineage-summary",
    },
    BuiltInCommand {
        action: "rewind",
        slash: "/rewind",
        aliases: &["backtrack"],
        usage: None,
        summary_key: "command-rewind-summary",
    },
    BuiltInCommand {
        action: "rename",
        slash: "/rename",
        aliases: &["title"],
        usage: None,
        summary_key: "command-rename-summary",
    },
    BuiltInCommand {
        action: "favorite",
        slash: "/favorite",
        aliases: &["star"],
        usage: None,
        summary_key: "command-favorite-summary",
    },
    BuiltInCommand {
        action: "timeline",
        slash: "/timeline",
        aliases: &["events"],
        usage: None,
        summary_key: "command-timeline-summary",
    },
    BuiltInCommand {
        action: "settings",
        slash: "/settings",
        aliases: &["config"],
        usage: None,
        summary_key: "command-settings-summary",
    },
    BuiltInCommand {
        action: "model",
        slash: "/model",
        aliases: &[],
        usage: None,
        summary_key: "command-model-summary",
    },
    BuiltInCommand {
        action: "commit",
        slash: "/commit",
        aliases: &[],
        usage: Some("<message>"),
        summary_key: "command-commit-summary",
    },
    BuiltInCommand {
        action: "pr",
        slash: "/pr",
        aliases: &[],
        usage: Some("<title> [--body <text>] [--base <branch>] [--head <branch>]"),
        summary_key: "command-pr-summary",
    },
    BuiltInCommand {
        action: "export",
        slash: "/export",
        aliases: &["save"],
        usage: Some("[path]"),
        summary_key: "command-export-summary",
    },
    BuiltInCommand {
        action: "pager",
        slash: "/pager",
        aliases: &["view", "less"],
        usage: None,
        summary_key: "command-pager-summary",
    },
    BuiltInCommand {
        action: "continue",
        slash: "/continue",
        aliases: &["resume-run"],
        usage: None,
        summary_key: "command-continue-summary",
    },
    BuiltInCommand {
        action: "compact",
        slash: "/compact",
        aliases: &["compress", "summarize"],
        usage: None,
        summary_key: "command-compact-summary",
    },
    BuiltInCommand {
        action: "user-input",
        slash: "/user-input",
        aliases: &["reply"],
        usage: None,
        summary_key: "command-user-input-summary",
    },
    BuiltInCommand {
        action: "allow",
        slash: "/allow",
        aliases: &[],
        usage: None,
        summary_key: "command-allow-summary",
    },
    BuiltInCommand {
        action: "allow-always",
        slash: "/allow-always",
        aliases: &[],
        usage: None,
        summary_key: "command-allow-always-summary",
    },
    BuiltInCommand {
        action: "deny",
        slash: "/deny",
        aliases: &[],
        usage: None,
        summary_key: "command-deny-summary",
    },
    BuiltInCommand {
        action: "deny-always",
        slash: "/deny-always",
        aliases: &[],
        usage: None,
        summary_key: "command-deny-always-summary",
    },
    BuiltInCommand {
        action: "attach",
        slash: "/attach",
        aliases: &["file"],
        usage: None,
        summary_key: "command-attach-summary",
    },
    BuiltInCommand {
        action: "download",
        slash: "/download",
        aliases: &["dl"],
        usage: Some("<workspace-path>"),
        summary_key: "command-download-summary",
    },
    BuiltInCommand {
        action: "editor",
        slash: "/editor",
        aliases: &["edit"],
        usage: None,
        summary_key: "command-editor-summary",
    },
    BuiltInCommand {
        action: "image",
        slash: "/image",
        aliases: &[],
        usage: None,
        summary_key: "command-image-summary",
    },
    BuiltInCommand {
        action: "paste",
        slash: "/paste",
        aliases: &["clipboard"],
        usage: None,
        summary_key: "command-paste-summary",
    },
    BuiltInCommand {
        action: "copy",
        slash: "/copy",
        aliases: &["yank"],
        usage: None,
        summary_key: "command-copy-summary",
    },
    BuiltInCommand {
        action: "copy-message",
        slash: "/copy-message",
        aliases: &["copy-last", "copy-assistant"],
        usage: None,
        summary_key: "command-copy-message-summary",
    },
    BuiltInCommand {
        action: "copy-visible",
        slash: "/copy-visible",
        aliases: &[],
        usage: None,
        summary_key: "command-copy-visible-summary",
    },
    BuiltInCommand {
        action: "fork",
        slash: "/fork",
        aliases: &["branch"],
        usage: None,
        summary_key: "command-fork-summary",
    },
    BuiltInCommand {
        action: "children",
        slash: "/children",
        aliases: &["child"],
        usage: None,
        summary_key: "command-children-summary",
    },
    BuiltInCommand {
        action: "parent",
        slash: "/parent",
        aliases: &[],
        usage: None,
        summary_key: "command-parent-summary",
    },
    BuiltInCommand {
        action: "diagnostics",
        slash: "/diagnostics",
        aliases: &["feedback"],
        usage: None,
        summary_key: "command-diagnostics-summary",
    },
    BuiltInCommand {
        action: "status",
        slash: "/status",
        aliases: &[],
        usage: None,
        summary_key: "command-status-summary",
    },
    BuiltInCommand {
        action: "usage",
        slash: "/usage",
        aliases: &["stats", "analytics"],
        usage: None,
        summary_key: "command-usage-summary",
    },
    BuiltInCommand {
        action: "activities",
        slash: "/activities",
        aliases: &["tasks"],
        usage: None,
        summary_key: "command-activities-summary",
    },
    BuiltInCommand {
        action: "background",
        slash: "/background",
        aliases: &[],
        usage: None,
        summary_key: "command-background-summary",
    },
    BuiltInCommand {
        action: "plan",
        slash: "/plan",
        aliases: &["plan-view", "show-plan"],
        usage: None,
        summary_key: "command-plan-summary",
    },
    BuiltInCommand {
        action: "side",
        slash: "/side",
        aliases: &["aside"],
        usage: Some("[question]"),
        summary_key: "command-side-summary",
    },
    BuiltInCommand {
        action: "btw",
        slash: "/btw",
        aliases: &[],
        usage: Some("[question]"),
        summary_key: "command-btw-summary",
    },
];

fn built_in_commands() -> Vec<CommandDefinition> {
    BUILT_IN_COMMANDS
        .iter()
        .map(|built_in| CommandDefinition {
            id: built_in.action.to_string(),
            title: built_in_summary_key_title(built_in.summary_key),
            group: GROUP.to_string(),
            category: Some(CATEGORY.to_string()),
            slash: Some(built_in.slash.to_string()),
            aliases: built_in
                .aliases
                .iter()
                .map(|alias| (*alias).to_string())
                .collect(),
            // Built-in commands document themselves through the client's
            // message catalog, so only the key is declared here. Usage, where
            // the command takes arguments, is the one literal: it is a shape,
            // not prose, and both clients render it verbatim.
            docs: CommandDocs {
                usage: built_in.usage.map(str::to_string),
                summary_key: Some(built_in.summary_key.to_string()),
                ..CommandDocs::default()
            },
            input: SettingsContract::empty_object("No input", ""),
            target: CommandTarget::Client {
                action: built_in.action.to_string(),
            },
        })
        .collect()
}

/// The title a client shows before it has resolved the summary key. Deriving
/// it from the key keeps the fallback honest instead of inventing display copy
/// in the declaration.
fn built_in_summary_key_title(summary_key: &str) -> String {
    summary_key
        .trim_start_matches("command-")
        .trim_end_matches("-summary")
        .to_string()
}

/// Both clients group built-ins together; `category` distinguishes them from
/// plugin-provided commands in the same group.
const GROUP: &str = "Built-in";
const CATEGORY: &str = "Client";

// ── declared command packages ──────────────────────────────────────────────

/// One command package compiled into the executable.
struct BundledCommand {
    name: &'static str,
    description: &'static str,
    aliases: &'static [&'static str],
    instructions: &'static str,
}

/// The packages every Agena install ships with. The instructions live in
/// markdown so the text stays readable and diffable instead of being buried in
/// a Rust string literal.
const BUNDLED_COMMANDS: &[BundledCommand] = &[
    BundledCommand {
        name: "batch",
        description: "Execute independent repository tasks with Git worktree isolation and delegated agents",
        aliases: &[],
        instructions: include_str!("../../../assets/commands/batch.md"),
    },
    BundledCommand {
        name: "debug",
        description: "Diagnose a reproducible failure from logs, state and source evidence",
        aliases: &[],
        instructions: include_str!("../../../assets/commands/debug.md"),
    },
    BundledCommand {
        name: "doctor",
        description: "Diagnose Agena runtime, provider, plugin, Skill, MCP and project tooling health",
        aliases: &[],
        instructions: include_str!("../../../assets/commands/doctor.md"),
    },
    BundledCommand {
        name: "imagegen",
        description: "Generate or edit images with ordinary ChatGPT and Gemini execution tools",
        aliases: &["image-generate", "image-edit"],
        instructions: include_str!("../../../assets/commands/imagegen.md"),
    },
    BundledCommand {
        name: "init",
        description: "Initialise an AGENA.md describing the codebase",
        aliases: &["bootstrap"],
        instructions: include_str!("../../../assets/commands/init.md"),
    },
    BundledCommand {
        name: "plugin_creator",
        description: "Scaffold and verify an Agena plugin using the repository SDK",
        aliases: &["create-plugin"],
        instructions: include_str!("../../../assets/commands/plugin_creator.md"),
    },
    BundledCommand {
        name: "review",
        description: "Review the current branch as a senior code reviewer",
        aliases: &[],
        instructions: include_str!("../../../assets/commands/review.md"),
    },
    BundledCommand {
        name: "run",
        description: "Identify and start the current project with a reusable shell process",
        aliases: &["start"],
        instructions: include_str!("../../../assets/commands/run.md"),
    },
    BundledCommand {
        name: "run_skill_generator",
        description: "Turn a proven one-off workflow into a reusable validated Skill package",
        aliases: &["skill-from-run"],
        instructions: include_str!("../../../assets/commands/run_skill_generator.md"),
    },
    BundledCommand {
        name: "security_review",
        description: "Audit the current branch for security regressions",
        aliases: &["security-review"],
        instructions: include_str!("../../../assets/commands/security_review.md"),
    },
    BundledCommand {
        name: "simplify",
        description: "Reduce unnecessary complexity without changing behavior",
        aliases: &[],
        instructions: include_str!("../../../assets/commands/simplify.md"),
    },
    BundledCommand {
        name: "skill_creator",
        description: "Create or update a validated Agena Skill package",
        aliases: &["create-skill"],
        instructions: include_str!("../../../assets/commands/skill_creator.md"),
    },
    BundledCommand {
        name: "skill_installer",
        description: "Install Skills from a trusted local or Git repository source",
        aliases: &["install-skill"],
        instructions: include_str!("../../../assets/commands/skill_installer.md"),
    },
    BundledCommand {
        name: "verify",
        description: "Run the smallest sufficient validation for the current change",
        aliases: &["check"],
        instructions: include_str!("../../../assets/commands/verify.md"),
    },
];

fn bundled_command_definitions() -> Vec<PluginSkillDefinition> {
    BUNDLED_COMMANDS
        .iter()
        .map(|command| PluginSkillDefinition {
            name: command.name.to_string(),
            description: command.description.to_string(),
            instructions: command.instructions.trim().to_string(),
            aliases: command
                .aliases
                .iter()
                .map(|alias| (*alias).to_string())
                .collect(),
        })
        .collect()
}

/// One declared package in the shape the capability manifest reports it.
///
/// The identity is the same `content_hash` the catalog uses, so a package's
/// entry in the shipped capability inventory and its entry in the live command
/// catalog can never disagree.
pub(crate) struct DeclaredCommand {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) content_sha256: String,
}

/// The declared packages, read from the same declarations the plugin ships in
/// its manifest rather than from a second hand-maintained list.
pub(crate) fn declared_commands() -> Vec<DeclaredCommand> {
    bundled_command_definitions()
        .into_iter()
        .map(|definition| {
            let command = DiscoveredCommand::bundled(
                discovery::CommandFrontmatter {
                    name: definition.name,
                    description: definition.description,
                    aliases: definition.aliases,
                },
                definition.instructions,
            );
            DeclaredCommand {
                name: command.frontmatter.name.clone(),
                description: command.frontmatter.description.clone(),
                aliases: command.frontmatter.aliases.clone(),
                content_sha256: command.content_hash(),
            }
        })
        .collect()
}

// ── dynamic command registry synchronization ───────────────────────────────

/// The command ids this plugin has registered in the host's dynamic command
/// registry, with the content hash each one was registered under.
///
/// The host registry has no bulk-replace, so a rescan diffs this set against
/// the new catalog: ids that disappeared are removed, ids whose content hash
/// changed are removed and re-registered, and an unchanged rescan writes
/// nothing at all.
#[derive(Debug, Clone, Default)]
struct RegisteredCommands {
    by_command_id: BTreeMap<String, String>,
}

/// Upper bound on registry mutations in a single synchronization pass, so a
/// very large package directory cannot turn one catalog refresh into an
/// unbounded burst of host callbacks. The remainder converges on later passes.
const MAX_SYNCS_PER_PASS: usize = 256;

/// What one synchronization pass did, for the diagnostics line the
/// `tool.definition` patch carries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ResyncOutcome {
    registered: usize,
    removed: usize,
    collisions: usize,
    diagnostics: usize,
}

impl ResyncOutcome {
    fn summary(&self) -> String {
        format!(
            "{} registered, {} removed, {} refused, {} diagnostic(s)",
            self.registered, self.removed, self.collisions, self.diagnostics
        )
    }
}

// ── plugin state ───────────────────────────────────────────────────────────

/// Owns the platform watcher for as long as the static plugin lives. `notify`
/// invokes the callback on its own worker context; an atomic counter is
/// sufficient because events only invalidate the next catalog build and never
/// carry content into the model context.
struct CommandCatalogWatcher {
    _watcher: RecommendedWatcher,
    watched_paths: Vec<PathBuf>,
}

/// Source of one discovered package, for the catalog payload and for deciding
/// whether it exposes filesystem-backed resources.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PackageOrigin {
    Declared,
    Filesystem,
    Plugin {
        plugin_id: String,
        plugin_version: String,
    },
}

impl PackageOrigin {
    fn source_label(&self) -> String {
        match self {
            Self::Declared => "declared".to_string(),
            Self::Filesystem => "filesystem".to_string(),
            Self::Plugin {
                plugin_id,
                plugin_version,
            } => format!("plugin:{plugin_id}@{plugin_version}"),
        }
    }

    fn supports_resources(&self) -> bool {
        matches!(self, Self::Filesystem)
    }
}

/// One entry of the effective command package catalog.
#[derive(Debug, Clone)]
struct PackageEntry {
    command: DiscoveredCommand,
    origin: PackageOrigin,
}

#[derive(Debug, Clone, Default)]
struct PackageCatalog {
    packages: BTreeMap<String, PackageEntry>,
    diagnostics: Vec<DiscoveryDiagnostic>,
}

#[derive(Debug, Clone)]
struct CatalogRefresh {
    catalog: PackageCatalog,
    fingerprint: String,
    generation: u64,
    changed: bool,
}

#[derive(Debug, Clone, Default)]
struct CatalogState {
    fingerprint: Option<String>,
    generation: u64,
}

#[derive(Debug, Clone)]
struct DefinitionCatalogSnapshot {
    catalog: PackageCatalog,
    watcher_generation: u64,
    captured_at: Instant,
}

/// One catalog build feeds every `tool.definition` hook in the same request.
/// The watcher invalidates ordinary filesystem changes; the short TTL keeps
/// request-driven discovery correct when the watcher is disabled or an event
/// is coalesced by the operating system.
const DEFINITION_CATALOG_CACHE_TTL: Duration = Duration::from_millis(250);

/// Plugin-owned policy for filesystem-backed packages. The regular roots remain
/// enabled by default; extra roots are deliberately workspace-relative so a
/// project configuration cannot silently turn an arbitrary user directory
/// into model-visible prompt content.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct CommandsPluginConfig {
    /// Canonical package names hidden from discovery.
    disabled: Vec<String>,
    /// Extra workspace-relative directories containing `SKILL.md` packages.
    additional_roots: Vec<PathBuf>,
    /// Extra workspace-relative directories containing command `.md` files.
    additional_command_roots: Vec<PathBuf>,
    /// Cross-platform OS watcher used only to invalidate the filesystem
    /// catalog. Discovery still happens at a normal request boundary; watcher
    /// events never inject instructions on their own.
    watcher: CommandsWatcherConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct CommandsWatcherConfig {
    enabled: bool,
}

impl Default for CommandsWatcherConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl CommandsPluginConfig {
    fn validate_for_workspace(&self, workspace_root: &Path) -> SdkResult<()> {
        let canonical_workspace = workspace_root.canonicalize().map_err(|error| {
            PluginError::invalid_params(format!(
                "commands config cannot canonicalize workspace '{}': {error}",
                workspace_root.display()
            ))
        })?;
        let mut seen = std::collections::BTreeSet::new();
        for name in &self.disabled {
            let normalized = name.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                return Err(PluginError::invalid_params(
                    "commands config disabled entries cannot be blank",
                ));
            }
            if !seen.insert(normalized) {
                return Err(PluginError::invalid_params(format!(
                    "commands config lists the same disabled name more than once: '{name}'"
                )));
            }
        }
        for (label, roots) in [
            ("additional_roots", &self.additional_roots),
            ("additional_command_roots", &self.additional_command_roots),
        ] {
            for root in roots {
                if root.as_os_str().is_empty()
                    || root.is_absolute()
                    || root
                        .components()
                        .any(|component| matches!(component, std::path::Component::ParentDir))
                {
                    return Err(PluginError::invalid_params(format!(
                        "commands config {label} entries must be non-empty workspace-relative paths without '..'"
                    )));
                }
                let candidate = workspace_root.join(root);
                if !candidate.starts_with(workspace_root) {
                    return Err(PluginError::invalid_params(format!(
                        "commands config {label} entry '{}' escapes the workspace",
                        root.display()
                    )));
                }
                // A lexical workspace-relative path may still resolve through a
                // symlink outside the workspace. Existing roots are
                // canonicalized at config-load time so that cannot silently
                // turn an arbitrary user directory into model-visible
                // instructions. A non-existent root is harmless until a later
                // config reload validates it again.
                if candidate.exists() {
                    let canonical_candidate = candidate.canonicalize().map_err(|error| {
                        PluginError::invalid_params(format!(
                            "commands config cannot canonicalize {label} entry '{}': {error}",
                            root.display()
                        ))
                    })?;
                    if !canonical_candidate.starts_with(&canonical_workspace) {
                        return Err(PluginError::invalid_params(format!(
                            "commands config {label} entry '{}' resolves outside the workspace",
                            root.display()
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    fn disabled_name(&self, name: &str) -> bool {
        self.disabled
            .iter()
            .any(|disabled| disabled.trim().eq_ignore_ascii_case(name.trim()))
    }
}

/// Owns the mutable half of the plugin. Every field is behind an `Arc` so the
/// blocking worker can take a cheap clone and never touch plugin state from a
/// Tokio worker thread.
#[derive(Clone)]
pub(crate) struct CommandsPlugin {
    workspace_root: Arc<OnceLock<PathBuf>>,
    config: Arc<OnceLock<CommandsPluginConfig>>,
    catalog_state: Arc<Mutex<CatalogState>>,
    definition_catalog: Arc<Mutex<Option<DefinitionCatalogSnapshot>>>,
    registered: Arc<Mutex<RegisteredCommands>>,
    watcher: Arc<Mutex<Option<CommandCatalogWatcher>>>,
    watcher_generation: Arc<AtomicU64>,
}

fn commands_settings_metadata() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        (
            "",
            "Commands Plugin Config",
            "Controls discovery policy for filesystem-backed command packages.",
        ),
        (
            "/disabled",
            "Disabled Commands",
            "Canonical package or command names to hide from list and read.",
        ),
        (
            "/additional_roots",
            "Additional Package Roots",
            "Workspace-relative directories scanned after the standard roots.",
        ),
        (
            "/additional_command_roots",
            "Additional Command Roots",
            "Workspace-relative directories scanned after the standard command roots.",
        ),
        (
            "/watcher",
            "Filesystem Watcher",
            "Use the platform filesystem watcher to invalidate the command catalog after on-disk changes.",
        ),
        (
            "/watcher/enabled",
            "Enabled",
            "Disable only the OS-level watcher; request-driven discovery remains active for every command tool.",
        ),
    ]
}

/// The settings contract the plugin publishes, with the presentation metadata
/// applied. A hand-written `impl Plugin` cannot use the `settings_metadata`
/// argument of `#[agena_plugin]`, so the same two steps happen here in the same
/// order the macro would take them.
fn commands_settings_contract() -> SettingsContract {
    agena_plugin_host::sdk::decorate_settings_contract(
        macro_support::settings_contract_for_default(CommandsPluginConfig::default()),
        commands_settings_metadata(),
    )
    .expect("commands settings metadata must address the typed contract")
}

// ── tool input contracts ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CommandsListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    #[serde(default)]
    verbose: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CommandsGetInput {
    /// Canonical name, or an alias, of the command package to read.
    name: String,
}

/// A complete package document. Keeping the editor boundary at the native
/// document format lets callers preserve a package's YAML frontmatter
/// alongside its Markdown instructions instead of maintaining a second,
/// lossy model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CommandsInstallInput {
    document: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CommandsRemoveInput {
    /// Canonical name (or alias) of the workspace-managed package to remove.
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CommandsReadResourceInput {
    name: String,
    path: String,
    #[serde(default = "default_resource_limit")]
    #[schemars(range(min = 1, max = 1048576))]
    max_bytes: u32,
}

const fn default_resource_limit() -> u32 {
    262_144
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct CommandsRefreshInput {
    /// Include discovery diagnostics in the human-readable response.
    verbose: bool,
}

/// The one-field contract every discovered package is registered with, so a
/// composer can hand the whole tail of `/<name> <args>` to the handler as one
/// optional string. A bare scalar is accepted through the same contract, which
/// is what a slash invocation produces.
fn package_contract() -> SettingsContract {
    SettingsContract::new(SettingsNode {
        id: "root".to_string(),
        path: String::new(),
        title: "Arguments".to_string(),
        description: "Optional text appended to the package instructions.".to_string(),
        required: true,
        default: Some(serde_json::json!({})),
        constraints: Default::default(),
        sensitive: false,
        secret: false,
        kind: SettingsNodeKind::Object {
            fields: vec![SettingsNode {
                id: "args".to_string(),
                path: "/args".to_string(),
                title: "Arguments".to_string(),
                description: String::new(),
                required: false,
                default: None,
                constraints: Default::default(),
                sensitive: false,
                secret: false,
                kind: SettingsNodeKind::Text,
            }],
        },
    })
}

// ── tool definitions ───────────────────────────────────────────────────────

/// The tools this plugin publishes. They are built by hand rather than by the
/// `#[tool]` macro because the plugin also needs a hand-written `impl Plugin`
/// for its command bridge, and the two cannot coexist on one type.
fn command_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        tool_definition(
            "list",
            "List discovered command packages.",
            Some(
                "List discovered command packages, with paging. Each entry names the package, its source, and whether it can be edited in this workspace.",
            ),
            macro_support::json_schema_for::<CommandsListInput>(),
            &[ToolTag::Query, ToolTag::Discovery, ToolTag::ReadOnly],
        ),
        tool_definition(
            "get",
            "Read one discovered command package.",
            Some("Read one package's instructions in full and apply them to the current task."),
            macro_support::json_schema_for::<CommandsGetInput>(),
            &[ToolTag::Query, ToolTag::Discovery, ToolTag::ReadOnly],
        ),
        tool_definition(
            "install",
            "Install a workspace-managed command package document.",
            Some(
                "Writes `.agena/skills/<name>/SKILL.md` from a complete package document. Only workspace-local packages are mutable; declared, plugin and user-global packages remain read-only.",
            ),
            macro_support::json_schema_for::<CommandsInstallInput>(),
            &[ToolTag::Mutate, ToolTag::Filesystem],
        ),
        tool_definition(
            "remove",
            "Remove a workspace-managed command package document.",
            Some(
                "Deletes only `.agena/skills/<name>/SKILL.md`; declared, plugin and user-global packages cannot be removed through this tool.",
            ),
            macro_support::json_schema_for::<CommandsRemoveInput>(),
            &[ToolTag::Mutate, ToolTag::Filesystem],
        ),
        tool_definition(
            "read_resource",
            "Read a bounded UTF-8 resource contained by one package.",
            None,
            macro_support::json_schema_for::<CommandsReadResourceInput>(),
            &[ToolTag::Query, ToolTag::Filesystem, ToolTag::ReadOnly],
        ),
        tool_definition(
            "refresh",
            "Rescan filesystem-backed packages and report the catalog generation.",
            None,
            macro_support::json_schema_for::<CommandsRefreshInput>(),
            &[ToolTag::Mutate, ToolTag::Discovery, ToolTag::ReadOnly],
        ),
    ]
}

fn tool_definition(
    name: &str,
    summary: &str,
    help: Option<&str>,
    input_schema: Value,
    tags: &[ToolTag],
) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        contract: ToolContract {
            input_schema,
            output_schema: Value::Null,
        },
        docs: ToolDocs {
            before_help: None,
            after_help: None,
            summary: Some(summary.to_string()),
            help: help.map(str::to_string),
        },
        runtime: ToolRuntimePolicy {
            streaming: ToolStreamingMode::Buffered,
        },
        tags: tags.to_vec(),
    }
}

// ── the plugin ─────────────────────────────────────────────────────────────

impl CommandsPlugin {
    pub(crate) fn new() -> Self {
        Self {
            workspace_root: Arc::new(OnceLock::new()),
            config: Arc::new(OnceLock::new()),
            catalog_state: Arc::new(Mutex::new(CatalogState::default())),
            definition_catalog: Arc::new(Mutex::new(None)),
            registered: Arc::new(Mutex::new(RegisteredCommands::default())),
            watcher: Arc::new(Mutex::new(None)),
            watcher_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    fn manifest_for(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("agena", "commands", env!("CARGO_PKG_VERSION"));
        manifest.summary = Some(
            "Declare the built-in commands every Agena client renders locally, and project skills from other agent ecosystems into the same surface."
                .to_string(),
        );
        // Discovered packages are registered dynamically, but the packages
        // compiled into this executable are part of the declaration.
        manifest.skills = bundled_command_definitions();
        manifest.tools = command_tool_definitions();
        manifest.settings = Some(commands_settings_contract());
        let mut hooks = HookSubscription::TOOL_DEFINITION | HookSubscription::TOOL_INVOKE;
        hooks |= HookSubscription::INIT;
        manifest.hooks = hooks;
        manifest
    }

    /// Filesystem discovery and document I/O are synchronous APIs. Keep them
    /// off Tokio workers and make the blocking boundary explicit at the
    /// plugin edge.
    async fn run_blocking<T, F>(&self, operation: F) -> SdkResult<T>
    where
        T: Send + 'static,
        F: FnOnce(Self) -> SdkResult<T> + Send + 'static,
    {
        let plugin = self.clone();
        let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
            .acquire()
            .await
            .map_err(|error| {
                PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                    "acquire a commands plugin worker",
                    &error,
                ))
            })?;
        tokio::task::spawn_blocking(move || {
            let _worker_permit = worker_permit;
            operation(plugin)
        })
        .await
        .map_err(|error| {
            PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                "commands plugin worker failed",
                &error,
            ))
        })?
    }

    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::internal("commands plugin invoked before init"))
    }

    fn config(&self) -> CommandsPluginConfig {
        self.config.get().cloned().unwrap_or_default()
    }

    /// The only mutable package location. Discovery includes declared, user and
    /// workspace roots, but mutation must not turn this plugin into a general
    /// filesystem editor or alter another project's global configuration.
    fn managed_root(&self) -> SdkResult<PathBuf> {
        let workspace_root = self.workspace_root()?;
        let canonical_workspace = workspace_root.canonicalize().map_err(|error| {
            PluginError::internal(format!(
                "cannot canonicalize command workspace '{}': {error}",
                workspace_root.display()
            ))
        })?;
        let root = canonical_workspace.join(".agena/skills");
        std::fs::create_dir_all(&root).map_err(package_write_error)?;
        let canonical_root = root.canonicalize().map_err(package_write_error)?;
        if !canonical_root.starts_with(&canonical_workspace) {
            return Err(PluginError::invalid_params(
                "workspace package root resolves outside the workspace",
            ));
        }
        Ok(canonical_root)
    }

    fn managed_package_path(&self, name: &str) -> SdkResult<PathBuf> {
        validate_managed_package_name(name)?;
        let root = self.managed_root()?;
        Ok(root.join(name).join("SKILL.md"))
    }

    fn managed_package_document(&self, name: &str) -> SdkResult<(String, PathBuf)> {
        let path = self.managed_package_path(name)?;
        let parent = path.parent().ok_or_else(|| {
            PluginError::internal("managed package path unexpectedly has no parent directory")
        })?;
        if !parent.is_dir() || parent.is_symlink() {
            return Err(PluginError::invalid_params(format!(
                "workspace-managed package '{name}' does not exist"
            )));
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(package_write_error)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(PluginError::invalid_params(format!(
                "workspace-managed package '{name}' is not a regular SKILL.md file"
            )));
        }
        let canonical_root = self.managed_root()?;
        let canonical_path = path.canonicalize().map_err(package_write_error)?;
        if !canonical_path.starts_with(&canonical_root) {
            return Err(PluginError::invalid_params(
                "workspace-managed package resolves outside the workspace",
            ));
        }
        let document = discovery::read_command_document(&canonical_path)
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        Ok((document, canonical_path))
    }

    /// Install one workspace-managed package document.
    fn install_managed_package(&self, document: &str) -> SdkResult<PackageWriteResult> {
        let package = parse_managed_package_document(document)?;
        let name = package.frontmatter.name;
        let path = self.managed_package_path(name.as_str())?;
        let lock_path = path.clone();
        agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&lock_path), || {
            if path.exists() {
                // Reinstalling is the useful operation for an editor or a
                // setup script; an install that refuses to run twice would
                // make both write a create-or-replace dance of their own.
                let (existing, _) = self.managed_package_document(name.as_str())?;
                check_package_revision(existing.as_str(), None)?;
                write_package_document(path.as_path(), document)?;
                return Ok(PackageWriteResult {
                    name,
                    path,
                    operation: "updated",
                    revision: Some(package_document_revision(document)),
                    warning: None,
                });
            }
            let parent = path.parent().ok_or_else(|| {
                PluginError::internal("managed package path unexpectedly has no parent directory")
            })?;
            std::fs::create_dir_all(parent).map_err(package_write_error)?;
            let canonical_root = self.managed_root()?;
            let canonical_parent = parent.canonicalize().map_err(package_write_error)?;
            if !canonical_parent.starts_with(&canonical_root) || parent.is_symlink() {
                return Err(PluginError::invalid_params(
                    "workspace-managed package directory resolves outside the workspace",
                ));
            }
            create_package_document(path.as_path(), document)?;
            Ok(PackageWriteResult {
                name,
                path,
                operation: "created",
                revision: Some(package_document_revision(document)),
                warning: None,
            })
        })
        .map_err(package_write_error)?
    }

    fn remove_managed_package(&self, requested_name: &str) -> SdkResult<PackageWriteResult> {
        let catalog = self.discovered_catalog()?;
        let (canonical_name, _) = Self::resolve_package(&catalog.packages, requested_name)?;
        let lock_path = self.managed_package_path(canonical_name)?;
        agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&lock_path), || {
            let (existing, path) = self.managed_package_document(canonical_name)?;
            check_package_revision(existing.as_str(), None)?;
            std::fs::remove_file(&path).map_err(package_write_error)?;
            // Empty-directory cleanup is secondary: a removed document must
            // not be reported as uncommitted because cleanup failed.
            let warning = path.parent().and_then(|parent| match parent.read_dir() {
                Ok(mut entries) => {
                    if entries.next().is_none() {
                        std::fs::remove_dir(parent).err().map(|error| {
                            format!("Package removed; empty directory cleanup failed: {error}")
                        })
                    } else {
                        None
                    }
                }
                Err(error) => Some(format!(
                    "Package removed; directory inspection failed: {error}"
                )),
            });
            Ok(PackageWriteResult {
                name: canonical_name.to_owned(),
                path,
                operation: "removed",
                revision: None,
                warning,
            })
        })
        .map_err(package_write_error)?
    }

    fn complete_package_write(&self, mut result: PackageWriteResult) -> ToolInvokeOutput {
        let refreshed = self
            .invalidate_definition_catalog()
            .and_then(|()| self.refresh_catalog());
        match refreshed {
            Ok(refresh) => package_write_output(result, Some(refresh.generation), refresh.changed),
            Err(error) => {
                result.warning = Some(format!(
                    "Package {} committed, but catalog refresh failed: {}",
                    result.operation, error.failure.user.fallback
                ));
                package_write_output(result, None, false)
            }
        }
    }

    fn discovered_catalog(&self) -> SdkResult<PackageCatalog> {
        Ok(self.refresh_catalog()?.catalog)
    }

    fn definition_catalog(&self) -> SdkResult<PackageCatalog> {
        let watcher_generation = self.watcher_generation.load(Ordering::Acquire);
        let mut snapshot = self
            .definition_catalog
            .lock()
            .map_err(|_| PluginError::internal("commands definition-catalog lock poisoned"))?;
        if let Some(cached) = snapshot.as_ref()
            && cached.watcher_generation == watcher_generation
            && cached.captured_at.elapsed() <= DEFINITION_CATALOG_CACHE_TTL
        {
            return Ok(cached.catalog.clone());
        }

        // Keep this non-async mutex while scanning so concurrent definition
        // requests coalesce into one blocking job. This method only runs on
        // Tokio's blocking pool and never waits on async work while locked.
        let catalog = self.discovered_catalog()?;
        *snapshot = Some(DefinitionCatalogSnapshot {
            catalog: catalog.clone(),
            watcher_generation,
            captured_at: Instant::now(),
        });
        Ok(catalog)
    }

    async fn definition_catalog_async(&self) -> SdkResult<PackageCatalog> {
        self.run_blocking(|plugin| plugin.definition_catalog())
            .await
    }

    fn invalidate_definition_catalog(&self) -> SdkResult<()> {
        *self
            .definition_catalog
            .lock()
            .map_err(|_| PluginError::internal("commands definition-catalog lock poisoned"))? =
            None;
        Ok(())
    }

    fn filesystem_roots(
        workspace_root: &Path,
        config: &CommandsPluginConfig,
    ) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let workspace =
            Some(workspace_root.to_path_buf()).filter(|path| !path.as_os_str().is_empty());
        let mut roots = default_roots(workspace.as_deref());
        let mut command_roots = default_command_roots(workspace.as_deref());
        roots.extend(
            config
                .additional_roots
                .iter()
                .map(|root| workspace_root.join(root)),
        );
        command_roots.extend(
            config
                .additional_command_roots
                .iter()
                .map(|root| workspace_root.join(root)),
        );
        roots.sort();
        roots.dedup();
        command_roots.sort();
        command_roots.dedup();
        (roots, command_roots)
    }

    fn discovered_catalog_for_workspace_with_config(
        workspace_root: &Path,
        config: &CommandsPluginConfig,
    ) -> PackageCatalog {
        let (roots, command_roots) = Self::filesystem_roots(workspace_root, config);

        let package_report = scan_packages_with_diagnostics(&roots);
        let command_report = scan_commands_with_diagnostics(&command_roots);

        let mut packages: BTreeMap<String, PackageEntry> = bundled_command_definitions()
            .into_iter()
            .map(|definition| {
                let name = definition.name.clone();
                let command = DiscoveredCommand::bundled(
                    discovery::CommandFrontmatter {
                        name: definition.name,
                        description: definition.description,
                        aliases: definition.aliases,
                    },
                    definition.instructions,
                );
                (
                    name,
                    PackageEntry {
                        command,
                        origin: PackageOrigin::Declared,
                    },
                )
            })
            .collect();
        for (name, entry) in Self::plugin_contributed_packages() {
            packages.insert(name, entry);
        }
        for command in package_report.commands {
            let name = command.frontmatter.name.clone();
            packages.insert(
                name,
                PackageEntry {
                    command,
                    origin: PackageOrigin::Filesystem,
                },
            );
        }
        for command in command_report.commands {
            packages.insert(
                command.frontmatter.name.clone(),
                PackageEntry {
                    command,
                    origin: PackageOrigin::Filesystem,
                },
            );
        }
        packages.retain(|name, _| !config.disabled_name(name));
        let mut diagnostics = package_report.diagnostics;
        diagnostics.extend(command_report.diagnostics);
        PackageCatalog {
            packages,
            diagnostics,
        }
    }

    /// Project declarative package contributions from loaded plugin manifests.
    /// Plugin manifests are already authenticated and validated by the host.
    fn plugin_contributed_packages() -> BTreeMap<String, PackageEntry> {
        let Some(host) = agena_runtime_plugins::plugin_slot::current_plugin_host() else {
            return BTreeMap::new();
        };
        let mut contributions = host
            .plugins()
            .iter()
            .filter(|plugin| plugin.key().to_string() != COMMANDS_PLUGIN_ID)
            .map(|plugin| {
                (
                    plugin.key().to_string(),
                    plugin.manifest.version.clone(),
                    plugin.manifest.skills.clone(),
                )
            })
            .collect::<Vec<_>>();
        contributions.sort_by(|left, right| left.0.cmp(&right.0));
        Self::plugin_contributed_packages_from_manifests(contributions)
    }

    fn plugin_contributed_packages_from_manifests(
        manifests: impl IntoIterator<Item = (String, String, Vec<PluginSkillDefinition>)>,
    ) -> BTreeMap<String, PackageEntry> {
        let mut packages = BTreeMap::new();
        for (plugin_id, plugin_version, commands) in manifests {
            for definition in commands {
                let name = definition.name.clone();
                let command = DiscoveredCommand::bundled(
                    discovery::CommandFrontmatter {
                        name: definition.name,
                        description: definition.description,
                        aliases: definition.aliases,
                    },
                    definition.instructions,
                );
                packages.insert(
                    name,
                    PackageEntry {
                        command,
                        origin: PackageOrigin::Plugin {
                            plugin_id: plugin_id.clone(),
                            plugin_version: plugin_version.clone(),
                        },
                    },
                );
            }
        }
        packages
    }

    fn catalog_fingerprint(catalog: &PackageCatalog) -> String {
        let mut digest = Sha256::new();
        for (name, entry) in &catalog.packages {
            digest.update(name.as_bytes());
            digest.update([0]);
            digest.update(entry.origin.source_label().as_bytes());
            digest.update([0]);
            digest.update(entry.command.content_hash().as_bytes());
            digest.update([0]);
            if let Some(path) = entry.command.source_path.as_ref() {
                digest.update(path.to_string_lossy().as_bytes());
            }
            digest.update([0xff]);
        }
        for diagnostic in &catalog.diagnostics {
            digest.update(diagnostic.path.to_string_lossy().as_bytes());
            digest.update([0]);
            digest.update(diagnostic.diagnostic.as_bytes());
            digest.update([0xfe]);
        }
        hex::encode(digest.finalize())
    }

    /// Rescan on demand instead of keeping stale prompt packages in memory.
    /// This gives every command tool a request-driven hot-reload boundary;
    /// `refresh` surfaces the generation/delta when a caller needs an explicit
    /// audit point.
    fn refresh_catalog(&self) -> SdkResult<CatalogRefresh> {
        let catalog = Self::discovered_catalog_for_workspace_with_config(
            self.workspace_root()?,
            &self.config(),
        );
        let fingerprint = Self::catalog_fingerprint(&catalog);
        let mut state = self
            .catalog_state
            .lock()
            .map_err(|_| PluginError::internal("commands catalog-state lock poisoned"))?;
        let changed = state.fingerprint.as_deref() != Some(fingerprint.as_str());
        if changed {
            state.generation = state.generation.saturating_add(1);
            state.fingerprint = Some(fingerprint.clone());
        }
        Ok(CatalogRefresh {
            catalog,
            fingerprint,
            generation: state.generation,
            changed,
        })
    }

    fn paginate<T>(
        items: Vec<T>,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> (Vec<T>, usize, usize) {
        let total = items.len();
        let offset = offset.unwrap_or(0) as usize;
        if offset >= total {
            return (Vec::new(), total, offset);
        }
        let limit = limit
            .map(|value| value as usize)
            .unwrap_or(total.saturating_sub(offset));
        let end = offset.saturating_add(limit).min(total);
        let page = items
            .into_iter()
            .skip(offset)
            .take(end.saturating_sub(offset))
            .collect::<Vec<_>>();
        (page, total, offset)
    }

    fn package_description(entry: &PackageEntry) -> String {
        if entry.command.frontmatter.description.trim().is_empty() {
            format!(
                "Read the '{}' package instructions.",
                entry.command.frontmatter.name
            )
        } else {
            entry.command.frontmatter.description.clone()
        }
    }

    fn resolve_package<'a>(
        packages: &'a BTreeMap<String, PackageEntry>,
        requested: &str,
    ) -> SdkResult<(&'a str, &'a PackageEntry)> {
        if let Some((name, entry)) = packages.get_key_value(requested) {
            return Ok((name.as_str(), entry));
        }
        let normalized = requested.trim().to_ascii_lowercase();
        packages
            .iter()
            .find(|(_, entry)| entry.command.matches(normalized.as_str()))
            .map(|(name, entry)| (name.as_str(), entry))
            .ok_or_else(|| {
                PluginError::invalid_params(format!("unknown command package '{requested}'"))
            })
    }

    // ── the dynamic command registry ───────────────────────────────────────

    /// Bring the host's dynamic command registry in line with the current
    /// catalog: remove what disappeared, register what is new or changed.
    ///
    /// The registry has no bulk-replace, and a registration at an occupied key
    /// is refused rather than overwritten, so a changed package is removed and
    /// re-registered. An unchanged rescan does neither, and the first rescan
    /// after init is the one that publishes everything.
    ///
    /// Entries register at the global layer, because that is the layer
    /// `resolve_command` and the command catalog read: a package that existed
    /// only inside one session's scope would be invisible to the very request
    /// that discovered it. The cost of that choice is that a discovered name
    /// matching a declared command — a user file called `review.md` next to the
    /// built-in `/review` — is refused by the host. That is the intended
    /// outcome: a built-in is not a package anyone can shadow from disk, and
    /// saying so out loud beats silently offering a command that resolves to
    /// something else. Each refusal is counted and logged here.
    ///
    /// Registration is clamped: at most [`MAX_SYNCS_PER_PASS`] registry writes
    /// run per pass, so a directory with thousands of files cannot turn one
    /// catalog refresh into an unbounded burst of host callbacks. The remainder
    /// registers on the next pass.
    async fn sync_commands(
        &self,
        host: &Arc<dyn HostClient>,
        catalog: &PackageCatalog,
    ) -> SdkResult<ResyncOutcome> {
        let desired: BTreeMap<String, String> = catalog
            .packages
            .iter()
            .map(|(name, entry)| (name.clone(), entry.command.content_hash()))
            .collect();

        let (stale, changed) = {
            let synced = self
                .registered
                .lock()
                .map_err(|_| PluginError::internal("commands registry-state lock poisoned"))?;
            let stale = synced
                .by_command_id
                .keys()
                .filter(|command_id| !desired.contains_key(*command_id))
                .cloned()
                .collect::<Vec<_>>();
            let changed = desired
                .iter()
                .filter(|(command_id, hash)| synced.by_command_id.get(*command_id) != Some(*hash))
                .map(|(command_id, _)| command_id.clone())
                .take(MAX_SYNCS_PER_PASS)
                .collect::<Vec<_>>();
            (stale, changed)
        };

        let mut removed = 0usize;
        let mut registered = 0usize;
        let mut collisions = 0usize;
        for command_id in &stale {
            host.remove_command(HostCommandRemoveRequest {
                id: command_id.clone(),
            })
            .await?;
            removed = removed.saturating_add(1);
            self.registered
                .lock()
                .map_err(|_| PluginError::internal("commands registry-state lock poisoned"))?
                .by_command_id
                .remove(command_id);
        }
        for command_id in &changed {
            let Some(entry) = catalog.packages.get(command_id) else {
                continue;
            };
            // A changed entry may already be registered under an older
            // definition; a duplicate is refused rather than replaced, so the
            // old registration goes first.
            if self
                .registered
                .lock()
                .map_err(|_| PluginError::internal("commands registry-state lock poisoned"))?
                .by_command_id
                .contains_key(command_id)
            {
                host.remove_command(HostCommandRemoveRequest {
                    id: command_id.clone(),
                })
                .await?;
                removed = removed.saturating_add(1);
                self.registered
                    .lock()
                    .map_err(|_| PluginError::internal("commands registry-state lock poisoned"))?
                    .by_command_id
                    .remove(command_id);
            }
            match host
                .register_command(HostCommandRegisterRequest {
                    command: package_command_definition(command_id, entry),
                })
                .await
            {
                Ok(_) => {
                    registered = registered.saturating_add(1);
                    self.registered
                        .lock()
                        .map_err(|_| {
                            PluginError::internal("commands registry-state lock poisoned")
                        })?
                        .by_command_id
                        .insert(command_id.clone(), entry.command.content_hash());
                }
                Err(error) => {
                    collisions = collisions.saturating_add(1);
                    tracing::warn!(
                        target: "agena_commands::registry",
                        command = %command_id,
                        diagnostic = %error.diagnostic_message(),
                        "discovered command could not be registered; keeping the existing owner"
                    );
                }
            }
        }

        Ok(ResyncOutcome {
            registered,
            removed,
            collisions,
            diagnostics: catalog.diagnostics.len(),
        })
    }

    /// Synchronize the registry from inside a hook callback.
    ///
    /// The host client for a callback is not cached anywhere: the client `init`
    /// receives belongs to the effect scope that existed when the plugin was
    /// prepared, and a reload replaces that scope. Resolving the live host and
    /// its scope on every pass keeps a long-lived plugin from writing through a
    /// retired one. Static plugins are never reloaded onto a *different*
    /// instance — a rebuild always calls `init` again — so `current_plugin_host`
    /// and the loaded plugin's scope are the live pair whenever a callback runs.
    async fn sync_from_callback(&self, catalog: &PackageCatalog) -> ResyncOutcome {
        let Some(host) = agena_runtime_plugins::plugin_slot::current_plugin_host() else {
            return ResyncOutcome {
                diagnostics: catalog.diagnostics.len(),
                ..ResyncOutcome::default()
            };
        };
        let Ok(plugin_key) = PluginKey::new("agena", "commands") else {
            return ResyncOutcome {
                diagnostics: catalog.diagnostics.len(),
                ..ResyncOutcome::default()
            };
        };
        let Some(loaded) = host
            .plugins()
            .iter()
            .find(|plugin| plugin.key() == plugin_key)
            .cloned()
        else {
            return ResyncOutcome {
                diagnostics: catalog.diagnostics.len(),
                ..ResyncOutcome::default()
            };
        };
        let client = host
            .host_handle()
            .scoped_host_client_for_scope(COMMANDS_PLUGIN_ID.to_string(), loaded.effect_scope());
        match self.sync_commands(&client, catalog).await {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(
                    target: "agena_commands::registry",
                    diagnostic = %error.diagnostic_message(),
                    "command package registry synchronization failed"
                );
                ResyncOutcome {
                    diagnostics: catalog.diagnostics.len(),
                    ..ResyncOutcome::default()
                }
            }
        }
    }

    // ── filesystem watcher ─────────────────────────────────────────────────

    /// Install a platform watcher over existing catalog roots. For a root that
    /// does not yet exist, watch its nearest existing ancestor without
    /// recursion so creation of `.agena/skills` is still observed without
    /// recursively monitoring an entire workspace. The normal next request
    /// rescans and can then use the newly created root.
    fn start_filesystem_watcher(&self) -> SdkResult<()> {
        if !self.config().watcher.enabled {
            return Ok(());
        }
        let (package_roots, command_roots) =
            Self::filesystem_roots(self.workspace_root()?, &self.config());
        let mut desired = BTreeMap::<PathBuf, RecursiveMode>::new();
        for root in package_roots.into_iter().chain(command_roots) {
            let (path, mode) = watcher_target(root.as_path());
            desired
                .entry(path)
                .and_modify(|current| {
                    if matches!(mode, RecursiveMode::Recursive) {
                        *current = RecursiveMode::Recursive;
                    }
                })
                .or_insert(mode);
        }

        let generation = Arc::clone(&self.watcher_generation);
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.is_ok() {
                    generation.fetch_add(1, Ordering::AcqRel);
                }
            })
            .map_err(|error| {
                PluginError::internal(format!("cannot start command filesystem watcher: {error}"))
            })?;
        let mut watched_paths = Vec::new();
        for (path, mode) in desired {
            match watcher.watch(path.as_path(), mode) {
                Ok(()) => watched_paths.push(path),
                Err(error) => tracing::warn!(
                    target: "agena_commands::watcher",
                    path = %path.display(),
                    "cannot watch command catalog path: {error}"
                ),
            }
        }
        *self
            .watcher
            .lock()
            .map_err(|_| PluginError::internal("commands watcher lock poisoned"))? =
            Some(CommandCatalogWatcher {
                _watcher: watcher,
                watched_paths,
            });
        Ok(())
    }

    fn watcher_status(&self) -> SdkResult<(bool, usize, u64)> {
        let watcher = self
            .watcher
            .lock()
            .map_err(|_| PluginError::internal("commands watcher lock poisoned"))?;
        Ok((
            self.config().watcher.enabled,
            watcher
                .as_ref()
                .map(|watcher| watcher.watched_paths.len())
                .unwrap_or_default(),
            self.watcher_generation.load(Ordering::Acquire),
        ))
    }
}

fn watcher_target(root: &Path) -> (PathBuf, RecursiveMode) {
    if root.is_dir() {
        return (root.to_path_buf(), RecursiveMode::Recursive);
    }
    let mut ancestor = root.to_path_buf();
    while !ancestor.is_dir() {
        if !ancestor.pop() {
            return (PathBuf::from("."), RecursiveMode::NonRecursive);
        }
    }
    (ancestor, RecursiveMode::NonRecursive)
}

/// Turn one discovered package into the command a composer offers.
fn package_command_definition(name: &str, entry: &PackageEntry) -> CommandDefinition {
    CommandDefinition {
        id: name.to_string(),
        title: name.to_string(),
        group: DISCOVERED_GROUP.to_string(),
        category: Some(DISCOVERED_CATEGORY.to_string()),
        slash: Some(format!("/{name}")),
        aliases: entry.command.frontmatter.aliases.clone(),
        docs: CommandDocs {
            summary: Some(CommandsPlugin::package_description(entry)),
            usage: None,
            ..CommandDocs::default()
        },
        input: package_contract(),
        target: CommandTarget::Method {
            handler: HANDLER_RUN.to_string(),
        },
    }
}

// ── managed document I/O ───────────────────────────────────────────────────

#[derive(Debug)]
struct PackageWriteResult {
    name: String,
    path: PathBuf,
    operation: &'static str,
    revision: Option<String>,
    warning: Option<String>,
}

fn validate_managed_package_name(name: &str) -> SdkResult<()> {
    let name = name.trim();
    let valid = !name.is_empty()
        && name.len() <= 96
        && name.bytes().all(|byte| {
            matches!(
                byte,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.'
            )
        })
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(PluginError::invalid_params(
            "Package names must start with an ASCII letter or digit and contain only letters, digits, '.', '_' or '-' (maximum 96 characters)",
        ))
    }
}

fn enforce_package_document_size(document: &str) -> SdkResult<()> {
    if document.len() <= MAX_COMMAND_DOCUMENT_BYTES {
        Ok(())
    } else {
        Err(PluginError::invalid_params(format!(
            "Package document exceeds the {} byte limit",
            MAX_COMMAND_DOCUMENT_BYTES
        )))
    }
}

fn parse_managed_package_document(document: &str) -> SdkResult<DiscoveredCommand> {
    enforce_package_document_size(document)?;
    let package = DiscoveredCommand::from_raw(document).map_err(|error| {
        PluginError::invalid_params(format!("invalid package document: {error}"))
    })?;
    validate_managed_package_name(package.frontmatter.name.as_str())?;
    let mut aliases = std::collections::BTreeSet::new();
    for alias in &package.frontmatter.aliases {
        let alias = alias.trim();
        validate_managed_package_name(alias)?;
        if !aliases.insert(alias.to_ascii_lowercase()) {
            return Err(PluginError::invalid_params(format!(
                "Package aliases contain a duplicate value: '{alias}'"
            )));
        }
    }
    Ok(package)
}

fn package_document_revision(document: &str) -> String {
    hex::encode(Sha256::digest(document.as_bytes()))
}

fn check_package_revision(document: &str, expected: Option<&str>) -> SdkResult<()> {
    if let Some(expected) = expected
        && expected != package_document_revision(document)
    {
        return Err(PluginError::invalid_params(
            "Package revision is stale; read the document again before writing",
        ));
    }
    Ok(())
}

fn write_package_document(path: &Path, document: &str) -> SdkResult<()> {
    enforce_package_document_size(document)?;
    agena_runtime_tools::atomic_replace_file(path, document.as_bytes()).map_err(package_write_error)
}

fn create_package_document(path: &Path, document: &str) -> SdkResult<()> {
    enforce_package_document_size(document)?;
    agena_runtime_tools::atomic_create_file(path, document.as_bytes(), None)
        .map_err(package_write_error)
}

fn package_write_error(error: std::io::Error) -> PluginError {
    PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
        "workspace command package filesystem operation failed",
        &error,
    ))
}

fn workspace_managed_package_path(workspace_root: &Path, name: &str) -> Option<PathBuf> {
    if let Err(error) = validate_managed_package_name(name) {
        tracing::warn!(
            package_name = name,
            diagnostic = %error.diagnostic_message(),
            "discovered package has an invalid managed name"
        );
        return None;
    }
    let workspace = match workspace_root.canonicalize() {
        Ok(workspace) => workspace,
        Err(error) => {
            tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "canonicalize workspace while classifying a managed package",
                    &error,
                ),
                "managed package classification could not resolve the workspace"
            );
            return None;
        }
    };
    let root = match workspace.join(".agena/skills").canonicalize() {
        Ok(root) => root,
        Err(error) => {
            tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "canonicalize the managed package root while classifying a package",
                    &error,
                ),
                "managed package classification could not resolve its root"
            );
            return None;
        }
    };
    root.starts_with(&workspace)
        .then(|| root.join(name).join("SKILL.md"))
}

fn is_workspace_managed_package(workspace_root: &Path, name: &str, entry: &PackageEntry) -> bool {
    let Some(expected) = workspace_managed_package_path(workspace_root, name) else {
        return false;
    };
    let Some(source) = entry.command.source_path.as_ref() else {
        return false;
    };
    let (Ok(expected), Ok(source)) = (expected.canonicalize(), source.canonicalize()) else {
        return false;
    };
    expected == source
}

fn package_write_output(
    result: PackageWriteResult,
    generation: Option<u64>,
    catalog_changed: bool,
) -> ToolInvokeOutput {
    let path = result.path.display().to_string();
    let generation_label = generation
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown (refresh failed)".into());
    let operation = result.operation;
    let outcome_note = format!(
        "Revision: {:?}{}",
        result.revision,
        result
            .warning
            .as_ref()
            .map(|warning| format!("\nWarning: {warning}"))
            .unwrap_or_default()
    );
    ToolInvokeOutput::from_parts(
        format!("Command package {operation}: {}", result.name),
        format!("{operation} · catalog generation {generation_label}"),
        format!(
            "Command package '{}' was {operation} at {path}. Catalog generation {generation_label}.\n{outcome_note}",
            result.name
        ),
        Some(serde_json::json!({
            "name": result.name,
            "path": path,
            "operation": operation,
            "committed": true,
            "revision": result.revision,
            "catalog_warning": result.warning,
            "catalog_generation": generation,
            "catalog_changed": catalog_changed,
            "editable": operation != "removed",
        })),
        BTreeMap::from([
            ("agena.effect".to_string(), "command_catalog".to_string()),
            ("operation".to_string(), operation.to_string()),
            ("catalog_generation".to_string(), generation_label),
        ]),
        Vec::new(),
    )
}

// ── the trait impl ─────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl Plugin for CommandsPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = self.manifest_for();
        manifest.commands = built_in_commands();
        manifest
    }

    async fn init(&self, ctx: InitContext, _host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        // The host client handed to `init` belongs to this plugin's first
        // effect scope, which a later rebind replaces. Nothing here keeps it:
        // the live client for a callback comes from
        // `current_plugin_host()` inside the callback itself.
        let config: CommandsPluginConfig = macro_support::parse_defaulted_settings(
            ctx.settings,
            "invalid commands plugin config",
        )?;
        config.validate_for_workspace(ctx.workspace_root.as_path())?;
        self.workspace_root
            .set(ctx.workspace_root)
            .map_err(|_| PluginError::internal("commands plugin initialized more than once"))?;
        self.config.set(config).map_err(|_| {
            PluginError::internal("commands plugin config initialized more than once")
        })?;
        self.start_filesystem_watcher()?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_definition(
        &self,
        input: ToolDefinitionInput,
    ) -> SdkResult<Option<ToolDefinitionPatch>> {
        if input.plugin_key().to_string() != COMMANDS_PLUGIN_ID {
            return Ok(None);
        }
        // Rescan and republish the dynamic command registry before describing
        // the tools. This is the synchronization point because it is a real
        // request boundary with a valid host callback, and the scan stays cheap
        // through the definition catalog's short cache.
        let catalog = self.definition_catalog_async().await?;
        let sync = self.sync_from_callback(&catalog).await;

        let declared = catalog
            .packages
            .values()
            .filter(|entry| entry.origin == PackageOrigin::Declared)
            .count();
        let external = catalog.packages.len().saturating_sub(declared);
        let preview = catalog
            .packages
            .iter()
            .take(12)
            .map(|(name, entry)| {
                format!(
                    "- /{} [{}]: {}",
                    name,
                    entry.origin.source_label(),
                    Self::package_description(entry)
                )
            })
            .collect::<Vec<_>>();
        let mut lines = vec![
            format!(
                "Currently discovered command packages: {declared} declared, {external} external."
            ),
            "Every package is also a slash command; running it inserts its instructions into the composer.".to_string(),
            format!("Use `{COMMANDS_PLUGIN_ID}.list` to page through discovered packages."),
            format!("Use `{COMMANDS_PLUGIN_ID}.get` to read one package in full, then apply its plain-text body to the current task."),
            format!("Use `{COMMANDS_PLUGIN_ID}.read_resource` to read a bounded text resource from a package directory."),
            format!("Use `{COMMANDS_PLUGIN_ID}.refresh` to rescan filesystem-backed packages and inspect the catalog generation."),
            format!("Use `{COMMANDS_PLUGIN_ID}.install` and `{COMMANDS_PLUGIN_ID}.remove` only for workspace-managed `.agena/skills` documents."),
        ];
        if !catalog.diagnostics.is_empty() {
            lines.push(format!(
                "Discovery diagnostics: {} invalid or unreadable package(s); call list with verbose=true for details.",
                catalog.diagnostics.len()
            ));
        }
        if sync.removed > 0 || sync.collisions > 0 {
            lines.push(format!("Command registry: {}.", sync.summary()));
        }
        if !preview.is_empty() {
            lines.push("Preview:".to_string());
            lines.extend(preview);
        }

        let summary = match input.tool_name() {
            "list" => Some(format!(
                "List discovered command packages. Currently {declared} declared and {external} external."
            )),
            "get" => Some("Read one discovered command package.".to_string()),
            "read_resource" => Some(
                "Read a bounded UTF-8 resource contained by one package directory.".to_string(),
            ),
            "refresh" => Some(
                "Rescan filesystem-backed command packages and report whether the catalog changed."
                    .to_string(),
            ),
            "install" => Some(
                "Install a workspace-managed `.agena/skills/<name>/SKILL.md` document.".to_string(),
            ),
            "remove" => {
                Some("Remove one workspace-managed `.agena/skills` package document.".to_string())
            }
            _ => None,
        };

        Ok(Some(ToolDefinitionPatch {
            summary,
            help: Some(lines.join("\n")),
            input_schema: None,
        }))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        let tool = input.tool_name.clone();
        let value = input.input.clone();
        match tool.as_str() {
            "list" => {
                let input: CommandsListInput = parse_tool_input(&tool, value)?;
                self.invoke_list(input).await
            }
            "get" => {
                let input: CommandsGetInput = parse_tool_input(&tool, value)?;
                self.invoke_get(input).await
            }
            "install" => {
                let input: CommandsInstallInput = parse_tool_input(&tool, value)?;
                self.invoke_install(input).await
            }
            "remove" => {
                let input: CommandsRemoveInput = parse_tool_input(&tool, value)?;
                self.invoke_remove(input).await
            }
            "read_resource" => {
                let input: CommandsReadResourceInput = parse_tool_input(&tool, value)?;
                self.invoke_read_resource(input).await
            }
            "refresh" => {
                let input: CommandsRefreshInput = parse_tool_input(&tool, value)?;
                self.invoke_refresh(input).await
            }
            other => Err(PluginError::not_implemented(format!(
                "commands plugin tool `{other}`"
            ))),
        }
    }

    async fn command_invoke(&self, input: CommandInvokeInput) -> SdkResult<CommandResult> {
        let handler = input.command_id.clone();
        // Every registered package is a `Method` target under one handler, so
        // the command id — which is the package's canonical name — is what
        // selects the instructions.
        if handler != HANDLER_RUN {
            return Err(PluginError::not_implemented(format!(
                "commands plugin handler `{handler}`"
            )));
        }
        let requested = input
            .slash
            .as_deref()
            .map(|slash| slash.trim_start_matches('/'))
            .filter(|name| !name.is_empty())
            .unwrap_or(handler.as_str())
            .to_string();
        let args = command_args(&input.input);
        let (name, body) = {
            let catalog = self.definition_catalog_async().await?;
            let (name, entry) = Self::resolve_package(&catalog.packages, requested.as_str())?;
            (name.to_string(), entry.command.body.trim().to_string())
        };
        let prompt = if args.is_empty() {
            body
        } else {
            format!("{body}\n\n{args}")
        };
        Ok(
            CommandResult::succeeded(format!("Inserted `/{name}` instructions."))
                .with_title(format!("/{name}"))
                .with_effect(CommandHostEffect::InsertPrompt { prompt }),
        )
    }
}

impl CommandsPlugin {
    async fn invoke_list(&self, input: CommandsListInput) -> SdkResult<ToolInvokeOutput> {
        let catalog = self.definition_catalog_async().await?;
        let workspace_root = self.workspace_root()?.to_path_buf();
        let entries = catalog.packages.into_iter().collect::<Vec<_>>();
        let (entries, total, offset) = Self::paginate(entries, input.offset, input.limit);
        let mut lines = vec![format!(
            "Discovered command package(s): returned {}/{} starting at offset {}.",
            entries.len(),
            total,
            offset
        )];
        for (name, entry) in &entries {
            if input.verbose {
                lines.push(format!(
                    "- /{}: {} ({})",
                    name,
                    Self::package_description(entry),
                    entry.origin.source_label()
                ));
            } else {
                lines.push(format!("- /{name}"));
            }
        }
        if input.verbose && !catalog.diagnostics.is_empty() {
            lines.push("Discovery diagnostics:".to_string());
            lines.extend(
                catalog
                    .diagnostics
                    .iter()
                    .map(|diagnostic| format!("- {}", diagnostic.failure.user.fallback)),
            );
        }
        let payload = serde_json::json!({
            "packages": entries.iter().map(|(name, entry)| serde_json::json!({
                "name": name,
                "summary": Self::package_description(entry),
                "aliases": entry.command.frontmatter.aliases,
                "source_path": entry.command.source_path,
                "source": entry.origin.source_label(),
                "content_hash": entry.command.content_hash(),
                "editable": is_workspace_managed_package(workspace_root.as_path(), name, entry),
            })).collect::<Vec<_>>(),
            "diagnostics": catalog.diagnostics.iter().map(|diagnostic| serde_json::json!({
                "problem": diagnostic.failure,
            })).collect::<Vec<_>>(),
            "total": total,
            "offset": offset,
            "returned": entries.len(),
        });
        Ok(ToolInvokeOutput::from_parts(
            "commands list",
            if offset.saturating_add(entries.len()) < total {
                format!("{} of {total} packages · more available", entries.len())
            } else {
                format!("{} of {total} packages", entries.len())
            },
            lines.join("\n"),
            Some(payload),
            BTreeMap::from([
                ("total_packages".to_string(), total.to_string()),
                ("returned_packages".to_string(), entries.len().to_string()),
                ("offset".to_string(), offset.to_string()),
            ]),
            Vec::new(),
        ))
    }

    async fn invoke_get(&self, input: CommandsGetInput) -> SdkResult<ToolInvokeOutput> {
        let requested_name = input.name;
        self.run_blocking(move |plugin| {
            let catalog = plugin.discovered_catalog()?;
            let workspace_root = plugin.workspace_root()?;
            let (name, entry) = Self::resolve_package(&catalog.packages, requested_name.as_str())?;
            let summary = Self::package_description(entry);
            let body = entry.command.body.trim();
            let document = match entry.command.source_path.as_ref() {
                Some(path) => discovery::read_command_document(path)
                    .map_err(|error| PluginError::invalid_params_error(&error))?,
                None => format_package_document(entry),
            };
            enforce_package_document_size(document.as_str())?;
            let revision = package_document_revision(&document);
            let text =
                format!("Name: {name}\nRevision: {revision}\nSummary: {summary}\n\nBody:\n{body}");
            let payload = serde_json::json!({
                "name": name,
                "summary": summary,
                "body": body,
                "aliases": entry.command.frontmatter.aliases,
                "source_path": entry.command.source_path,
                "source": entry.origin.source_label(),
                "content_hash": entry.command.content_hash(),
                "document": document,
                "revision": revision,
                "editable": is_workspace_managed_package(workspace_root, name, entry),
            });
            Ok(ToolInvokeOutput::from_parts(
                format!("commands get {name}"),
                format!("{} · {summary}", entry.origin.source_label()),
                text,
                Some(payload),
                BTreeMap::from([("name".to_string(), name.to_string())]),
                Vec::new(),
            ))
        })
        .await
    }

    async fn invoke_install(&self, input: CommandsInstallInput) -> SdkResult<ToolInvokeOutput> {
        let document = input.document;
        self.run_blocking(move |plugin| {
            let result = plugin.install_managed_package(document.as_str())?;
            Ok(plugin.complete_package_write(result))
        })
        .await
    }

    async fn invoke_remove(&self, input: CommandsRemoveInput) -> SdkResult<ToolInvokeOutput> {
        let name = input.name;
        self.run_blocking(move |plugin| {
            let result = plugin.remove_managed_package(name.as_str())?;
            Ok(plugin.complete_package_write(result))
        })
        .await
    }

    async fn invoke_read_resource(
        &self,
        input: CommandsReadResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let requested_name = input.name;
        let resource_path = input.path;
        let max_bytes = input.max_bytes;
        self.run_blocking(move |plugin| {
            let catalog = plugin.discovered_catalog()?;
            let (name, entry) = Self::resolve_package(&catalog.packages, requested_name.as_str())?;
            if !entry.origin.supports_resources() {
                return Err(PluginError::invalid_params(format!(
                    "{} '{}' does not expose filesystem-backed resources",
                    entry.origin.source_label(),
                    name
                )));
            }
            let content = entry
                .command
                .read_text_resource(Path::new(resource_path.as_str()), max_bytes as usize)
                .map_err(|error| PluginError::invalid_params_error(&error))?;
            Ok(ToolInvokeOutput::from_parts(
                format!("command resource {name}/{resource_path}"),
                format!("{} bytes", content.len()),
                content.clone(),
                Some(serde_json::json!({
                    "name": name,
                    "path": resource_path,
                    "content": content,
                    "bytes": content.len(),
                    "content_hash": hex::encode(Sha256::digest(content.as_bytes())),
                    "package_content_hash": entry.command.content_hash(),
                    "source_path": entry.command.source_path,
                    "source": entry.origin.source_label(),
                })),
                BTreeMap::from([
                    ("package".to_string(), name.to_string()),
                    ("resource_path".to_string(), resource_path),
                ]),
                Vec::new(),
            ))
        })
        .await
    }

    async fn invoke_refresh(&self, input: CommandsRefreshInput) -> SdkResult<ToolInvokeOutput> {
        let (refresh, watcher_status) = self
            .run_blocking(|plugin| {
                let refresh = plugin.refresh_catalog()?;
                let watcher_status = plugin.watcher_status()?;
                Ok((refresh, watcher_status))
            })
            .await?;
        let (watcher_enabled, watched_path_count, watcher_generation) = watcher_status;
        let declared = refresh
            .catalog
            .packages
            .values()
            .filter(|entry| entry.origin == PackageOrigin::Declared)
            .count();
        let external = refresh.catalog.packages.len().saturating_sub(declared);
        let mut lines = vec![format!(
            "Command catalog generation {} {} ({} declared, {} external).",
            refresh.generation,
            if refresh.changed {
                "changed"
            } else {
                "is unchanged"
            },
            declared,
            external,
        )];
        if input.verbose && !refresh.catalog.diagnostics.is_empty() {
            lines.push("Discovery diagnostics:".to_string());
            lines.extend(
                refresh
                    .catalog
                    .diagnostics
                    .iter()
                    .map(|diagnostic| format!("- {}", diagnostic.failure.user.fallback)),
            );
        }
        Ok(ToolInvokeOutput::from_parts(
            "commands refresh",
            format!(
                "{} · {declared} declared · {external} external",
                if refresh.changed {
                    "Changed"
                } else {
                    "Unchanged"
                }
            ),
            lines.join("\n"),
            Some(serde_json::json!({
                "changed": refresh.changed,
                "generation": refresh.generation,
                "fingerprint": refresh.fingerprint,
                "declared": declared,
                "external": external,
                "watcher": {
                    "enabled": watcher_enabled,
                    "watched_path_count": watched_path_count,
                    "generation": watcher_generation,
                },
                "diagnostics": refresh.catalog.diagnostics.iter().map(|diagnostic| serde_json::json!({
                    "problem": diagnostic.failure,
                })).collect::<Vec<_>>(),
            })),
            BTreeMap::from([
                (
                    "catalog_generation".to_string(),
                    refresh.generation.to_string(),
                ),
                ("catalog_changed".to_string(), refresh.changed.to_string()),
                ("catalog_fingerprint".to_string(), refresh.fingerprint),
                ("watcher_enabled".to_string(), watcher_enabled.to_string()),
                (
                    "watcher_generation".to_string(),
                    watcher_generation.to_string(),
                ),
            ]),
            Vec::new(),
        ))
    }
}

fn format_package_document(entry: &PackageEntry) -> String {
    let frontmatter = entry.command.frontmatter.clone();
    let mut document = String::from("---\n");
    document.push_str(&format!("name: {}\n", frontmatter.name));
    if !frontmatter.description.trim().is_empty() {
        document.push_str(&format!("description: {}\n", frontmatter.description));
    }
    if !frontmatter.aliases.is_empty() {
        document.push_str(&format!("aliases: [{}]\n", frontmatter.aliases.join(", ")));
    }
    document.push_str("---\n");
    document.push_str(entry.command.body.trim());
    document.push('\n');
    document
}

fn parse_tool_input<T: serde::de::DeserializeOwned + JsonSchema>(
    tool: &str,
    value: Value,
) -> SdkResult<T> {
    macro_support::parse_typed_json_value_with_field_suggestions(
        value,
        &macro_support::json_schema_for::<T>(),
        &format!("{COMMANDS_PLUGIN_ID}.{tool} input field"),
    )
}

/// The one argument a package invocation can carry. The contract supplies
/// `{ "args": "…" }`; a caller that bypasses the contract and hands over a bare
/// string still works.
fn command_args(value: &Value) -> String {
    match value {
        Value::Object(object) => object
            .get("args")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
        Value::String(text) => text.trim().to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_plugin_host::sdk::{CommandTarget, NoopHostClient};

    async fn init_test_plugin(plugin: &CommandsPlugin, workspace_root: &Path) {
        init_test_plugin_with_config(plugin, workspace_root, Value::Null).await;
    }

    async fn init_test_plugin_with_config(
        plugin: &CommandsPlugin,
        workspace_root: &Path,
        config: Value,
    ) {
        plugin
            .init(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: workspace_root.to_path_buf(),
                    plugin_id: PluginKey::new("agena", "commands").expect("plugin key"),
                    host_callback_url: None,
                    host_callback_token: None,
                    settings: config,
                    protocol_version: 1,
                },
                Arc::new(NoopHostClient),
            )
            .await
            .expect("init plugin");
    }

    #[test]
    fn every_built_in_command_is_client_targeted_with_an_empty_input_contract() {
        let manifest = CommandsPlugin::new().manifest();
        let declared = manifest
            .commands
            .iter()
            .filter(|command| matches!(&command.target, CommandTarget::Client { .. }))
            .count();
        assert_eq!(declared, BUILT_IN_COMMANDS.len());
        for command in &manifest.commands {
            let CommandTarget::Client { .. } = &command.target else {
                continue;
            };
            assert!(
                command.input.accepts_only_empty_object(),
                "{} must not carry an input contract the server cannot honor",
                command.id
            );
            assert!(
                command.docs.summary.is_none(),
                "{} documents itself through a message key",
                command.id
            );
            assert!(command.docs.summary_key.is_some());
            assert!(
                command
                    .slash
                    .as_deref()
                    .is_some_and(|slash| slash.starts_with('/')),
                "{} needs a slash spelling",
                command.id
            );
            // The host refuses a manifest whose commands do not validate, so
            // the declaration must pass the same gate the loader applies.
            command.validate().expect("declared command must validate");
        }
    }

    #[test]
    fn declared_slash_and_aliases_are_unique() {
        let mut spellings = std::collections::BTreeSet::new();
        let mut actions = std::collections::BTreeSet::new();
        for built_in in BUILT_IN_COMMANDS {
            assert!(actions.insert(built_in.action), "duplicate action");
            assert!(
                spellings.insert(built_in.slash),
                "duplicate slash {}",
                built_in.slash
            );
            for alias in built_in.aliases {
                assert!(
                    spellings.insert(alias),
                    "duplicate alias {alias} on {}",
                    built_in.action
                );
            }
        }
    }

    #[test]
    fn a_client_action_and_its_slash_name_the_same_command() {
        let mut spellings = std::collections::BTreeSet::new();
        for built_in in BUILT_IN_COMMANDS {
            assert!(
                !built_in.action.is_empty()
                    && built_in
                        .action
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || "._-".contains(ch)),
                "{} is not a stable identifier",
                built_in.action
            );
            let name = built_in.slash.strip_prefix('/').unwrap_or_default();
            assert!(
                !name.is_empty() && !name.contains('/'),
                "{} must be `/` plus one name",
                built_in.slash
            );
            assert!(
                spellings.insert(built_in.slash),
                "duplicate slash {}",
                built_in.slash
            );
            for alias in built_in.aliases {
                assert!(
                    spellings.insert(alias),
                    "duplicate alias {alias} on {}",
                    built_in.action
                );
            }
        }
    }

    #[test]
    fn every_declared_action_is_in_the_client_vocabulary() {
        // The declaration and the client vocabulary are the same action set
        // seen from two sides: the plugin publishes what a client should show,
        // the enum says what a client can run. A drift between them means a
        // built-in command silently disappears from every palette.
        use agena_api::client_command::ClientCommandAction;

        for built_in in BUILT_IN_COMMANDS {
            let action = ClientCommandAction::from_action(built_in.action).unwrap_or_else(|| {
                panic!(
                    "{} is declared but no client implements it",
                    built_in.action
                )
            });
            assert_eq!(
                action.slash(),
                built_in.slash,
                "{} is published and implemented under different spellings",
                built_in.action
            );
        }
        assert_eq!(ClientCommandAction::ALL.len(), BUILT_IN_COMMANDS.len());
    }

    #[test]
    fn usage_is_declared_only_for_commands_that_take_arguments() {
        let manifest = CommandsPlugin::new().manifest();
        let usage_of = |id: &str| {
            manifest
                .commands
                .iter()
                .find(|command| command.id == id)
                .and_then(|command| command.docs.usage.as_deref())
        };
        assert_eq!(usage_of("commit"), Some("<message>"));
        assert_eq!(
            usage_of("pr"),
            Some("<title> [--body <text>] [--base <branch>] [--head <branch>]")
        );
        assert_eq!(usage_of("download"), Some("<workspace-path>"));
        assert_eq!(usage_of("export"), Some("[path]"));
        assert_eq!(usage_of("settings"), None);
        assert_eq!(usage_of("side"), Some("[question]"));
        assert_eq!(usage_of("btw"), Some("[question]"));
    }

    #[test]
    fn the_declaration_is_stable_across_calls() {
        assert_eq!(built_in_commands(), built_in_commands());
    }

    #[test]
    fn the_manifest_declares_the_bundled_packages_and_their_tools() {
        let manifest = CommandsPlugin::new().manifest();
        let names = manifest
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "batch",
                "debug",
                "doctor",
                "imagegen",
                "init",
                "plugin_creator",
                "review",
                "run",
                "run_skill_generator",
                "security_review",
                "simplify",
                "skill_creator",
                "skill_installer",
                "verify",
            ]
        );
        assert!(
            manifest
                .skills
                .iter()
                .all(|skill| !skill.instructions.trim().is_empty()),
            "every declared package carries instructions"
        );
        let tool_names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            tool_names,
            [
                "list",
                "get",
                "install",
                "remove",
                "read_resource",
                "refresh"
            ]
        );
        assert!(manifest.hooks.contains(HookSubscription::TOOL_DEFINITION));
        assert!(manifest.hooks.contains(HookSubscription::TOOL_INVOKE));
        assert!(!manifest.hooks.contains(HookSubscription::TOOL_BEFORE));
    }

    #[test]
    fn bundled_aliases_resolve_to_canonical_names() {
        let dir = tempfile::tempdir().expect("temp dir");
        let catalog = CommandsPlugin::discovered_catalog_for_workspace_with_config(
            dir.path(),
            &CommandsPluginConfig::default(),
        );
        let (name, _) = CommandsPlugin::resolve_package(&catalog.packages, "bootstrap")
            .expect("resolve declared alias");
        assert_eq!(name, "init");
        let (name, _) = CommandsPlugin::resolve_package(&catalog.packages, "check")
            .expect("resolve declared alias");
        assert_eq!(name, "verify");
    }

    #[test]
    fn manifest_packages_are_external_entries_without_resources() {
        let packages = CommandsPlugin::plugin_contributed_packages_from_manifests(vec![(
            "example.docs".to_string(),
            "1.2.3".to_string(),
            vec![PluginSkillDefinition {
                name: "plugin_docs".to_string(),
                description: "Use the plugin documentation workflow.".to_string(),
                instructions: "Read the plugin documentation first.".to_string(),
                aliases: vec!["docs-plugin".to_string()],
            }],
        )]);
        let entry = packages.get("plugin_docs").expect("plugin contribution");
        assert_eq!(entry.command.frontmatter.aliases, ["docs-plugin"]);
        assert_eq!(entry.command.body, "Read the plugin documentation first.");
        assert_eq!(entry.origin.source_label(), "plugin:example.docs@1.2.3");
        assert!(!entry.origin.supports_resources());
    }

    #[test]
    fn watcher_uses_recursive_roots_and_nonrecursive_existing_parents() {
        let workspace = tempfile::tempdir().expect("workspace");
        let existing = workspace.path().join("commands");
        std::fs::create_dir_all(&existing).expect("existing root");
        let (recursive, recursive_mode) = watcher_target(existing.as_path());
        assert_eq!(recursive, existing);
        assert!(matches!(recursive_mode, notify::RecursiveMode::Recursive));

        let missing = workspace.path().join(".agena/skills");
        let (ancestor, ancestor_mode) = watcher_target(missing.as_path());
        assert_eq!(ancestor, workspace.path());
        assert!(matches!(ancestor_mode, notify::RecursiveMode::NonRecursive));
    }

    #[tokio::test]
    async fn config_filters_disabled_names_and_accepts_extra_workspace_roots() {
        let workspace = tempfile::tempdir().expect("workspace");
        let disabled_dir = workspace.path().join(".agena/skills/disabled-package");
        let extra_dir = workspace.path().join("contributed/extra-package");
        std::fs::create_dir_all(&disabled_dir).expect("disabled package dir");
        std::fs::create_dir_all(&extra_dir).expect("extra package dir");
        std::fs::write(
            disabled_dir.join("SKILL.md"),
            "---\nname: disabled_package\ndescription: should be hidden\n---\nDisabled.",
        )
        .expect("disabled package");
        std::fs::write(
            extra_dir.join("SKILL.md"),
            "---\nname: extra_package\ndescription: explicit workspace root\n---\nExtra.",
        )
        .expect("extra package");

        let plugin = CommandsPlugin::new();
        init_test_plugin_with_config(
            &plugin,
            workspace.path(),
            serde_json::json!({
                "disabled": ["disabled_package"],
                "additional_roots": ["contributed"],
            }),
        )
        .await;

        let listed = plugin
            .invoke_list(CommandsListInput {
                offset: None,
                limit: None,
                verbose: true,
            })
            .await
            .expect("list configured catalog");
        let names = listed
            .payload
            .as_ref()
            .and_then(|payload| payload.get("packages"))
            .and_then(Value::as_array)
            .expect("packages payload")
            .iter()
            .filter_map(|entry| entry.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert!(names.contains(&"extra_package"));
        assert!(!names.contains(&"disabled_package"));
        assert!(
            plugin
                .invoke_get(CommandsGetInput {
                    name: "disabled_package".to_string(),
                })
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn refresh_reports_deterministic_catalog_generations() {
        let workspace = tempfile::tempdir().expect("workspace");
        let plugin = CommandsPlugin::new();
        init_test_plugin(&plugin, workspace.path()).await;

        let first = plugin
            .invoke_refresh(CommandsRefreshInput::default())
            .await
            .expect("initial refresh");
        let first_payload = first.payload.expect("initial refresh payload");
        assert_eq!(first_payload["changed"], true);
        assert_eq!(first_payload["generation"], 1);

        let unchanged = plugin
            .invoke_refresh(CommandsRefreshInput::default())
            .await
            .expect("unchanged refresh");
        let unchanged_payload = unchanged.payload.expect("unchanged refresh payload");
        assert_eq!(unchanged_payload["changed"], false);
        assert_eq!(unchanged_payload["generation"], 1);

        let added_dir = workspace.path().join(".agena/skills/refreshed-package");
        std::fs::create_dir_all(&added_dir).expect("added package dir");
        std::fs::write(
            added_dir.join("SKILL.md"),
            "---\nname: refreshed_package\ndescription: refresh test\n---\nFresh.",
        )
        .expect("added package");
        let changed = plugin
            .invoke_refresh(CommandsRefreshInput::default())
            .await
            .expect("changed refresh");
        let changed_payload = changed.payload.expect("changed refresh payload");
        assert_eq!(changed_payload["changed"], true);
        assert_eq!(changed_payload["generation"], 2);
        plugin
            .invoke_get(CommandsGetInput {
                name: "refreshed_package".to_string(),
            })
            .await
            .expect("newly discovered package");
    }

    #[tokio::test]
    async fn workspace_managed_packages_support_install_read_and_remove() {
        let workspace = tempfile::tempdir().expect("workspace");
        let plugin = CommandsPlugin::new();
        init_test_plugin(&plugin, workspace.path()).await;

        let created = plugin
            .invoke_install(CommandsInstallInput {
                document: "---\nname: team_review\ndescription: Review a proposed change\naliases: [review-team]\n---\nReview the change carefully.\n".to_string(),
            })
            .await
            .expect("install managed package");
        assert_eq!(
            created
                .payload
                .as_ref()
                .and_then(|value| value.get("operation"))
                .and_then(Value::as_str),
            Some("created")
        );

        let listed = plugin
            .invoke_list(CommandsListInput {
                offset: None,
                limit: None,
                verbose: false,
            })
            .await
            .expect("list packages");
        let entry = listed
            .payload
            .as_ref()
            .and_then(|value| value.get("packages"))
            .and_then(Value::as_array)
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|entry| entry.get("name").and_then(Value::as_str) == Some("team_review"))
            })
            .expect("installed package entry");
        assert_eq!(entry.get("editable").and_then(Value::as_bool), Some(true));

        let loaded = plugin
            .invoke_get(CommandsGetInput {
                name: "review-team".to_string(),
            })
            .await
            .expect("read by alias");
        assert!(
            loaded
                .payload
                .as_ref()
                .and_then(|value| value.get("document"))
                .and_then(Value::as_str)
                .is_some_and(|document| document.contains("Review the change carefully."))
        );

        let resource = workspace
            .path()
            .join(".agena/skills/team_review/example.txt");
        std::fs::write(&resource, "first resource").unwrap();
        let request = CommandsReadResourceInput {
            name: "team_review".into(),
            path: "example.txt".into(),
            max_bytes: 1024,
        };
        let first = plugin
            .invoke_read_resource(request)
            .await
            .unwrap()
            .payload
            .unwrap();
        assert_eq!(first["content"], "first resource");
        assert!(first["package_content_hash"].is_string());

        plugin
            .invoke_remove(CommandsRemoveInput {
                name: "review-team".to_string(),
            })
            .await
            .expect("remove by alias");
        assert!(
            plugin
                .invoke_get(CommandsGetInput {
                    name: "team_review".to_string(),
                })
                .await
                .is_err()
        );
        assert!(
            !workspace
                .path()
                .join(".agena/skills/team_review/SKILL.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn managed_packages_cannot_rewrite_read_only_catalog_entries() {
        let workspace = tempfile::tempdir().expect("workspace");
        let plugin = CommandsPlugin::new();
        init_test_plugin(&plugin, workspace.path()).await;

        let error = plugin
            .invoke_remove(CommandsRemoveInput {
                name: "review".to_string(),
            })
            .await
            .expect_err("declared packages must remain read-only");
        assert!(error.diagnostic_message().contains("does not exist"));
        assert!(error.to_string().contains("does not exist"));
    }

    #[tokio::test]
    async fn config_rejects_workspace_escape_roots() {
        let workspace = tempfile::tempdir().expect("workspace");
        let plugin = CommandsPlugin::new();
        let result = plugin
            .init(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: workspace.path().to_path_buf(),
                    plugin_id: PluginKey::new("agena", "commands").expect("plugin key"),
                    host_callback_url: None,
                    host_callback_token: None,
                    settings: serde_json::json!({ "additional_roots": ["../outside"] }),
                    protocol_version: 1,
                },
                Arc::new(NoopHostClient),
            )
            .await;
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn config_rejects_existing_roots_that_symlink_outside_workspace() {
        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        let link = workspace.path().join("linked-packages");
        std::os::unix::fs::symlink(outside.path(), &link).expect("create outside symlink");
        let plugin = CommandsPlugin::new();
        let result = plugin
            .init(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: workspace.path().to_path_buf(),
                    plugin_id: PluginKey::new("agena", "commands").expect("plugin key"),
                    host_callback_url: None,
                    host_callback_token: None,
                    settings: serde_json::json!({ "additional_roots": ["linked-packages"] }),
                    protocol_version: 1,
                },
                Arc::new(NoopHostClient),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn running_a_package_returns_its_instructions_as_an_inserted_prompt() {
        let workspace = tempfile::tempdir().expect("workspace");
        let plugin = CommandsPlugin::new();
        init_test_plugin(&plugin, workspace.path()).await;

        let result = plugin
            .command_invoke(CommandInvokeInput {
                command_id: HANDLER_RUN.to_string(),
                input: serde_json::json!({ "args": "focus on the parser" }),
                session_id: None,
                call_id: None,
                workspace_root: None,
                slash: Some("/verify".to_string()),
                raw: String::new(),
            })
            .await
            .expect("invoke a declared package");
        assert_eq!(
            result.status,
            agena_plugin_host::sdk::CommandStatus::Succeeded
        );
        let inserted = result
            .effects
            .iter()
            .find_map(|effect| match effect {
                CommandHostEffect::InsertPrompt { prompt } => Some(prompt.as_str()),
                _ => None,
            })
            .expect("the insertion effect");
        assert!(inserted.contains("Verify the current work from evidence."));
        assert!(inserted.ends_with("focus on the parser"));
    }

    #[test]
    fn a_package_command_carries_one_optional_text_argument() {
        let contract = package_contract();
        // A bare tail is accepted as the single field's value, which is what a
        // composer produces when a user types `/verify check the diff`.
        let parsed = contract
            .parse_shorthand("check the diff")
            .expect("shorthand parses");
        assert_eq!(parsed["args"], "check the diff");
        assert_eq!(
            contract.parse_shorthand("").expect("empty shorthand"),
            serde_json::json!({})
        );
        assert!(!contract.accepts_only_empty_object());
    }
}
