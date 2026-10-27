use std::{io::Write, path::PathBuf, sync::Arc};

use agena_skills::{
    DeclaredSkill, SkillErrorKind, SkillImport, SkillImportFile, SkillManager, SkillScope,
    SkillSource, package_enabled_override,
};

fn document(name: &str, body: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: Use this skill to review source evidence.\nlicense: MIT\nmetadata:\n  owner: Example\n---\n{body}\n"
    )
}

struct Fixture {
    _temp: tempfile::TempDir,
    workspace: PathBuf,
    global: PathBuf,
    manager: SkillManager,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let global = temp.path().join("global/skills");
    std::fs::create_dir_all(&workspace).unwrap();
    let base = temp.path().to_path_buf();
    let manager = SkillManager::new(workspace.clone(), global.clone())
        .with_read_filter(Arc::new(move |path| path.starts_with(&base)));
    Fixture {
        _temp: temp,
        workspace,
        global,
        manager,
    }
}

fn input_file(path: &str, content: impl Into<Vec<u8>>) -> SkillImportFile {
    SkillImportFile {
        path: path.into(),
        content: content.into(),
        executable: false,
    }
}

#[test]
fn managed_scopes_override_without_losing_disabled_or_shadowed_entries() {
    let fixture = fixture();
    let global = fixture
        .manager
        .create(SkillScope::Global, &document("review-evidence", "Global"))
        .unwrap();
    let project = fixture
        .manager
        .create(
            SkillScope::Workspace,
            &document("review-evidence", "Project"),
        )
        .unwrap();
    assert_ne!(global.summary.id, project.summary.id);
    let catalog = fixture.manager.list().unwrap();
    assert!(
        catalog
            .skills
            .iter()
            .find(|skill| skill.id == global.summary.id)
            .unwrap()
            .shadowed
    );
    assert!(
        !catalog
            .skills
            .iter()
            .find(|skill| skill.id == project.summary.id)
            .unwrap()
            .shadowed
    );
    let disabled = fixture
        .manager
        .set_enabled(&project.summary.id, false, Some(&project.summary.revision))
        .unwrap();
    assert!(!disabled.summary.enabled);
    assert_eq!(
        package_enabled_override(std::path::Path::new(&disabled.summary.path)),
        Some(false)
    );
    let catalog = fixture.manager.list().unwrap();
    assert!(
        !catalog
            .skills
            .iter()
            .find(|skill| skill.id == global.summary.id)
            .unwrap()
            .shadowed
    );
    let reopened = fixture.manager.get(&disabled.summary.id).unwrap();
    assert!(!reopened.summary.enabled);
}

#[test]
fn explicit_enable_overrides_legacy_disabled_names() {
    let fixture = fixture();
    let path = fixture.workspace.join(".agena/skills/review-evidence");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("SKILL.md"), document("review-evidence", "Legacy")).unwrap();
    let manager = fixture
        .manager
        .clone()
        .with_disabled_names(vec!["review-evidence".into()]);
    let original = manager
        .list()
        .unwrap()
        .skills
        .into_iter()
        .find(|skill| skill.name == "review-evidence")
        .unwrap();
    assert!(!original.enabled);
    assert_eq!(package_enabled_override(&path.join("SKILL.md")), None);
    let updated = manager
        .set_enabled(&original.id, true, Some(&original.revision))
        .unwrap();
    assert!(updated.summary.enabled);
    assert_eq!(package_enabled_override(&path.join("SKILL.md")), Some(true));
}

