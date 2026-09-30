//! The shipped permission defaults, end to end.
//!
//! These used to be keys of their own - a `permission.guards` section, then two
//! keys on `path` and five on `tools`. They are now ordinary entries of the
//! collections that always existed: the temp and runtime-state paths are
//! `path.rules` entries, the interaction tool is a `tools.names` entry, and the
//! read-only and command classes are `tools.rules` entries. `global_default()`
//! is where they live, and it is the single place a configuration file is
//! merged onto, so these tests compile the shipped default and pin the three
//! properties that matter:
//!
//! - a built-in that answers `auto` really does reach the approval model;
//! - the user's own entry of the same name wins over the built-in one;
//! - deleting the user's entry restores the built-in.

use agena_domain::{
    AccessKind, PathAccessModes, PermissionConfig, PermissionDecision, PermissionMode,
    ToolPermissionRules,
};
use agena_plugin_host::sdk::ToolTag;
use agena_runtime_contracts::authorization::{
    apply_to_permission_policy, apply_to_tool_permission_policy,
};
use agena_runtime_contracts::permission::{PermissionPolicy, ToolPermissionPolicy};

/// The shipped configuration, with `edit` applied on top.
fn shipped(edit: impl FnOnce(&mut PermissionConfig)) -> PermissionConfig {
    let mut config = PermissionConfig::global_default();
    edit(&mut config);
    config
}

/// As [`shipped`], but with `tools.default` cleared so an entry of a tool class
/// or command class is the only thing that can answer. The shipped
/// `tools.default = allow` is a fallback that would otherwise mask whether the
/// class entry under test was consulted at all.
fn class_entries(edit: impl FnOnce(&mut PermissionConfig)) -> PermissionConfig {
    shipped(|config| {
        config.tools.as_mut().expect("tools section").default = None;
        edit(config);
    })
}

/// Compile the tool half of `config` the way the runtime does.
fn tool_policy(config: &PermissionConfig) -> ToolPermissionPolicy {
    apply_to_tool_permission_policy(config, ToolPermissionPolicy::new(PermissionMode::Auto))
        .expect("tool policy compiles")
}

/// Compile the path half of `config` the way the runtime does.
fn path_policy(config: &PermissionConfig) -> PermissionPolicy {
    apply_to_permission_policy(
        config,
        PermissionPolicy::new(PermissionMode::Auto, PermissionMode::Auto),
    )
    .expect("path policy compiles")
}

fn check(
    policy: &ToolPermissionPolicy,
    name: &str,
    tags: &[ToolTag],
    command: Option<&str>,
) -> PermissionDecision {
    policy.check_tool(name, command, tags)
}

fn path_decision(
    config: &PermissionConfig,
    workspace: &str,
    target: &str,
    access: AccessKind,
) -> PermissionDecision {
    path_policy(config).check_access(
        access,
        std::path::Path::new(workspace),
        std::path::Path::new(target),
    )
}

fn read_only_tags() -> Vec<ToolTag> {
    vec![ToolTag::ReadOnly]
}

fn interactive_tags() -> Vec<ToolTag> {
    vec![ToolTag::Interactive]
}

fn shell_tags() -> Vec<ToolTag> {
    vec![ToolTag::Shell]
}

