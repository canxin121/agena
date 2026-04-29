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
    let names: Vec<_> = mgr.list().iter().map(|s| s.frontmatter.name.clone()).collect();
    for required in ["init", "review", "security-review"] {
        assert!(names.contains(&required.to_string()), "missing {required}");
    }
    assert_eq!(mgr.get("review").unwrap().frontmatter.name, "review");
    assert!(mgr.get("does-not-exist").is_err());
}