#[test]
fn document_and_resource_edits_use_optimistic_revisions() {
    let fixture = fixture();
    let skill = fixture
        .manager
        .create(SkillScope::Workspace, &document("edit-example", "Original"))
        .unwrap();
    let changed = fixture
        .manager
        .update(
            &skill.summary.id,
            &document("edit-example", "Updated"),
            Some(&skill.summary.revision),
        )
        .unwrap();
    assert_ne!(skill.summary.revision, changed.summary.revision);
    assert_eq!(
        fixture
            .manager
            .update(
                &skill.summary.id,
                &document("edit-example", "Stale"),
                Some(&skill.summary.revision)
            )
            .unwrap_err()
            .kind,
        SkillErrorKind::Conflict
    );
    assert_eq!(
        fixture
            .manager
            .remove(&skill.summary.id, Some(&skill.summary.revision))
            .unwrap_err()
            .kind,
        SkillErrorKind::Conflict
    );
    let resource = fixture
        .manager
        .write_resource(
            &skill.summary.id,
            "references/source.md",
            "  exact\ntext\n",
            Some(""),
        )
        .unwrap();
    assert_eq!(
        fixture
            .manager
            .read_resource(&skill.summary.id, "references/source.md")
            .unwrap(),
        resource
    );
    assert_eq!(
        fixture
            .manager
            .write_resource(&skill.summary.id, "references/source.md", "New", Some(""))
            .unwrap_err()
            .kind,
        SkillErrorKind::Conflict
    );
    let revised = fixture
        .manager
        .write_resource(
            &skill.summary.id,
            "references/source.md",
            "New",
            Some(&resource.revision),
        )
        .unwrap();
    assert_ne!(revised.revision, resource.revision);
    assert_eq!(
        fixture
            .manager
            .remove_resource(
                &skill.summary.id,
                "references/source.md",
                Some(&resource.revision)
            )
            .unwrap_err()
            .kind,
        SkillErrorKind::Conflict
    );
    let detail = fixture
        .manager
        .remove_resource(
            &skill.summary.id,
            "references/source.md",
            Some(&revised.revision),
        )
        .unwrap();
    assert!(
        !detail
            .files
            .iter()
            .any(|file| file.path == "references/source.md")
    );
    for invalid in [
        "../outside",
        "/outside",
        r"C:\outside",
        "references/../../outside",
        "SKILL.md",
        ".agena-skill.json",
    ] {
        assert!(
            fixture
                .manager
                .write_resource(&skill.summary.id, invalid, "No", Some(""))
                .is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn removal_removes_the_entire_package() {
    let fixture = fixture();
    let skill = fixture
        .manager
        .create(SkillScope::Workspace, &document("remove-example", "Remove"))
        .unwrap();
    fixture
        .manager
        .write_resource(
            &skill.summary.id,
            "assets/nested/info.txt",
            "Resource",
            Some(""),
        )
        .unwrap();
    let current = fixture.manager.get(&skill.summary.id).unwrap();
    let removed = fixture
        .manager
        .remove(&skill.summary.id, Some(&current.summary.revision))
        .unwrap();
    assert_eq!(removed.name, "remove-example");
    assert!(
        !fixture
            .workspace
            .join(".agena/skills/remove-example")
            .exists()
    );
    assert!(fixture.manager.get(&skill.summary.id).is_err());
}

#[test]
fn invalid_managed_documents_remain_visible_and_can_be_repaired() {
    let fixture = fixture();
    let native = fixture.workspace.join(".agena/skills/broken-example");
    std::fs::create_dir_all(&native).unwrap();
    std::fs::write(
        native.join("SKILL.md"),
        "---\nname: broken-example\n---\nMissing description",
    )
    .unwrap();
    let summary = fixture
        .manager
        .list()
        .unwrap()
        .skills
        .into_iter()
        .find(|skill| skill.name == "broken-example")
        .unwrap();
    assert!(!summary.valid);
    assert!(summary.editable);
    assert!(!summary.diagnostics.is_empty());
    let repaired = fixture
        .manager
        .update(
            &summary.id,
            &document("broken-example", "Repaired"),
            Some(&summary.revision),
        )
        .unwrap();
    assert!(repaired.summary.valid);
}

#[test]
fn compatible_roots_and_declared_skills_are_read_only() {
    let fixture = fixture();
    let compatible = fixture.workspace.join(".agents/skills/shared-example");
    std::fs::create_dir_all(&compatible).unwrap();
    std::fs::write(
        compatible.join("SKILL.md"),
        document("shared-example", "Shared"),
    )
    .unwrap();
    let manager = fixture
        .manager
        .clone()
        .with_declared_skills(vec![DeclaredSkill {
            plugin_id: "test.plugin".into(),
            version: "1.0".into(),
            name: "declared_example".into(),
            description: "Legacy plugin declaration".into(),
            instructions: "Instructions".into(),
            aliases: vec!["declared".into()],
        }]);
    let catalog = manager.list().unwrap();
    let shared = catalog
        .skills
        .iter()
        .find(|skill| skill.name == "shared-example")
        .unwrap();
    let declared = catalog
        .skills
        .iter()
        .find(|skill| skill.name == "declared_example")
        .unwrap();
    assert!(!shared.editable);
    assert!(!declared.editable);
    assert!(declared.valid);
    assert_eq!(
        manager.remove(&shared.id, None).unwrap_err().kind,
        SkillErrorKind::ReadOnly
    );
    assert_eq!(
        manager
            .set_enabled(&declared.id, false, None)
            .unwrap_err()
            .kind,
        SkillErrorKind::ReadOnly
    );
    assert!(
        manager
            .get(&declared.id)
            .unwrap()
            .document
            .contains("Instructions")
    );
    assert!(
        manager
            .resolve_path(&declared.id)
            .unwrap()
            .to_string_lossy()
            .starts_with("plugin://")
    );
}

#[test]
fn folder_import_copies_resources_and_records_provenance() {
    let fixture = fixture();
    let source = fixture.workspace.join("source/reusable-example");
    std::fs::create_dir_all(source.join("assets")).unwrap();
    std::fs::create_dir_all(source.join("scripts")).unwrap();
    std::fs::write(
        source.join("SKILL.md"),
        document("reusable-example", "Use assets"),
    )
    .unwrap();
    std::fs::write(source.join("assets/image.bin"), [0, 255, 42, 0]).unwrap();
    std::fs::write(source.join("scripts/run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            source.join("scripts/run.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let import = SkillImport::from_folder(&source).unwrap();
    let preview = fixture
        .manager
        .preview_install(&import, SkillScope::Workspace)
        .unwrap();
    assert_eq!(preview.candidates.len(), 1);
    assert_eq!(preview.candidates[0].path, ".");
    assert_eq!(preview.revision, import.revision());
    let result = fixture
        .manager
        .install(
            &import,
            SkillScope::Workspace,
            &[],
            false,
            Some(SkillSource {
                kind: "folder".into(),
                location: source.to_string_lossy().into_owned(),
                reference: None,
                subpath: None,
                revision: None,
            }),
        )
        .unwrap();
    let installed = &result.installed[0];
    assert_eq!(
        installed.source.as_ref().unwrap().revision.as_deref(),
        Some(import.revision().as_str())
    );
    let destination = fixture.workspace.join(".agena/skills/reusable-example");
    assert_eq!(
        std::fs::read(destination.join("assets/image.bin")).unwrap(),
        [0, 255, 42, 0]
    );
    assert!(
        fixture
            .manager
            .read_resource(&installed.id, "assets/image.bin")
            .is_err()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(
            std::fs::metadata(destination.join("scripts/run.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o100,
            0
        );
        let resource = fixture
            .manager
            .read_resource(&installed.id, "scripts/run.sh")
            .unwrap();
        fixture
            .manager
            .write_resource(
                &installed.id,
                "scripts/run.sh",
                "#!/bin/sh\nexit 1\n",
                Some(&resource.revision),
            )
            .unwrap();
        assert_ne!(
            std::fs::metadata(destination.join("scripts/run.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o100,
            0
        );
    }
}

#[test]
fn install_preflights_all_conflicts_before_mutating_any_package() {
    let fixture = fixture();
    let old = fixture
        .manager
        .create(
            SkillScope::Workspace,
            &document("second-example", "Existing"),
        )
        .unwrap();
    let import = SkillImport::from_files(vec![
        input_file("first-example/SKILL.md", document("first-example", "First")),
        input_file(
            "second-example/SKILL.md",
            document("second-example", "Replacement"),
        ),
    ])
    .unwrap();
    let preview = fixture
        .manager
        .preview_install(&import, SkillScope::Workspace)
        .unwrap();
    assert!(
        preview
            .candidates
            .iter()
            .find(|candidate| candidate.name == "second-example")
            .unwrap()
            .conflict
    );
    assert_eq!(
        fixture
            .manager
            .install(&import, SkillScope::Workspace, &[], false, None)
            .unwrap_err()
            .kind,
        SkillErrorKind::Conflict
    );
    assert!(
        !fixture
            .workspace
            .join(".agena/skills/first-example")
            .exists()
    );
    assert_eq!(
        fixture.manager.get(&old.summary.id).unwrap().document,
        old.document
    );
    let disabled = fixture
        .manager
        .set_enabled(&old.summary.id, false, Some(&old.summary.revision))
        .unwrap();
    let result = fixture
        .manager
        .install(
            &import,
            SkillScope::Workspace,
            &["second-example".into()],
            true,
            None,
        )
        .unwrap();
    assert_eq!(result.installed.len(), 1);
    assert!(!result.installed[0].enabled);
    assert_ne!(result.installed[0].revision, disabled.summary.revision);
    assert!(
        !fixture
            .workspace
            .join(".agena/skills/first-example")
            .exists()
    );
}

#[test]
fn upload_validation_rejects_duplicate_paths_and_keeps_internal_state_local() {
    assert!(SkillImport::from_files(vec![input_file("x", "one"), input_file("x", "two")]).is_err());
    assert!(
        SkillImport::from_files(vec![input_file("x", "one"), input_file("x/child", "two")])
            .is_err()
    );
    for path in ["../escape", "/absolute", r"C:\absolute", "a/../../escape"] {
        assert!(
            SkillImport::from_files(vec![input_file(path, "No")]).is_err(),
            "{path}"
        );
    }
    let fixture = fixture();
    let import = SkillImport::from_files(vec![
        input_file("SKILL.md", document("state-example", "Content")),
        input_file(
            ".agena-skill.json",
            r#"{"enabled":false,"source":{"kind":"fake","location":"fake"}}"#,
        ),
    ])
    .unwrap();
    let installed = fixture
        .manager
        .install(&import, SkillScope::Global, &[], false, None)
        .unwrap();
    assert!(installed.installed[0].enabled);
    assert!(installed.installed[0].source.is_none());
}

fn zip_files(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (path, content) in files {
        writer
            .start_file(*path, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(content).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn archive_install_handles_repository_wrappers_and_subdirectories() {
    let fixture = fixture();
    let bytes = zip_files(&[
        ("repository-main/README.md", b"Repository".to_vec()),
        (
            "repository-main/skills/archive-example/SKILL.md",
            document("archive-example", "Archive").into_bytes(),
        ),
        (
            "repository-main/skills/archive-example/assets/data.bin",
            vec![0, 255],
        ),
    ]);
    let import = SkillImport::from_archive("skills.zip", &bytes)
        .unwrap()
        .with_subpath("skills")
        .unwrap();
    let preview = fixture
        .manager
        .preview_install(&import, SkillScope::Workspace)
        .unwrap();
    assert_eq!(preview.candidates[0].path, "archive-example");
    let result = fixture
        .manager
        .install(
            &import,
            SkillScope::Workspace,
            &["archive-example".into()],
            false,
            None,
        )
        .unwrap();
    assert_eq!(result.installed[0].name, "archive-example");
    assert_eq!(
        std::fs::read(
            fixture
                .workspace
                .join(".agena/skills/archive-example/assets/data.bin")
        )
        .unwrap(),
        [0, 255]
    );
    assert!(
        SkillImport::from_archive("skills.zip", &bytes)
            .unwrap()
            .with_subpath("../outside")
            .is_err()
    );
}

#[test]
fn archives_reject_traversal_and_tar_links_without_writing_anything() {
    let bytes = zip_files(&[("../escape", b"No".to_vec())]);
    assert!(SkillImport::from_archive("evil.zip", &bytes).is_err());
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    header.set_link_name("/outside").unwrap();
    header.set_cksum();
    builder
        .append_data(&mut header, "escape", std::io::empty())
        .unwrap();
    let bytes = builder.into_inner().unwrap();
    assert!(SkillImport::from_archive("evil.tar", &bytes).is_err());
}

#[test]
fn tar_gzip_preserves_binary_resources() {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, content) in [
        (
            "package/SKILL.md",
            document("gzip-example", "Compressed").into_bytes(),
        ),
        ("package/assets/data.bin", vec![255, 0, 3]),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, path, content.as_slice())
            .unwrap();
    }
    let tar = builder.into_inner().unwrap();
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar).unwrap();
    let bytes = gzip.finish().unwrap();
    let import = SkillImport::from_archive("skills.tar.gz", &bytes).unwrap();
    let fixture = fixture();
    let result = fixture
        .manager
        .install(&import, SkillScope::Global, &[], false, None)
        .unwrap();
    assert_eq!(result.installed[0].name, "gzip-example");
    assert_eq!(
        std::fs::read(fixture.global.join("gzip-example/assets/data.bin")).unwrap(),
        [255, 0, 3]
    );
}

#[test]
fn read_filter_excludes_denied_documents_and_resources() {
    let fixture = fixture();
    let first = fixture
        .manager
        .create(SkillScope::Workspace, &document("first-example", "One"))
        .unwrap();
    let second = fixture
        .manager
        .create(SkillScope::Workspace, &document("second-example", "Two"))
        .unwrap();
    fixture
        .manager
        .write_resource(
            &first.summary.id,
            "references/private.txt",
            "Private",
            Some(""),
        )
        .unwrap();
    let denied = PathBuf::from(second.summary.path)
        .parent()
        .unwrap()
        .to_path_buf();
    let resource = fixture
        .workspace
        .join(".agena/skills/first-example/references/private.txt");
    let base = fixture.workspace.clone();
    let manager = fixture
        .manager
        .clone()
        .with_read_filter(Arc::new(move |path| {
            path.starts_with(&base) && !path.starts_with(&denied) && path != resource
        }));
    let catalog = manager.list().unwrap();
    assert_eq!(catalog.skills.len(), 1);
    // Identity lookup is metadata-only; reading the identified document is
    // still rejected by the content filter.
    assert!(manager.resolve_path(&second.summary.id).is_ok());
    assert!(manager.get(&second.summary.id).is_err());
    assert!(
        manager
            .read_resource(&first.summary.id, "references/private.txt")
            .is_err()
    );
    assert!(
        !manager
            .get(&first.summary.id)
            .unwrap()
            .files
            .iter()
            .any(|file| file.path == "references/private.txt")
    );
}

#[cfg(unix)]
#[test]
fn symbolic_links_cannot_escape_imports_or_managed_resources() {
    use std::os::unix::fs::symlink;
    let fixture = fixture();
    let outside = fixture._temp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "Outside").unwrap();
    let skill = fixture
        .manager
        .create(SkillScope::Workspace, &document("links-example", "Links"))
        .unwrap();
    let directory = fixture.workspace.join(".agena/skills/links-example");
    symlink(&outside, directory.join("linked")).unwrap();
    assert!(
        fixture
            .manager
            .write_resource(&skill.summary.id, "linked/secret.txt", "Changed", None)
            .is_err()
    );
    assert!(
        fixture
            .manager
            .read_resource(&skill.summary.id, "linked/secret.txt")
            .is_err()
    );
    assert!(SkillImport::from_folder(&directory).is_err());
    assert_eq!(
        std::fs::read_to_string(outside.join("secret.txt")).unwrap(),
        "Outside"
    );

    let other_workspace = fixture._temp.path().join("other-workspace");
    std::fs::create_dir(&other_workspace).unwrap();
    symlink(&outside, other_workspace.join(".agena")).unwrap();
    let other = SkillManager::new(other_workspace, fixture.global.clone());
    assert!(
        other
            .create(SkillScope::Workspace, &document("escape-example", "No"))
            .is_err()
    );
    assert!(!outside.join("skills").exists());
}

#[test]
fn replace_never_deletes_an_unrelated_destination_folder() {
    let fixture = fixture();
    let destination = fixture.workspace.join(".agena/skills/occupied-example");
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(destination.join("personal.txt"), "Keep").unwrap();
    let import = SkillImport::from_files(vec![input_file(
        "SKILL.md",
        document("occupied-example", "Import"),
    )])
    .unwrap();
    assert_eq!(
        fixture
            .manager
            .install(&import, SkillScope::Workspace, &[], true, None)
            .unwrap_err()
            .kind,
        SkillErrorKind::Conflict
    );
    assert_eq!(
        std::fs::read_to_string(destination.join("personal.txt")).unwrap(),
        "Keep"
    );
}