fn temp_file(name: &str) -> String {
    std::env::temp_dir()
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// The `agena.shell.run` command-class table of `config`.
fn shell_classes(config: &PermissionConfig) -> indexmap::IndexMap<String, PermissionMode> {
    match config
        .tools
        .as_ref()
        .expect("tools section")
        .rules
        .get("agena.shell.run")
    {
        Some(ToolPermissionRules::Ordered(entries)) => entries.clone(),
        other => panic!("the shipped shell rules are an ordered table, got {other:?}"),
    }
}

/// Set one command-class keyword of `agena.shell.run`, or clear it when `mode`
/// is `None`.
fn set_shell_class(config: &mut PermissionConfig, keyword: &str, mode: Option<PermissionMode>) {
    let mut entries = shell_classes(config);
    match mode {
        Some(mode) => {
            entries.insert(keyword.to_owned(), mode);
        }
        None => {
            entries.shift_remove(keyword);
        }
    }
    config.tools.as_mut().expect("tools section").rules.insert(
        "agena.shell.run".to_owned(),
        ToolPermissionRules::Ordered(entries),
    );
}

/// Set the read-only class of the `*` tool entry, or clear it when `mode` is
/// `None`.
fn set_read_only_class(config: &mut PermissionConfig, mode: Option<PermissionMode>) {
    let tools = config.tools.as_mut().expect("tools section");
    match mode {
        Some(mode) => {
            tools.rules.insert(
                "*".to_owned(),
                ToolPermissionRules::Ordered(indexmap::IndexMap::from([(
                    agena_domain::TOOL_CLASS_READ_ONLY.to_owned(),
                    mode,
                )])),
            );
        }
        None => {
            tools.rules.remove("*");
        }
    }
}

#[test]
fn a_read_only_tool_is_allowed_by_the_default() {
    // The automatic-approval fast path used to answer this in process, after the
    // static policy had already returned `auto`. It is now an entry the compiled
    // policy applies itself, so it never reaches the approval model.
    let policy = tool_policy(&shipped(|_| {}));
    assert_eq!(
        check(&policy, "mcp.read_file", read_only_tags().as_slice(), None),
        PermissionDecision::Allow
    );
}

#[test]
fn the_read_only_class_set_to_auto_reaches_the_model() {
    let policy = tool_policy(&class_entries(|config| {
        set_read_only_class(config, Some(PermissionMode::Auto));
    }));
    let decision = check(&policy, "mcp.read_file", read_only_tags().as_slice(), None);
    assert!(
        matches!(decision, PermissionDecision::Auto { .. }),
        "a read-only tool with no class entry must reach the model, got {decision:?}"
    );
}

#[test]
fn omitting_an_entry_restores_its_built_in_value() {
    // Deleting the entry is the restore: an omitted entry falls back to the
    // built-in one rather than switching the class off. An absent section
    // resolves the same way, which is why a configuration file need not repeat
    // these values.
    let policy = tool_policy(&shipped(|config| set_read_only_class(config, None)));
    assert_eq!(
        check(&policy, "mcp.read_file", read_only_tags().as_slice(), None),
        PermissionDecision::Allow
    );
    let absent_sections = shipped(|config| {
        config.path = None;
        config.tools = None;
    });
    assert_eq!(
        check(
            &tool_policy(&absent_sections),
            "mcp.read_file",
            read_only_tags().as_slice(),
            None
        ),
        PermissionDecision::Allow,
        "a configuration with no sections at all still resolves the built-in defaults"
    );
}

#[test]
fn the_interaction_tool_is_allowed_so_its_own_ask_flow_runs() {
    // `agena.interaction.ask` *is* the prompt. A permission `Ask` here would
    // confirm the question instead of asking it, so the default is `Allow` and
    // the tool raises its own `UserInputRequired`.
    let policy = tool_policy(&shipped(|_| {}));
    assert_eq!(
        check(
            &policy,
            "agena.interaction.ask",
            interactive_tags().as_slice(),
            None
        ),
        PermissionDecision::Allow
    );
    assert_eq!(
        check(
            &policy,
            "interaction.ask",
            interactive_tags().as_slice(),
            None
        ),
        PermissionDecision::Allow,
        "the bare alias is the same tool"
    );
}

#[test]
fn omitting_the_interaction_entry_reaches_the_model() {
    // The entry is a `tools.names` entry like any other, so removing both
    // spellings leaves the tool to the ordinary policy - here, the fixture's
    // `Auto` default.
    let policy = tool_policy(&class_entries(|config| {
        config.tools.as_mut().expect("tools section").names.clear();
    }));
    let decision = check(
        &policy,
        "agena.interaction.ask",
        interactive_tags().as_slice(),
        None,
    );
    assert!(
        matches!(decision, PermissionDecision::Auto { .. }),
        "an interaction tool with no entry must reach the model, got {decision:?}"
    );
}

#[test]
fn a_configured_tool_rule_wins_over_the_interaction_default() {
    // The default is *policy*, not a boundary: an explicit entry for the tool's
    // name overrides it, which is what makes it visible and editable from the
    // settings surfaces.
    let config = shipped(|config| {
        config
            .tools
            .as_mut()
            .expect("tools section")
            .names
            .insert("agena.interaction.ask".to_owned(), PermissionMode::Deny);
    });
    let policy = tool_policy(&config);
    assert!(matches!(
        check(
            &policy,
            "agena.interaction.ask",
            interactive_tags().as_slice(),
            None
        ),
        PermissionDecision::Deny { .. }
    ));
}

#[test]
fn routine_commands_are_allowed_and_dangerous_ones_denied_by_default() {
    let policy = tool_policy(&shipped(|_| {}));
    assert_eq!(
        check(
            &policy,
            "agena.shell.run",
            shell_tags().as_slice(),
            Some("git status")
        ),
        PermissionDecision::Allow
    );
    assert!(matches!(
        check(
            &policy,
            "agena.shell.run",
            shell_tags().as_slice(),
            Some("rm -rf /")
        ),
        PermissionDecision::Deny { .. }
    ));
}

#[test]
fn a_dangerous_command_class_set_to_auto_reaches_the_model() {
    let policy = tool_policy(&shipped(|config| {
        set_shell_class(
            config,
            agena_domain::COMMAND_CLASS_DANGEROUS,
            Some(PermissionMode::Auto),
        );
    }));
    let decision = check(
        &policy,
        "agena.shell.run",
        shell_tags().as_slice(),
        Some("rm -rf /"),
    );
    assert!(
        matches!(decision, PermissionDecision::Auto { .. }),
        "a dangerous command with no class entry must reach the model, got {decision:?}"
    );
}

#[test]
fn an_ambiguous_command_reaches_the_model() {
    let policy = tool_policy(&class_entries(|_| {}));
    let decision = check(
        &policy,
        "agena.shell.run",
        shell_tags().as_slice(),
        Some("git push origin main"),
    );
    assert!(
        matches!(decision, PermissionDecision::Auto { .. }),
        "an unclassified command must reach the model, got {decision:?}"
    );
}

#[test]
fn a_configured_tool_rule_wins_over_a_command_class_entry() {
    // Ranking, stated once: the user's own rule for a tool outranks the command
    // class, which in turn outranks `tools.default`.
    let config = shipped(|config| {
        config
            .tools
            .as_mut()
            .expect("tools section")
            .names
            .insert("agena.shell.run".to_owned(), PermissionMode::Ask);
    });
    let policy = tool_policy(&config);
    assert!(
        matches!(
            check(
                &policy,
                "agena.shell.run",
                shell_tags().as_slice(),
                Some("git status")
            ),
            PermissionDecision::Ask { .. }
        ),
        "a tool the user named is answered by that rule, not by the class entry"
    );
}

#[test]
fn a_command_pattern_outranks_the_command_class() {
    // A pattern the user wrote for the same tool is matched before the class
    // entries, so a routine command can be narrowed without giving up the
    // class defaults for everything else.
    let config = shipped(|config| {
        config.tools.as_mut().expect("tools section").rules.insert(
            "agena.shell.run".to_owned(),
            ToolPermissionRules::Ordered(indexmap::IndexMap::from([
                ("git status *".to_owned(), PermissionMode::Deny),
                (
                    agena_domain::COMMAND_CLASS_ROUTINE.to_owned(),
                    PermissionMode::Allow,
                ),
            ])),
        );
    });
    let policy = tool_policy(&config);
    assert!(matches!(
        check(
            &policy,
            "agena.shell.run",
            shell_tags().as_slice(),
            Some("git status --short")
        ),
        PermissionDecision::Deny { .. }
    ));
    assert_eq!(
        check(
            &policy,
            "agena.shell.run",
            shell_tags().as_slice(),
            Some("ls")
        ),
        PermissionDecision::Allow,
        "the class entry still covers the other routine commands"
    );
}

#[test]
fn temp_paths_are_allowed_by_default_for_read_and_write() {
    let config = shipped(|_| {});
    let temp = temp_file("agena-defaults-test.bin");
    for access in [AccessKind::Read, AccessKind::Write] {
        assert_eq!(
            path_decision(&config, "/work", &temp, access),
            PermissionDecision::Allow,
            "a temp path must be allowed by the default ({access:?})"
        );
    }
    // Nothing leaks outside the temp directory: a workspace write is still the
    // policy's own default.
    assert!(matches!(
        path_decision(&config, "/work", "/work/src/main.rs", AccessKind::Write),
        PermissionDecision::Auto { .. }
    ));
}

#[test]
fn a_narrowed_temp_rule_reaches_the_model() {
    let config = shipped(|config| {
        config.path.as_mut().expect("path section").rules.insert(
            "<tmp>/**".to_owned(),
            PathAccessModes {
                read: Some(PermissionMode::Allow),
                write: Some(PermissionMode::Auto),
            },
        );
    });
    let temp = temp_file("agena-defaults-test.bin");
    let decision = path_decision(&config, "/work", &temp, AccessKind::Write);
    assert!(
        matches!(decision, PermissionDecision::Auto { .. }),
        "a temp path whose rule says `auto` must reach the model, got {decision:?}"
    );
    // The read half of the same rule still stands.
    assert_eq!(
        path_decision(&config, "/work", &temp, AccessKind::Read),
        PermissionDecision::Allow
    );
}

#[test]
fn the_runtime_state_directory_is_covered_by_its_own_rule() {
    // The rule is written against the `<home>` alias, which resolves to the same
    // `HOME` the runtime's `agena_home_dir()` reads, so a path under it is
    // allowed without the session layer injecting anything.
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_owned());
    let state_file = format!("{home}/agena/projects/-work/snapshots/tree.bin");
    let config = shipped(|_| {});
    for access in [AccessKind::Read, AccessKind::Write] {
        assert_eq!(
            path_decision(&config, "/work", &state_file, access),
            PermissionDecision::Allow,
            "the runtime state directory must be allowed ({access:?})"
        );
    }
}

