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
fn generated_media_larger_than_twenty_mib_can_be_read_within_the_writer_limit() {
    let f = Fixture::new();
    let bytes = vec![0x5a; 20 * 1024 * 1024 + 1];
    assert!(bytes.len() <= agena_runtime_tools::MAX_GENERATED_IMAGE_BYTES);
    fs::write(f.generated.join("large-image.png"), &bytes).unwrap();

    let (filename, downloaded) = f.read("large-image.png").unwrap();
    assert_eq!(filename, "large-image.png");
    assert_eq!(downloaded, bytes);
}

#[test]
fn generated_media_over_the_writer_limit_is_rejected_from_metadata() {
    let f = Fixture::new();
    // Sparse file: no giant image allocation is needed to test the metadata
    // guard, which must reject the file before the bounded reader opens it.
    fs::File::create(f.generated.join("oversize-image.png"))
        .unwrap()
        .set_len(agena_runtime_tools::MAX_GENERATED_IMAGE_BYTES as u64 + 1)
        .unwrap();

    let error = f.read("oversize-image.png").unwrap_err();
    assert_eq!(
        error.to_string(),
        "session media exceeds the generated image size limit"
    );
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

#[test]
fn historical_generated_paths_require_the_exact_managed_layout_and_writer_name() {
    let f = Fixture::new();
    let layout = agena_storage_layout::StorageLayout::new(f.base.join("state"));
    let key = "a".repeat(64);
    let root = layout.root();
    for (name, expected_tool) in [
        ("provider-hosted-image-557.png", 557),
        ("provider-hosted-image-557-1.jpeg", 557),
        ("image-tool-42.webp", 42),
        ("image-tool-42-2.gif", 42),
    ] {
        let owner = root
            .join("workspaces")
            .join(&key)
            .join("generated_images/46");
        assert_eq!(
            historical_generated_attachment(root, &owner.join(name)),
            Some((owner, expected_tool)),
        );
    }
    for relative in [
        format!(
            "workspaces/{}/generated_images/46/provider-hosted-image-557.png",
            "a".repeat(63)
        ),
        format!(
            "workspaces/{}/generated_images/46/provider-hosted-image-557.png",
            "A".repeat(64)
        ),
        format!("workspaces/{key}/uploads/46/provider-hosted-image-557.png"),
        format!("workspaces/{key}/generated_images/0/provider-hosted-image-557.png"),
        format!("workspaces/{key}/generated_images/+46/provider-hosted-image-557.png"),
        format!("workspaces/{key}/generated_images/046/provider-hosted-image-557.png"),
        format!("workspaces/{key}/generated_images/46/provider-hosted-image-0.png"),
        format!("workspaces/{key}/generated_images/46/provider-hosted-image-557-private.png"),
        format!("workspaces/{key}/generated_images/46/provider-hosted-image-557-0.png"),
        format!("workspaces/{key}/generated_images/46/provider-hosted-image-557-1-2.png"),
        format!("workspaces/{key}/generated_images/46/provider-hosted-image-557.html"),
        format!("workspaces/{key}/generated_images/46/provider-hosted-image-557.png.exe"),
        format!("workspaces/{key}/generated_images/46/../47/provider-hosted-image-557.png"),
        format!("workspaces/{key}/generated_images/46/subdir/provider-hosted-image-557.png"),
        format!("workspaces/{key}/generated_images/46/./provider-hosted-image-557.png"),
        format!("workspaces/{key}/generated_images/46//provider-hosted-image-557.png"),
        format!("workspaces/{key}/generated_images/46/provider-hosted-image-557.png/"),
    ] {
        assert!(
            historical_generated_attachment(root, &root.join(&relative)).is_none(),
            "{relative}"
        );
    }
    let sibling = PathBuf::from(format!("{}-other", root.display()))
        .join("workspaces")
        .join(&key)
        .join("generated_images/46/provider-hosted-image-557.png");
    assert!(historical_generated_attachment(root, &sibling).is_none());
}

#[tokio::test]
async fn historical_workspace_images_require_visible_exact_attachments_and_matching_bytes() {
    use agena_domain::{ComposerDocument, ComposerNode};
    use agena_storage::kernel::{ExecutionPhase, RequestId, ToolPhase, ToolResult, ToolStart};
    use sha2::Digest as _;

    let f = Fixture::new();
    let layout = agena_storage_layout::StorageLayout::new(f.base.join("state"));
    let kernel = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
        .await
        .unwrap();
    let workspace_path = f.workspace.to_string_lossy();
    let workspace = kernel
        .workspaces
        .ensure_id(workspace_path.as_ref())
        .await
        .unwrap();
    let session = kernel
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "current workspace".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let foreign = kernel
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "unrelated session".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let execution = kernel
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Create an image".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = kernel
        .ports
        .executor
        .claim_next(RequestId::new(), session, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_provider_tool(
            RequestId::new(),
            ToolStart {
                call_id: "historical-image".into(),
                name: "image_generation".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;

    // The source worktree need not still exist. Its managed artifacts remain
    // under the historical hash, while the session now belongs elsewhere.
    let old_root = layout
        .workspace_dir(&f.base.join("removed-worktree"))
        .join("generated_images")
        .join(session.get().to_string());
    let current_root = layout
        .workspace_dir(&f.workspace)
        .join("generated_images")
        .join(session.get().to_string());
    assert_ne!(old_root, current_root);
    assert!(!f.base.join("removed-worktree").exists());
    fs::create_dir_all(&old_root).unwrap();
    let path = old_root.join(format!("provider-hosted-image-{}.png", tool.id().get()));
    let bytes = b"recorded image bytes";
    fs::write(&path, bytes).unwrap();
    let hash: String = sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let attachment: agena_domain::AttachmentItem = serde_json::from_value(serde_json::json!({
        "kind":"image", "mime":"image/png",
        "source":{"source":"local_path","path":path.to_string_lossy()},
        "sha256":hash, "size_bytes":bytes.len(),
    }))
    .unwrap();
    let result =
        serde_json::json!({"raw_output":{"attachments":[attachment.clone()],"truncated":false}});
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: result.clone(),
        },
    )
    .await
    .unwrap();
    let reader = &kernel.ports.reader;
    let (owner_root, id) = historical_generated_attachment(layout.root(), &path).unwrap();
    let query = || WorkspaceFileDownloadQuery {
        path: path.to_string_lossy().into_owned(),
    };
    assert_eq!(
        read_authorized_generated_attachment(
            reader,
            session.get(),
            owner_root.clone(),
            id,
            query(),
        )
        .await
        .unwrap()
        .1,
        bytes
    );
    assert!(
        read_authorized_generated_attachment(
            reader,
            foreign.get(),
            owner_root.clone(),
            id,
            query(),
        )
        .await
        .is_err()
    );

    let sibling = old_root.join(format!("provider-hosted-image-{}-1.png", tool.id().get()));
    fs::write(&sibling, bytes).unwrap();
    assert!(
        read_authorized_generated_attachment(
            reader,
            session.get(),
            owner_root.clone(),
            id,
            WorkspaceFileDownloadQuery {
                path: sibling.to_string_lossy().into_owned()
            },
        )
        .await
        .is_err()
    );

    let mut changed = bytes.to_vec();
    changed[0] ^= 1;
    fs::write(&path, &changed).unwrap();
    assert!(
        read_authorized_generated_attachment(
            reader,
            session.get(),
            owner_root.clone(),
            id,
            query(),
        )
        .await
        .is_err()
    );
    fs::write(&path, bytes).unwrap();
    let mut wrong_size = attachment.clone();
    wrong_size.size_bytes = Some(bytes.len() as u64 + 1);
    assert!(read_verified_generated_attachment(owner_root.clone(), query(), &wrong_size).is_err());
    let mut wrong_hash = attachment;
    wrong_hash.sha256 = Some("0".repeat(64));
    assert!(read_verified_generated_attachment(owner_root.clone(), query(), &wrong_hash).is_err());

    let stored = reader.tool(session, tool.id()).await.unwrap();
    assert_eq!(stored.result.as_ref(), Some(&result));
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let child = kernel
        .ports
        .sessions
        .fork(RequestId::new(), session, stored.entry_id)
        .await
        .unwrap()
        .value
        .id;
    assert_eq!(
        read_authorized_generated_attachment(reader, child.get(), owner_root, id, query(),)
            .await
            .unwrap()
            .1,
        bytes
    );
}

#[cfg(unix)]
#[test]
fn historical_generated_image_reads_reject_file_and_owner_directory_symlink_escapes() {
    use sha2::Digest as _;
    use std::os::unix::fs::symlink;

    let f = Fixture::new();
    let layout = agena_storage_layout::StorageLayout::new(f.base.join("state"));
    let owner = layout
        .root()
        .join("workspaces")
        .join("b".repeat(64))
        .join("generated_images/46");
    fs::create_dir_all(&owner).unwrap();
    let outside = f.base.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let name = "provider-hosted-image-557.png";
    let bytes = b"matching recorded bytes";
    fs::write(outside.join(name), bytes).unwrap();
    let path = owner.join(name);
    let hash: String = sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let attachment = serde_json::from_value(serde_json::json!({
        "kind":"image","mime":"image/png",
        "source":{"source":"local_path","path":path.to_string_lossy()},
        "sha256":hash,"size_bytes":bytes.len(),
    }))
    .unwrap();
    let query = || WorkspaceFileDownloadQuery {
        path: path.to_string_lossy().into_owned(),
    };
    assert!(historical_generated_attachment(layout.root(), &path).is_some());
    symlink(outside.join(name), &path).unwrap();
    assert!(read_verified_generated_attachment(owner.clone(), query(), &attachment).is_err());
    fs::remove_file(&path).unwrap();
    fs::remove_dir(&owner).unwrap();
    symlink(&outside, &owner).unwrap();
    assert!(read_verified_generated_attachment(owner, query(), &attachment).is_err());
}

async fn exported_attachment_history(
    ports: &agena_storage::kernel::SessionKernel,
    workspace: i64,
) -> (
    agena_storage::kernel::SessionId,
    agena_storage::kernel::SessionId,
    agena_storage::kernel::AttemptHandle,
    agena_storage::kernel::RoundHandle,
) {
    use agena_domain::{ComposerDocument, ComposerNode};
    use agena_storage::kernel::RequestId;

    let session = ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "explicit output export".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let foreign = ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "unrelated session".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let execution = ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Export the generated image to ai-art".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = ports
        .executor
        .claim_next(RequestId::new(), session, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    (session, foreign, attempt, round)
}

fn exported_image_attachment(path: &Path, bytes: &[u8]) -> agena_domain::AttachmentItem {
    use sha2::Digest as _;

    let hash: String = sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    serde_json::from_value(serde_json::json!({
        "kind":"image", "mime":"image/png", "filename":path.file_name().unwrap().to_string_lossy(),
        "source":{"source":"local_path","path":path.to_string_lossy()},
        "sha256":hash, "size_bytes":bytes.len(),
    }))
    .unwrap()
}

async fn record_exported_attachments(
    round: &agena_storage::kernel::RoundHandle,
    call_id: &str,
    attachments: serde_json::Value,
) -> agena_storage::kernel::ToolHandle {
    use agena_storage::kernel::{RequestId, ToolPhase, ToolResult, ToolStart};

    let output_path = attachments
        .get(0)
        .and_then(|item| item.pointer("/source/path"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let tool = round
        .begin_provider_tool(
            RequestId::new(),
            ToolStart {
                call_id: call_id.into(),
                name: "chatgpt.image_generate".into(),
                input: serde_json::json!({"output_path":output_path}),
            },
        )
        .await
        .unwrap()
        .value;
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: serde_json::json!({"raw_output":{"attachments":attachments,"truncated":false}}),
        },
    )
    .await
    .unwrap();
    tool
}

#[tokio::test]
async fn explicit_tool_exports_require_exact_visible_membership_and_preserve_fork_access() {
    use agena_storage::kernel::{ExecutionPhase, RequestId};

    let f = Fixture::new();
    let kernel = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
        .await
        .unwrap();
    let workspace = kernel
        .workspaces
        .ensure_id(f.workspace.to_string_lossy().as_ref())
        .await
        .unwrap();
    let (session, foreign, attempt, round) =
        exported_attachment_history(&kernel.ports, workspace).await;

    // Like session 48, this is an explicit output_path outside both the
    // selected workspace and the managed generated_images layout.
    let owner = f.base.join("ai-art");
    fs::create_dir_all(&owner).unwrap();
    let path = owner.join("ink-wash-landscape.png");
    assert!(!path.starts_with(&f.workspace));
    assert!(!path.starts_with(&f.generated));
    let bytes = b"exported landscape image bytes";
    fs::write(&path, bytes).unwrap();
    let attachment = exported_image_attachment(&path, bytes);
    let tool = record_exported_attachments(
        &round,
        "explicit-export",
        serde_json::json!([attachment.clone()]),
    )
    .await;
    let reader = &kernel.ports.reader;
    let stored = reader.tool(session, tool.id()).await.unwrap();
    let query = || WorkspaceFileDownloadQuery {
        path: path.to_string_lossy().into_owned(),
    };
    let visible = visible_exported_tool_attachment(reader, session.get(), &query().path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(visible, attachment);
    let (filename, downloaded) =
        read_verified_generated_attachment(owner.clone(), query(), &visible).unwrap();
    assert_eq!(filename, "ink-wash-landscape.png");
    assert_eq!(downloaded, bytes);
    assert!(
        visible_exported_tool_attachment(reader, foreign.get(), &query().path)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        f.read(query().path).is_err(),
        "an outside path has no workspace fallback grant"
    );
    let sibling = owner.join("ink-wash-landscape-copy.png");
    fs::write(&sibling, bytes).unwrap();
    for locator in [
        sibling.to_string_lossy().into_owned(),
        format!("{}/./ink-wash-landscape.png", owner.display()),
    ] {
        assert!(
            visible_exported_tool_attachment(reader, session.get(), &locator)
                .await
                .unwrap()
                .is_none(),
            "attachment identity must match the exact stored path: {locator}",
        );
        assert!(f.read(locator).is_err());
    }

    let mut changed = bytes.to_vec();
    changed[0] ^= 1;
    fs::write(&path, &changed).unwrap();
    assert!(read_verified_generated_attachment(owner.clone(), query(), &visible).is_err());
    let mut enlarged = bytes.to_vec();
    enlarged.push(b'!');
    fs::write(&path, enlarged).unwrap();
    assert!(read_verified_generated_attachment(owner.clone(), query(), &visible).is_err());
    fs::write(&path, bytes).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        // Matching bytes do not authorize following a file or parent symlink
        // outside the exact export directory.
        let outside = f.base.join("outside-image.png");
        fs::write(&outside, bytes).unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&outside, &path).unwrap();
        assert!(read_verified_generated_attachment(owner.clone(), query(), &visible).is_err());
        fs::remove_file(&path).unwrap();
        fs::write(&path, bytes).unwrap();
        let moved = f.base.join("moved-ai-art");
        fs::rename(&owner, &moved).unwrap();
        symlink(&moved, &owner).unwrap();
        assert!(read_verified_generated_attachment(owner.clone(), query(), &visible).is_err());
        fs::remove_file(&owner).unwrap();
        fs::rename(&moved, &owner).unwrap();
    }

    assert_eq!(
        reader
            .tool(session, tool.id())
            .await
            .unwrap()
            .result
            .as_ref(),
        stored.result.as_ref()
    );
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let child = kernel
        .ports
        .sessions
        .fork(RequestId::new(), session, stored.entry_id)
        .await
        .unwrap()
        .value
        .id;
    let inherited = visible_exported_tool_attachment(reader, child.get(), &query().path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(inherited, attachment);
    assert_eq!(
        read_verified_generated_attachment(owner, query(), &inherited)
            .unwrap()
            .1,
        bytes,
    );
}

#[tokio::test]
async fn exported_attachment_lookup_crosses_pages_ignores_unrelated_bad_data_and_uses_latest_identity()
 {
    use agena_storage::kernel::{EntryPayload, ExecutionPhase, RequestId};

    let f = Fixture::new();
    let kernel = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
        .await
        .unwrap();
    let workspace = kernel
        .workspaces
        .ensure_id(f.workspace.to_string_lossy().as_ref())
        .await
        .unwrap();
    let (session, _, attempt, round) = exported_attachment_history(&kernel.ports, workspace).await;
    let owner = f.base.join("ai-art");
    fs::create_dir_all(&owner).unwrap();
    let path = owner.join("landscape.png");
    let original = b"image version one";
    fs::write(&path, original).unwrap();
    let first = exported_image_attachment(&path, original);
    let first_tool =
        record_exported_attachments(&round, "old-export", serde_json::json!([first.clone()])).await;

    // More than the reader's 128-entry page. Unrelated malformed attachment
    // objects must not prevent a valid older attachment from being read.
    for index in 0..130 {
        let unrelated = serde_json::json!([
            null,
            17,
            {"source":{"source":"local_path","path":format!("/unrelated/{index}.png")},"kind":"invalid-kind"},
            {"source":{"source":"url","url":"https://example.invalid/not-fetched"}},
        ]);
        record_exported_attachments(&round, &format!("unrelated-{index}"), unrelated).await;
    }
    let reader = &kernel.ports.reader;
    let newest_page = reader.history(session, None, 128).await.unwrap();
    assert!(newest_page.has_more);
    assert!(!newest_page.entries.iter().any(|entry| {
        matches!(entry.payload, EntryPayload::Tool { tool_id } if tool_id == first_tool.id())
    }));
    let query = || WorkspaceFileDownloadQuery {
        path: path.to_string_lossy().into_owned(),
    };
    assert_eq!(
        visible_exported_tool_attachment(reader, session.get(), &query().path)
            .await
            .unwrap(),
        Some(first),
    );

    let replacement_bytes = b"image version two";
    assert_eq!(replacement_bytes.len(), original.len());
    let replacement = exported_image_attachment(&path, replacement_bytes);
    record_exported_attachments(
        &round,
        "replacement-export",
        serde_json::json!([replacement.clone()]),
    )
    .await;
    let visible = visible_exported_tool_attachment(reader, session.get(), &query().path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(visible, replacement);
    // The bytes still belong to the older record: searching older history
    // after this hash mismatch would silently revive a superseded identity.
    assert!(read_verified_generated_attachment(owner.clone(), query(), &visible).is_err());
    fs::write(&path, replacement_bytes).unwrap();
    assert_eq!(
        read_verified_generated_attachment(owner, query(), &visible)
            .unwrap()
            .1,
        replacement_bytes,
    );

    // A malformed newest match is an explicit failure, not permission to
    // continue to an older, seemingly valid descriptor for the same path.
    record_exported_attachments(
        &round,
        "invalid-newest-export",
        serde_json::json!([{
            "kind":"image", "mime":"image/png",
            "source":{"source":"local_path","path":path.to_string_lossy()},
            "size_bytes":replacement_bytes.len(),
        }]),
    )
    .await;
    assert!(
        visible_exported_tool_attachment(reader, session.get(), &query().path)
            .await
            .is_err()
    );
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
}
