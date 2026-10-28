use super::*;

struct Fixture {
    base: PathBuf,
    workspace: PathBuf,
    generated: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!("agena-attachment-tests-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let workspace = base.join("workspace");
        let generated = base.join("managed/generated_images/43");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&generated).unwrap();
        Self {
            base,
            workspace,
            generated,
        }
    }
    fn read(&self, path: impl Into<String>) -> ApplicationResult<(String, Vec<u8>)> {
        read_attachment_file_sync(
            self.workspace.clone(),
            self.generated.clone(),
            WorkspaceFileDownloadQuery { path: path.into() },
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[test]
fn relative_absolute_and_generated_downloads_keep_bytes_and_names() {
    let f = Fixture::new();
    fs::write(f.workspace.join("向日葵.txt"), b"original").unwrap();
    fs::write(f.generated.join("flower.png"), b"image bytes").unwrap();
    for path in [
        "向日葵.txt".to_owned(),
        f.workspace
            .join("向日葵.txt")
            .to_string_lossy()
            .into_owned(),
    ] {
        let (name, bytes) = f.read(path).unwrap();
        assert_eq!(name, "向日葵.txt");
        assert_eq!(bytes, b"original");
    }
    for path in [
        "flower.png".to_owned(),
        f.generated
            .join("flower.png")
            .to_string_lossy()
            .into_owned(),
    ] {
        assert_eq!(f.read(path).unwrap().1, b"image bytes");
    }
}

#[test]
fn traversal_other_session_and_workspace_prefix_confusion_are_denied() {
    let f = Fixture::new();
    let other = f.base.join("managed/generated_images/44");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("private.png"), b"secret").unwrap();
    fs::write(f.base.join("private.txt"), b"secret").unwrap();
    let lookalike = f.base.join("workspace-other");
    fs::create_dir_all(&lookalike).unwrap();
    fs::write(lookalike.join("private.txt"), b"secret").unwrap();
    for path in [
        "../private.txt".into(),
        other.join("private.png").to_string_lossy().into_owned(),
        lookalike.join("private.txt").to_string_lossy().into_owned(),
    ] {
        assert!(f.read(path).is_err());
    }
}

#[cfg(unix)]
#[test]
fn symlink_escape_never_falls_back_to_a_different_file() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let secret = f.base.join("secret");
    fs::write(&secret, b"secret").unwrap();
    symlink(&secret, f.generated.join("image.png")).unwrap();
    fs::write(f.workspace.join("image.png"), b"different image").unwrap();
    assert!(f.read("image.png").is_err());
    symlink(&secret, f.workspace.join("leak.txt")).unwrap();
    assert!(f.read("leak.txt").is_err());
}

#[test]
fn uploads_are_atomic_private_bounded_unicode_files_with_hashes() {
    let f = Fixture::new();
    let layout = agena_storage_layout::StorageLayout::new(f.base.join("upload-state"));
    for filename in [
        "向日葵 报告.pdf".to_owned(),
        format!("{}.png", "花".repeat(300)),
    ] {
        let bytes = b"exact uploaded bytes".to_vec();
        let result = upload_workspace_file_in(
            1,
            f.workspace.clone(),
            WorkspaceFileUploadRequest {
                filename: filename.clone(),
                data_base64: String::new(),
                mime: None,
            },
            Some(bytes.clone()),
            &layout,
        )
        .unwrap();
        assert_eq!(fs::read(&result.path).unwrap(), bytes);
        assert_eq!(result.name, sanitize_upload_filename(&filename));
        assert!(result.name.len() <= 200);
        assert_eq!(result.sha256.as_ref().unwrap().len(), 64);
        assert!(Path::new(&result.path).starts_with(layout.uploads_dir(&f.workspace)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&result.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

#[test]
fn invalid_empty_uploads_and_directory_names_are_rejected() {
    let f = Fixture::new();
    let layout = agena_storage_layout::StorageLayout::new(f.base.join("upload-state"));
    for (filename, base64) in [
        ("x", "%%%"),
        ("x", ""),
        ("dir/..", "YQ=="),
        ("dir/", "YQ=="),
    ] {
        assert!(
            upload_workspace_file_in(
                1,
                f.workspace.clone(),
                WorkspaceFileUploadRequest {
                    filename: filename.into(),
                    data_base64: base64.into(),
                    mime: None,
                },
                None,
                &layout
            )
            .is_err()
        );
    }
    assert!(!layout.uploads_dir(&f.workspace).exists());
}

#[test]
fn inherited_generated_names_are_only_candidates_not_blanket_directory_grants() {
    let root = Path::new("/state/workspaces/key/generated_images");
    let (owner, id) =
        inherited_generated_attachment(root, &root.join("43/provider-hosted-image-491-2.png"))
            .unwrap();
    assert_eq!(owner, root.join("43"));
    assert_eq!(id, 491);
    for relative in [
        "43/unknown.png",
        "43/../44/provider-hosted-image-491.png",
        "43/subdir/provider-hosted-image-491.png",
        "0/provider-hosted-image-491.png",
        "43/provider-hosted-image-0.png",
    ] {
        assert!(
            inherited_generated_attachment(root, &root.join(relative)).is_none(),
            "{relative}"
        );
    }
    assert!(
        inherited_generated_attachment(
            root,
            Path::new("/another/43/provider-hosted-image-491.png")
        )
        .is_none()
    );
}