#[test]
fn a_user_path_rule_overrides_the_default() {
    // The built-in entries are compiled before the user's `path.rules`, and the
    // rule list is last-match-wins, so an explicit rule wins.
    let temp = temp_file("agena-defaults-test.bin");
    let config = shipped(|config| {
        config.path.as_mut().expect("path section").rules.insert(
            temp.clone(),
            PathAccessModes {
                read: Some(PermissionMode::Deny),
                write: Some(PermissionMode::Deny),
            },
        );
    });
    assert!(matches!(
        path_decision(&config, "/work", &temp, AccessKind::Write),
        PermissionDecision::Deny { .. }
    ));
}

#[test]
fn the_shipped_default_is_ordinary_configuration() {
    // Nothing about the defaults is special-cased: they are the same shapes a
    // user writes, they survive a serialize/deserialize round trip, and a
    // configuration file that says nothing about permissions resolves to
    // exactly this.
    let config = PermissionConfig::global_default();
    let json = serde_json::to_string(&config).expect("the default serializes");
    let round_tripped: PermissionConfig =
        serde_json::from_str(&json).expect("the default deserializes");
    assert_eq!(round_tripped, config);

    let path_rules = &config.path.as_ref().expect("path section").rules;
    for pattern in [
        agena_domain::RUNTIME_STATE_PATH,
        agena_domain::RUNTIME_STATE_PATH_GLOB,
    ] {
        assert!(
            path_rules.contains_key(pattern),
            "the runtime state path is an ordinary path rule: {pattern}"
        );
    }
    assert_eq!(
        shell_classes(&config).get(agena_domain::COMMAND_CLASS_DANGEROUS),
        Some(&PermissionMode::Deny)
    );
    assert!(matches!(
        config
            .tools
            .as_ref()
            .expect("tools section")
            .rules
            .get("*"),
        Some(ToolPermissionRules::Ordered(entries))
            if entries.get(agena_domain::TOOL_CLASS_READ_ONLY) == Some(&PermissionMode::Allow)
    ));
}
