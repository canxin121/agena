use std::fs;

use agena_skills::bundled;
use agena_skills::discovery::{default_command_roots, default_roots, scan, scan_commands};
use agena_skills::{Skill, SkillsManager};

const TEST_OWNER: &str = "test";

fn build_manager(workspace: Option<&std::path::Path>) -> SkillsManager {
    let mgr = SkillsManager::new();
    let roots = default_roots(workspace);
    let mut discovered = scan(&roots).unwrap_or_default();
    for b in bundled::all().unwrap_or_default() {
        if !discovered
            .iter()
            .any(|s| s.frontmatter.name == b.frontmatter.name)
        {
            discovered.push(b);
        }
    }
    for skill in discovered {
        mgr.register(TEST_OWNER, skill);
    }
    let command_roots = default_command_roots(workspace);
    for command in scan_commands(&command_roots).unwrap_or_default() {
        mgr.register_command(TEST_OWNER, command);
    }
    mgr
}

#[test]
fn parse_minimal_skill() {
    let raw = "---\nname: hello\ndescription: greet\n---\nbody here\n";
    let s = Skill::from_raw(raw).unwrap();
    assert_eq!(s.frontmatter.name, "hello");
    assert_eq!(s.body.trim(), "body here");
}

#[test]
fn bundled_skills_resolve() {
    let mgr = build_manager(None);
    let names: Vec<_> = mgr
        .list()
        .iter()
        .map(|s| s.frontmatter.name.clone())
        .collect();
    for required in ["init", "review", "security-review"] {
        assert!(names.contains(&required.to_string()), "missing {required}");
    }
    assert_eq!(mgr.get("review").unwrap().frontmatter.name, "review");
    assert!(mgr.get("does-not-exist").is_err());
}

#[test]
fn workspace_markdown_commands_resolve_by_name_and_alias() {
    let temp = tempfile::tempdir().unwrap();
    let commands_dir = temp.path().join(".agena").join("commands");
    fs::create_dir_all(&commands_dir).unwrap();
    fs::write(
        commands_dir.join("fix.md"),
        "---\nname: fix\ndescription: Fix a bug\nallowed_tools: [Read, Edit]\nmodel: claude-sonnet-4-6\naliases: [repair]\n---\nFix this: $ARGUMENTS\n",
    )
    .unwrap();
    fs::write(
        commands_dir.join("explain.md"),
        "---\ndescription: Explain code\n---\nExplain $ARGUMENTS\n",
    )
    .unwrap();

    let mgr = build_manager(Some(temp.path()));
    let command = mgr.get_command("repair").unwrap();

    assert_eq!(command.frontmatter.name, "fix");
    assert_eq!(command.frontmatter.allowed_tools, vec!["Read", "Edit"]);
    assert_eq!(
        command.frontmatter.model.as_deref(),
        Some("claude-sonnet-4-6")
    );
    assert!(command.body.contains("$ARGUMENTS"));
    assert_eq!(
        mgr.get_command("explain").unwrap().frontmatter.name,
        "explain"
    );
}

#[test]
fn registry_replaces_skill_owned_by_same_plugin() {
    let mgr = SkillsManager::new();
    let original = Skill::from_raw("---\nname: foo\n---\nbody-1\n").unwrap();
    let updated = Skill::from_raw("---\nname: foo\n---\nbody-2\n").unwrap();
    mgr.register("plugin-a", original);
    mgr.register("plugin-a", updated);
    let skills = mgr.list();
    assert_eq!(skills.len(), 1);
    assert!(skills[0].body.contains("body-2"));
}

#[test]
fn registry_keeps_separate_owners() {
    let mgr = SkillsManager::new();
    let s1 = Skill::from_raw("---\nname: foo\n---\nfrom-a\n").unwrap();
    let s2 = Skill::from_raw("---\nname: foo\n---\nfrom-b\n").unwrap();
    mgr.register("plugin-a", s1);
    mgr.register("plugin-b", s2);
    assert_eq!(mgr.list().len(), 2);
    mgr.remove_owned_by("plugin-a");
    let remaining = mgr.list_with_owners();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].0, "plugin-b");
}
