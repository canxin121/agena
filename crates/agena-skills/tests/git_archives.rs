use std::io::Write;

use agena_skills::{SkillErrorKind, SkillImport, SkillManager, SkillScope};

fn append(builder: &mut tar::Builder<Vec<u8>>, kind: tar::EntryType, path: &str, content: &[u8]) {
    let mut header = tar::Header::new_ustar();
    header.set_entry_type(kind);
    header.set_mode(0o644);
    header.set_size(content.len() as u64);
    header.set_cksum();
    builder.append_data(&mut header, path, content).unwrap();
}

fn git_archive() -> tar::Builder<Vec<u8>> {
    let mut builder = tar::Builder::new(Vec::new());
    append(
        &mut builder,
        tar::EntryType::XGlobalHeader,
        "pax_global_header",
        b"52 comment=0123456789abcdef0123456789abcdef01234567\n",
    );
    builder
}

#[test]
fn github_tar_gzip_metadata_preserves_selected_package_resources() {
    let mut builder = git_archive();
    for (path, content) in [
        ("repository-main/README.md", b"Repository".as_slice()),
        (
            "repository-main/skills/github-example/SKILL.md",
            b"---\nname: github-example\ndescription: Review source evidence.\n---\nRead assets/data.bin.\n".as_slice(),
        ),
        ("repository-main/skills/github-example/assets/data.bin", &[0, 255, 3]),
        ("repository-main/skills/github-example/dist/main.js", b"export default 1;".as_slice()),
    ] {
        append(&mut builder, tar::EntryType::Regular, path, content);
    }
    let tar = builder.into_inner().unwrap();
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar).unwrap();
    let import = SkillImport::from_archive("repository.tar.gz", &gzip.finish().unwrap())
        .unwrap()
        .with_subpath("skills/github-example")
        .unwrap();
    assert_eq!(import.files().count(), 3);
    assert!(
        import
            .files()
            .all(|file| !file.path.contains("pax_global_header"))
    );

    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let manager = SkillManager::new(workspace.clone(), temporary.path().join("global/skills"));
    let preview = manager
        .preview_install(&import, SkillScope::Workspace)
        .unwrap();
    assert_eq!(preview.candidates.len(), 1);
    assert!(preview.candidates[0].valid);
    let installed = manager
        .install(&import, SkillScope::Workspace, &[], false, None)
        .unwrap();
    assert_eq!(installed.installed[0].name, "github-example");
    let destination = workspace.join(".agena/skills/github-example");
    assert_eq!(
        std::fs::read(destination.join("assets/data.bin")).unwrap(),
        [0, 255, 3]
    );
    assert_eq!(
        std::fs::read_to_string(destination.join("dist/main.js")).unwrap(),
        "export default 1;"
    );
    assert!(!destination.join("pax_global_header").exists());
}

#[test]
fn accepting_git_metadata_still_rejects_links_and_pax_path_traversal() {
    for kind in [tar::EntryType::Symlink, tar::EntryType::Link] {
        let mut builder = git_archive();
        let mut header = tar::Header::new_ustar();
        header.set_entry_type(kind);
        header.set_mode(0o777);
        header.set_size(0);
        header.set_link_name("/outside").unwrap();
        header.set_cksum();
        builder
            .append_data(&mut header, "repository-main/link", std::io::empty())
            .unwrap();
        assert!(
            SkillImport::from_archive("repository.tar", &builder.into_inner().unwrap()).is_err()
        );
    }

    let mut builder = git_archive();
    append(
        &mut builder,
        tar::EntryType::XHeader,
        "pax_local_header",
        b"19 path=../outside\n",
    );
    append(
        &mut builder,
        tar::EntryType::Regular,
        "repository-main/safe.txt",
        b"Payload",
    );
    assert!(SkillImport::from_archive("repository.tar", &builder.into_inner().unwrap()).is_err());
}

#[test]
fn global_pax_metadata_remains_bounded() {
    let mut builder = tar::Builder::new(Vec::new());
    let oversized = vec![b'x'; 1024 * 1024 + 1];
    append(
        &mut builder,
        tar::EntryType::XGlobalHeader,
        "pax_global_header",
        &oversized,
    );
    let error =
        SkillImport::from_archive("repository.tar", &builder.into_inner().unwrap()).unwrap_err();
    assert_eq!(error.kind, SkillErrorKind::TooLarge);
}
