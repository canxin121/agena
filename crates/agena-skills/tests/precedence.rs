use std::sync::Arc;

use agena_skills::{SkillManager, SkillScope};

#[test]
fn invalid_project_copy_stays_repairable_without_hiding_a_valid_global_copy() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let base = temporary.path().to_path_buf();
    let manager = SkillManager::new(workspace, temporary.path().join("global/skills"))
        .with_read_filter(Arc::new(move |path| path.starts_with(&base)));
    let document = "---\nname: repairable-example\ndescription: Review source evidence.\n---\nRead the evidence.\n";
    let global = manager.create(SkillScope::Global, document).unwrap();
    let project = manager.create(SkillScope::Workspace, document).unwrap();
    std::fs::write(
        &project.summary.path,
        "---\nname: repairable-example\n---\nIncomplete metadata.\n",
    )
    .unwrap();

    let catalog = manager.list().unwrap();
    let global_summary = catalog
        .skills
        .iter()
        .find(|skill| skill.id == global.summary.id)
        .unwrap();
    let project_summary = catalog
        .skills
        .iter()
        .find(|skill| skill.id == project.summary.id)
        .unwrap();
    assert!(global_summary.valid && global_summary.enabled && !global_summary.shadowed);
    assert!(!project_summary.valid);
    assert!(project_summary.editable && project_summary.enabled);
    assert!(!project_summary.diagnostics.is_empty());

    manager
        .update(
            &project_summary.id,
            document,
            Some(&project_summary.revision),
        )
        .unwrap();
    let repaired = manager.list().unwrap();
    assert!(
        repaired
            .skills
            .iter()
            .find(|skill| skill.id == global.summary.id)
            .unwrap()
            .shadowed
    );
    let project_summary = repaired
        .skills
        .iter()
        .find(|skill| skill.id == project.summary.id)
        .unwrap();
    assert!(project_summary.valid && project_summary.enabled && !project_summary.shadowed);
}
