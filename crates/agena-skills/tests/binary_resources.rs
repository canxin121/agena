use agena_skills::{
    DeclaredSkill, MAX_DOCUMENT_BYTES, SkillErrorKind, SkillImport, SkillImportFile, SkillManager,
    SkillScope,
};

fn document(name: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: Use this skill for package regression checks.\n---\nInspect assets.\n"
    )
}

#[test]
fn binary_and_large_resources_can_be_removed_with_current_package_revision() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let manager = SkillManager::new(workspace.clone(), temporary.path().join("global/skills"));
    let package = SkillImport::from_files(vec![
        SkillImportFile {
            path: "SKILL.md".into(),
            content: document("binary-example").into_bytes(),
            executable: false,
        },
        SkillImportFile {
            path: "assets/image.bin".into(),
            content: vec![0, 255, 42],
            executable: false,
        },
        SkillImportFile {
            path: "assets/large.txt".into(),
            content: vec![b'a'; MAX_DOCUMENT_BYTES + 1],
            executable: false,
        },
    ])
    .unwrap();
    let installed = manager
        .install(&package, SkillScope::Workspace, &[], false, None)
        .unwrap();
    let skill = &installed.installed[0];
    assert!(
        manager
            .read_resource(&skill.id, "assets/image.bin")
            .is_err()
    );
    assert!(
        manager
            .read_resource(&skill.id, "assets/large.txt")
            .is_err()
    );

    manager
        .write_resource(&skill.id, "references/new.txt", "Concurrent edit", Some(""))
        .unwrap();
    let stale = manager
        .remove_resource(&skill.id, "assets/image.bin", Some(&skill.revision))
        .unwrap_err();
    assert_eq!(stale.kind, SkillErrorKind::Conflict);
    assert_eq!(
        std::fs::read(workspace.join(".agena/skills/binary-example/assets/image.bin")).unwrap(),
        [0, 255, 42]
    );

    let current = manager.get(&skill.id).unwrap();
    let after_binary = manager
        .remove_resource(
            &skill.id,
            "assets/image.bin",
            Some(&current.summary.revision),
        )
        .unwrap();
    assert!(
        !after_binary
            .files
            .iter()
            .any(|file| file.path == "assets/image.bin")
    );
    let after_large = manager
        .remove_resource(
            &skill.id,
            "assets/large.txt",
            Some(&after_binary.summary.revision),
        )
        .unwrap();
    assert!(
        !after_large
            .files
            .iter()
            .any(|file| file.path == "assets/large.txt")
    );
    assert!(
        after_large
            .files
            .iter()
            .any(|file| file.path == "references/new.txt")
    );
}

#[test]
fn administrative_declared_detail_preserves_catalog_shadow_status() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let manager = SkillManager::new(workspace, temporary.path().join("global/skills"))
        .with_declared_skills(vec![DeclaredSkill {
            plugin_id: "test.plugin".into(),
            version: "1".into(),
            name: "shadow-example".into(),
            description: "Plugin example".into(),
            instructions: "Original".into(),
            aliases: Vec::new(),
        }]);
    let declared = manager
        .list()
        .unwrap()
        .skills
        .into_iter()
        .find(|skill| skill.path.starts_with("plugin://"))
        .unwrap();
    manager
        .create(SkillScope::Workspace, &document("shadow-example"))
        .unwrap();
    assert!(manager.get(&declared.id).unwrap().summary.shadowed);
}
