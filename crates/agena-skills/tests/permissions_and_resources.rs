use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use agena_skills::{SkillImport, SkillManager, SkillScope};

fn document(name: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: Use this skill for package regression checks.\n---\nRead supporting files.\n"
    )
}

#[test]
fn targeted_operations_do_not_request_unrelated_catalog_reads() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let unrelated = workspace.join("other-skills/unrelated");
    std::fs::create_dir_all(&unrelated).unwrap();
    std::fs::write(unrelated.join("SKILL.md"), document("unrelated")).unwrap();
    let manager = SkillManager::new(workspace.clone(), temporary.path().join("global/skills"))
        .with_additional_roots(vec![PathBuf::from("other-skills")]);
    let target = workspace.join(".agena/skills/target-example");
    let denied = Arc::new(AtomicUsize::new(0));
    let count = denied.clone();
    let restricted = manager.with_read_filter(Arc::new(move |path| {
        if path.starts_with(&target) {
            true
        } else {
            count.fetch_add(1, Ordering::SeqCst);
            false
        }
    }));
    let created = restricted
        .create(SkillScope::Workspace, &document("target-example"))
        .unwrap();
    assert_eq!(denied.load(Ordering::SeqCst), 0);
    let path = restricted.resolve_path(&created.summary.id).unwrap();
    assert!(path.ends_with("target-example/SKILL.md"));
    let detail = restricted.get(&created.summary.id).unwrap();
    restricted
        .update(
            &detail.summary.id,
            &document("target-example"),
            Some(&detail.summary.revision),
        )
        .unwrap();
    assert_eq!(denied.load(Ordering::SeqCst), 0);
}

#[test]
fn complete_packages_preserve_arbitrary_support_directories() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let folder = temporary.path().join("source");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("SKILL.md"), document("complete-example")).unwrap();
    for relative in [
        "dist/main.js",
        "assets/target/payload.bin",
        "node_modules/tiny/index.js",
        "__pycache__/fixture.bin",
    ] {
        let file = folder.join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, relative.as_bytes()).unwrap();
    }
    let package = SkillImport::from_folder(&folder).unwrap();
    let manager = SkillManager::new(workspace.clone(), temporary.path().join("global/skills"));
    let result = manager
        .install(&package, SkillScope::Workspace, &[], false, None)
        .unwrap();
    for relative in [
        "dist/main.js",
        "assets/target/payload.bin",
        "node_modules/tiny/index.js",
        "__pycache__/fixture.bin",
    ] {
        assert_eq!(
            manager
                .read_resource(&result.installed[0].id, relative)
                .unwrap()
                .content,
            relative
        );
    }
}
