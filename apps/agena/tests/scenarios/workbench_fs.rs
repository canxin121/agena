//! Real workbench filesystem HTTP routes, distinct from the builtin fs plugin.
//! Exercise API queries, persisted bytes, streaming chunks, and rejection paths.

use super::fixture;
use agena_testkit::TestResult;
use anyhow::{Context, ensure};
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fs_workbench_real_http_creates_searches_replaces_renames_and_deletes_files() -> TestResult
{
    fixture()
        .run(|case| async move {
            let fs = case.fs_workbench();
            let created: Value = fs
                .post("mkdir", &json!({"path":"docs"}))
                .await?
                .require_success()?
                .json()?;
            ensure!(
                created["success"] == true && fs.directory().join("docs").is_dir(),
                "{created}"
            );

            let contents = "one needle\ntwo ❄️ needle\n";
            let written: Value = fs
                .post(
                    "write",
                    &json!({"path":"docs/notes.txt","content":contents}),
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(written["success"] == true);
            let file = fs.directory().join("docs/notes.txt");
            ensure!(tokio::fs::read_to_string(&file).await? == contents);

            let path = file.to_str().context("fixture file path UTF-8")?;
            let read = fs
                .get_with_query("read", &[("path", path)])
                .await?
                .require_success()?;
            ensure!(read.text()? == contents);
            let listing: Value = fs
                .get_with_query("list", &[("path", fs.directory().to_str().unwrap())])
                .await?
                .require_success()?
                .json()?;
            ensure!(
                listing["entries"].as_array().is_some_and(|entries| entries
                    .iter()
                    .any(|e| e["name"] == "docs" && e["isDirectory"] == true)),
                "{listing}"
            );

            // The chunk endpoint must advance on complete UTF-8 characters even
            // when the requested byte limit cuts through a multibyte code point.
            let mut offset = 0;
            let mut assembled = String::new();
            for _ in 0..contents.len() {
                let offset_str = offset.to_string();
                let chunk: Value = fs
                    .get_with_query(
                        "read-chunk",
                        &[("path", path), ("offset", &offset_str), ("limit", "5")],
                    )
                    .await?
                    .require_success()?
                    .json()?;
                assembled.push_str(chunk["content"].as_str().context("chunk content")?);
                if chunk["hasMore"] == false {
                    break;
                }
                let next = chunk["nextOffset"]
                    .as_u64()
                    .context("nextOffset on nonfinal chunk")? as usize;
                ensure!(next > offset, "nonprogressing read chunk: {chunk}");
                offset = next;
            }
            ensure!(
                assembled == contents,
                "UTF-8 chunk reassembly changed the file: {assembled:?}"
            );

            let search: Value = fs
                .post(
                    "search-content",
                    &json!({"query":"needle", "paths":["docs/notes.txt"]}),
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(
                search["matchCount"] == 2 && search["fileCount"] == 1,
                "{search}"
            );

            let replaced: Value = fs
                .post(
                    "replace-content",
                    &json!({
                        "query":"needle", "replace":"changed", "paths":["docs/notes.txt"]
                    }),
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(
                replaced.to_string().contains("2"),
                "replace did not count two changes: {replaced}"
            );
            ensure!(tokio::fs::read_to_string(&file).await? == "one changed\ntwo ❄️ changed\n");

            let renamed: Value = fs
                .post(
                    "rename",
                    &json!({"oldPath":"docs/notes.txt", "newPath":"docs/renamed.txt"}),
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(renamed["success"] == true);
            ensure!(!file.exists());
            let renamed_file = fs.directory().join("docs/renamed.txt");
            ensure!(renamed_file.is_file());

            let deleted: Value = fs
                .post("delete", &json!({"path":"docs/renamed.txt"}))
                .await?
                .require_success()?
                .json()?;
            ensure!(deleted["success"] == true && !renamed_file.exists());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fs_workbench_denies_invalid_mutations_without_touching_file_contents() -> TestResult {
    fixture()
        .run(|case| async move {
            let fs = case.fs_workbench();
            case.seed_file("workspace/protected.txt", b"unchanged")
                .await?;
            let target = case.workspace().join("protected.txt");
            let invalid = fs
                .post(
                    "write",
                    &json!({"path":"../outside.txt","content":"danger"}),
                )
                .await?;
            ensure!(
                invalid.status.is_client_error(),
                "traversal unexpectedly succeeded: {invalid:?}"
            );
            ensure!(!case.paths().root.join("outside.txt").exists());
            let missing_content = fs.post("write", &json!({"path":"protected.txt"})).await?;
            ensure!(missing_content.status.is_client_error());
            let missing_from = fs.post("rename", &json!({"newPath":"renamed.txt"})).await?;
            ensure!(missing_from.status.is_client_error());
            ensure!(tokio::fs::read(&target).await? == b"unchanged");
            let unauthorized = case
                .http()
                .await
                .anonymous()
                .get("/api/v1/workbench/fs/home")
                .await?;
            ensure!(unauthorized.status.as_u16() == 401);
            Ok(())
        })
        .await
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fs_workbench_symlink_targets_cannot_escape_the_selected_directory() -> TestResult {
    use std::os::unix::fs::symlink;

    fixture()
        .run(|case| async move {
            let fs = case.fs_workbench();
            let sibling = case.paths().root.join("sibling-workspace");
            tokio::fs::create_dir_all(&sibling).await?;
            symlink(&sibling, case.workspace().join("outside-link"))?;
            ensure!(fs.clone().in_directory("workspace/outside-link").is_err());

            for (operation, body) in [
                (
                    "write",
                    json!({"path":"outside-link/escape.txt", "content":"forbidden"}),
                ),
                ("mkdir", json!({"path":"outside-link/subdir"})),
                (
                    "rename",
                    json!({"oldPath":"some.txt", "newPath":"outside-link/renamed.txt"}),
                ),
            ] {
                let denied = fs.post(operation, &body).await?;
                ensure!(
                    denied.status.as_u16() == 400,
                    "{operation} followed symlink: {denied:?}"
                );
            }
            ensure!(!sibling.join("escape.txt").exists() && !sibling.join("subdir").exists());
            ensure!(!sibling.join("renamed.txt").exists());
            Ok(())
        })
        .await
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fs_workbench_preserves_safe_internal_links_and_can_unlink_external_links() -> TestResult {
    use std::os::unix::fs::symlink;

    fixture()
        .run(|case| async move {
            let fs = case.fs_workbench();
            let internal = case.workspace().join("inside");
            tokio::fs::create_dir_all(&internal).await?;
            symlink(&internal, case.workspace().join("inside-link"))?;

            fs.post(
                "write",
                &json!({"path":"inside-link/notes.txt", "content":"linked-write"}),
            )
            .await?
            .require_success()?;
            ensure!(tokio::fs::read(internal.join("notes.txt")).await? == b"linked-write");
            let internal_alias = case.workspace().join("alias-file.txt");
            symlink(internal.join("notes.txt"), &internal_alias)?;
            fs.post(
                "write",
                &json!({"path":"alias-file.txt", "content":"updated-target"}),
            )
            .await?
            .require_success()?;
            ensure!(internal_alias.symlink_metadata()?.file_type().is_symlink());
            ensure!(tokio::fs::read(internal.join("notes.txt")).await? == b"updated-target");

            let sibling = case.paths().root.join("other-workspace");
            tokio::fs::create_dir_all(&sibling).await?;
            case.seed_file("other-workspace/preserve.txt", b"untouched")
                .await?;
            let external = case.workspace().join("outside-alias");
            symlink(&sibling, &external)?;
            fs.post("delete", &json!({"path":"outside-alias"}))
                .await?
                .require_success()?;
            ensure!(!external.exists() && external.symlink_metadata().is_err());
            ensure!(tokio::fs::read(sibling.join("preserve.txt")).await? == b"untouched");

            let moving = case.workspace().join("external-to-move");
            symlink(&sibling, &moving)?;
            fs.post(
                "rename",
                &json!({"oldPath":"external-to-move", "newPath":"moved-alias"}),
            )
            .await?
            .require_success()?;
            ensure!(moving.symlink_metadata().is_err());
            let moved = case.workspace().join("moved-alias");
            ensure!(moved.symlink_metadata()?.file_type().is_symlink());
            ensure!(tokio::fs::read(sibling.join("preserve.txt")).await? == b"untouched");
            Ok(())
        })
        .await
}
