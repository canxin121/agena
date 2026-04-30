use std::fs;

use agena_skills::{Skill, SkillsManager};

#[test]
fn parse_minimal_skill() {
    let raw = "---\nname: hello\ndescription: greet\n---\nbody here\n";
    let s = Skill::from_raw(raw).unwrap();
    assert_eq!(s.frontmatter.name, "hello");
    assert_eq!(s.body.trim(), "body here");
}

#[test]
fn bundled_skills_resolve() {
    let mgr = SkillsManager::build(None).unwrap();
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

    let mgr = SkillsManager::build(Some(temp.path())).unwrap();
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
