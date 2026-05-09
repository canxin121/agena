use std::fs;

use agena_skills::Skill;
use agena_skills::bundled;
use agena_skills::discovery::{default_command_roots, default_roots, scan, scan_commands};

#[test]
fn parse_minimal_skill() {
    let raw = "---\nname: hello\ndescription: greet\n---\nbody here\n";
    let skill = Skill::from_raw(raw).unwrap();
    assert_eq!(skill.frontmatter.name, "hello");
    assert_eq!(skill.body.trim(), "body here");
}

#[test]
fn bundled_skills_include_expected_workflows() {
    let names: Vec<_> = bundled::all()
        .unwrap()
        .into_iter()
        .map(|skill| skill.frontmatter.name)
        .collect();
    for required in ["init", "review", "security-review"] {
        assert!(names.contains(&required.to_string()), "missing {required}");
    }
}

#[test]
fn workspace_skills_are_discovered_from_default_roots() {
    let temp = tempfile::tempdir().unwrap();
    let skills_dir = temp.path().join(".agena").join("skills").join("demo");
    fs::create_dir_all(&skills_dir).unwrap();
    fs::write(
        skills_dir.join("SKILL.md"),
        "---\nname: explain\ndescription: Explain code\naliases: [describe]\n---\nExplain $ARGUMENTS\n",
    )
    .unwrap();

    let roots = default_roots(Some(temp.path()));
    let discovered = scan(&roots).unwrap();
    let skill = discovered
        .into_iter()
        .find(|skill| skill.frontmatter.name == "explain")
        .expect("explain skill should be discovered");

    assert!(skill.matches("describe"));
    assert!(skill.body.contains("$ARGUMENTS"));
}

#[test]
fn workspace_markdown_commands_resolve_name_and_aliases() {
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

    let roots = default_command_roots(Some(temp.path()));
    let discovered = scan_commands(&roots).unwrap();
    let fix = discovered
        .iter()
        .find(|skill| skill.matches("repair"))
        .expect("repair alias should resolve");
    let explain = discovered
        .iter()
        .find(|skill| skill.frontmatter.name == "explain")
        .expect("command file stem should become default name");

    assert_eq!(fix.frontmatter.name, "fix");
    assert_eq!(fix.frontmatter.allowed_tools, vec!["Read", "Edit"]);
    assert_eq!(fix.frontmatter.model.as_deref(), Some("claude-sonnet-4-6"));
    assert!(fix.body.contains("$ARGUMENTS"));
    assert!(explain.body.contains("Explain $ARGUMENTS"));
}
