use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

use agena_api::skills::{SkillInstallPreview, SkillInstallSource, SkillScope, SkillUploadFile};
use agena_tui_components::ConfirmDialogState;
use anyhow::{Context, bail};
use base64::Engine as _;
use crossterm::event::{KeyCode, KeyEvent};

use super::input::SkillMutation;
use super::state::{SkillConfirmAction, SkillEditAction, SkillInstallation, scope_label};
use super::{App, SkillsWorkbench};

const MAX_LOCAL_UPLOAD_BYTES: usize = 32 * 1024 * 1024;
const MAX_LOCAL_UPLOAD_FILES: usize = 2_000;

enum PreviewSource {
    Input(String),
    Prepared(Arc<SkillInstallSource>),
}

impl App {
    pub(super) fn preview_skill_install_input(
        &mut self,
        dialog: &mut SkillsWorkbench,
        input: String,
    ) {
        if input.trim().is_empty() {
            dialog.error = Some("Enter an installation source.".to_owned());
            return;
        }
        let scope = dialog.target_scope;
        self.request_skill_install_preview(dialog, scope, PreviewSource::Input(input), None);
    }

    pub(super) fn preview_skill_copy(&mut self, dialog: &mut SkillsWorkbench) {
        let Some(detail) = dialog.detail.clone() else {
            dialog.error = Some("Select a skill and wait for its details to load.".to_owned());
            return;
        };
        let scope = dialog.target_scope;
        if detail.summary.path.starts_with("plugin://") {
            dialog.open_editor(
                "Copy plugin instructions",
                &format!("Create an editable SKILL.md in {}. Choose a valid, unique name. Supporting plugin resources are not copied.", scope_label(scope)),
                true,
                detail.document,
                SkillEditAction::CreateDocument(scope),
            );
            return;
        }
        let Some((directory, _)) = detail.summary.path.rsplit_once(['/', '\\']) else {
            dialog.error = Some("This skill has no installation directory.".to_owned());
            return;
        };
        let source = SkillInstallSource::Folder {
            path: if directory.is_empty() {
                "/".to_owned()
            } else {
                directory.to_owned()
            },
        };
        self.request_skill_install_preview(
            dialog,
            scope,
            PreviewSource::Prepared(Arc::new(source)),
            Some(detail.summary.name),
        );
    }

    pub(super) fn preview_skill_source_update(&mut self, dialog: &mut SkillsWorkbench) {
        let Some(detail) = dialog.detail.as_ref() else {
            return;
        };
        if !detail.summary.editable {
            dialog.error =
                Some("Install a managed copy before updating this discovered skill.".to_owned());
            return;
        }
        let Some(provenance) = &detail.summary.source else {
            dialog.error = Some(
                "This skill has no recorded installation source. Use Install to choose one."
                    .to_owned(),
            );
            return;
        };
        let source = match provenance.kind.as_str() {
            "github" => SkillInstallSource::Github {
                url: provenance.location.clone(),
                reference: provenance.reference.clone(),
                subpath: provenance.subpath.clone(),
            },
            "folder" => SkillInstallSource::Folder {
                path: provenance.location.clone(),
            },
            "archive" => match source_from_location(&provenance.location) {
                Ok(source) => source,
                Err(error) => {
                    dialog.error = Some(error.to_string());
                    return;
                }
            },
            _ => {
                dialog.error = Some(
                    "This skill was uploaded. Use Install to upload a new version.".to_owned(),
                );
                return;
            }
        };
        let scope = detail.summary.scope;
        let name = detail.summary.name.clone();
        self.request_skill_install_preview(
            dialog,
            scope,
            PreviewSource::Prepared(Arc::new(source)),
            Some(name),
        );
    }

    fn request_skill_install_preview(
        &mut self,
        dialog: &mut SkillsWorkbench,
        scope: SkillScope,
        source: PreviewSource,
        selected_name: Option<String>,
    ) {
        let instance = dialog.instance;
        let workspace_id = dialog.workspace_id;
        let request = dialog.begin_operation();
        self.dispatch_backend_operation(
            move |application| async move {
                let source = match source {
                    PreviewSource::Input(input) => Arc::new(source_from_input(&input).await?),
                    PreviewSource::Prepared(source) => source,
                };
                let preview = application
                    .client()
                    .preview_skill_install(workspace_id, scope, &source)
                    .await?;
                Ok::<_, anyhow::Error>((source, preview))
            },
            move |app, result| {
                app.with_skills_workbench(instance, |_, dialog| {
                    if dialog.operation_request != request {
                        return;
                    }
                    dialog.busy = false;
                    match result {
                        Ok((source, preview)) if !preview.candidates.is_empty() => {
                            let selected = initial_selection(&preview, selected_name.as_deref());
                            dialog.editor = None;
                            dialog.error = None;
                            dialog.installation = Some(SkillInstallation {
                                scope,
                                source,
                                preview,
                                selected,
                                cursor: 0,
                                replace: false,
                            });
                        }
                        Ok(_) => dialog.error = Some(
                            "No skill directories containing SKILL.md were found in this source."
                                .to_owned(),
                        ),
                        Err(error) => dialog.error = Some(error.to_string()),
                    }
                });
            },
        );
    }

    pub(super) fn handle_skill_installation_key(
        &mut self,
        dialog: &mut SkillsWorkbench,
        key: KeyEvent,
    ) {
        let Some(installation) = dialog.installation.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                dialog.installation = None;
                dialog.error = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                installation.cursor = installation.cursor.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                installation.cursor = installation
                    .cursor
                    .saturating_add(1)
                    .min(installation.preview.candidates.len().saturating_sub(1))
            }
            KeyCode::Home => installation.cursor = 0,
            KeyCode::End => {
                installation.cursor = installation.preview.candidates.len().saturating_sub(1)
            }
            KeyCode::Char(' ') => {
                if let Some(candidate) = installation.preview.candidates.get(installation.cursor) {
                    if !candidate.valid {
                        dialog.error = Some(
                            "This candidate has validation errors and cannot be installed."
                                .to_owned(),
                        );
                    } else if !installation.selected.remove(&candidate.path) {
                        installation.selected.insert(candidate.path.clone());
                    }
                }
            }
            KeyCode::Char('a') => {
                let all: BTreeSet<String> = installation
                    .preview
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.valid)
                    .map(|candidate| candidate.path.clone())
                    .collect();
                if installation.selected == all {
                    installation.selected.clear();
                } else {
                    installation.selected = all;
                }
            }
            KeyCode::Char('o') => {
                installation.replace = !installation.replace;
                dialog.error = None;
            }
            KeyCode::Char('r') => {
                let scope = installation.scope;
                let source = installation.source.clone();
                self.request_skill_install_preview(
                    dialog,
                    scope,
                    PreviewSource::Prepared(source),
                    None,
                );
            }
            KeyCode::Enter => {
                if installation.selected.is_empty() {
                    dialog.error = Some("Select at least one valid skill with Space.".to_owned());
                    return;
                }
                let conflicts = installation
                    .preview
                    .candidates
                    .iter()
                    .filter(|candidate| {
                        candidate.conflict && installation.selected.contains(&candidate.path)
                    })
                    .map(|candidate| candidate.name.clone())
                    .collect::<Vec<_>>();
                if !conflicts.is_empty() && !installation.replace {
                    dialog.error = Some("Selected skills already exist. Press O to allow replacement, or deselect them.".to_owned());
                    return;
                }
                let mut lines = vec![format!(
                    "Install {} selected skill(s) in {}?",
                    installation.selected.len(),
                    scope_label(installation.scope)
                )];
                if !conflicts.is_empty() {
                    lines.push(format!("Replace existing skills: {}", conflicts.join(", ")));
                    lines.push(
                        "Local edits in those installed directories will be replaced.".to_owned(),
                    );
                }
                dialog.confirm = Some(ConfirmDialogState::new(
                    "Install selected skills?".to_owned(),
                    lines,
                    "Enter / Y install · Esc cancel".to_owned(),
                    SkillConfirmAction::Install,
                ));
            }
            _ => {}
        }
    }

    pub(super) fn install_selected_skills(&mut self, dialog: &mut SkillsWorkbench) {
        let Some(installation) = &dialog.installation else {
            return;
        };
        let mutation = SkillMutation::Install {
            scope: installation.scope,
            source: installation.source.clone(),
            selected: installation.selected.iter().cloned().collect(),
            replace: installation.replace,
            source_revision: installation.preview.revision.clone(),
        };
        self.submit_skill_mutation(dialog, mutation);
    }
}

fn initial_selection(
    preview: &SkillInstallPreview,
    selected_name: Option<&str>,
) -> BTreeSet<String> {
    preview
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.valid
                && match selected_name {
                    Some(name) => candidate.name == name,
                    None => !candidate.conflict,
                }
        })
        .map(|candidate| candidate.path.clone())
        .collect()
}

async fn source_from_input(input: &str) -> anyhow::Result<SkillInstallSource> {
    let input = input.trim();
    if let Some(path) = input.strip_prefix('@') {
        let path = if let Some(relative) = path.strip_prefix("~/") {
            PathBuf::from(std::env::var_os("HOME").context("Cannot expand ~: HOME is not set")?)
                .join(relative)
        } else {
            PathBuf::from(path)
        };
        return tokio::task::spawn_blocking(move || upload_local_source(&path))
            .await
            .context("Local source reader stopped")?;
    }
    source_from_location(input)
}

fn source_from_location(location: &str) -> anyhow::Result<SkillInstallSource> {
    if location.is_empty() {
        bail!("Enter an installation source");
    }
    if location.starts_with("https://") || location.starts_with("http://") {
        let url = url::Url::parse(location).context("Invalid source URL")?;
        if !is_archive_name(url.path())
            && matches!(url.host_str(), Some("github.com" | "www.github.com"))
        {
            return Ok(SkillInstallSource::Github {
                url: location.to_owned(),
                reference: None,
                subpath: None,
            });
        }
        return Ok(SkillInstallSource::Archive {
            path: None,
            url: Some(location.to_owned()),
            filename: None,
            data_base64: None,
        });
    }
    if is_archive_name(location) {
        Ok(SkillInstallSource::Archive {
            path: Some(location.to_owned()),
            url: None,
            filename: None,
            data_base64: None,
        })
    } else {
        Ok(SkillInstallSource::Folder {
            path: location.to_owned(),
        })
    }
}

fn is_archive_name(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    [".zip", ".tar", ".tar.gz", ".tgz"]
        .iter()
        .any(|extension| path.ends_with(extension))
}

fn upload_local_source(path: &Path) -> anyhow::Result<SkillInstallSource> {
    let root = path
        .canonicalize()
        .with_context(|| format!("Cannot open {} on this computer", path.display()))?;
    if root.is_file() {
        let filename = root
            .file_name()
            .and_then(|name| name.to_str())
            .context("Archive filename is not valid UTF-8")?;
        if !is_archive_name(filename) {
            bail!("Local file uploads must be .zip, .tar, .tar.gz, or .tgz archives");
        }
        let data = read_upload_file(&root, MAX_LOCAL_UPLOAD_BYTES)?;
        return Ok(SkillInstallSource::Archive {
            path: None,
            url: None,
            filename: Some(filename.to_owned()),
            data_base64: Some(base64::engine::general_purpose::STANDARD.encode(data)),
        });
    }
    if !root.is_dir() {
        bail!("Select a regular archive file or a directory");
    }
    let mut files = Vec::new();
    let mut pending = vec![root.clone()];
    let mut total = 0usize;
    let mut scanned = 0usize;
    while let Some(directory) = pending.pop() {
        let mut entries = std::fs::read_dir(&directory)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            scanned += 1;
            if scanned > 10_000 {
                bail!(
                    "Local upload contains too many directory entries. Select a skill subdirectory, a GitHub URL, or a server path."
                );
            }
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                bail!(
                    "Local folder uploads cannot contain symlinks: {}. Use a directory containing regular skill files.",
                    entry.path().display()
                );
            }
            if kind.is_dir() {
                if matches!(entry.file_name().to_str(), Some(".git" | ".hg" | ".svn")) {
                    continue;
                }
                pending.push(entry.path());
            } else if kind.is_file() {
                if entry.file_name() == ".agena-skill.json" {
                    continue;
                }
                if files.len() >= MAX_LOCAL_UPLOAD_FILES {
                    bail!(
                        "Local upload exceeds {MAX_LOCAL_UPLOAD_FILES} files. Select a skill subdirectory, a GitHub URL, or a server path."
                    );
                }
                let file = entry.path();
                let relative = file
                    .strip_prefix(&root)?
                    .to_str()
                    .context("Resource path is not valid UTF-8")?
                    .replace('\\', "/");
                let content =
                    read_upload_file(&file, MAX_LOCAL_UPLOAD_BYTES.saturating_sub(total))?;
                total += content.len();
                #[cfg(unix)]
                let executable = {
                    use std::os::unix::fs::PermissionsExt;
                    entry.metadata()?.permissions().mode() & 0o111 != 0
                };
                #[cfg(not(unix))]
                let executable = false;
                files.push(SkillUploadFile {
                    path: relative,
                    content_base64: base64::engine::general_purpose::STANDARD.encode(content),
                    executable,
                });
            }
        }
    }
    if files.is_empty() {
        bail!("The selected local folder contains no regular files");
    }
    Ok(SkillInstallSource::Files { files })
}

fn read_upload_file(path: &Path, remaining: usize) -> anyhow::Result<Vec<u8>> {
    let mut content = Vec::new();
    std::fs::File::open(path)?
        .take(remaining as u64 + 1)
        .read_to_end(&mut content)?;
    if content.len() > remaining {
        bail!(
            "Local upload exceeds 32 MiB. Select a skill subdirectory, a GitHub URL, or a server path."
        );
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_input_distinguishes_github_archives_and_server_folders() {
        assert!(matches!(
            source_from_location("https://github.com/owner/repo/tree/main/skills").unwrap(),
            SkillInstallSource::Github { .. }
        ));
        assert!(matches!(
            source_from_location("https://github.com/owner/repo/archive/refs/heads/main.zip")
                .unwrap(),
            SkillInstallSource::Archive { url: Some(_), .. }
        ));
        assert!(matches!(
            source_from_location("/srv/skills.tar.gz").unwrap(),
            SkillInstallSource::Archive { path: Some(_), .. }
        ));
        assert!(matches!(
            source_from_location("/srv/skills").unwrap(),
            SkillInstallSource::Folder { .. }
        ));
    }

    #[test]
    fn local_folder_upload_preserves_relative_paths_and_content() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("references")).unwrap();
        std::fs::write(directory.path().join("SKILL.md"), "instructions").unwrap();
        std::fs::write(directory.path().join("references/guide.md"), "guide\n").unwrap();
        for name in ["dist", "target", "node_modules", ".git"] {
            std::fs::create_dir(directory.path().join(name)).unwrap();
            std::fs::write(directory.path().join(name).join("asset.txt"), "resource").unwrap();
        }
        let SkillInstallSource::Files { files } = upload_local_source(directory.path()).unwrap()
        else {
            panic!("expected uploaded files")
        };
        assert_eq!(files.len(), 5);
        for name in ["dist", "target", "node_modules"] {
            assert!(
                files
                    .iter()
                    .any(|file| file.path == format!("{name}/asset.txt"))
            );
        }
        assert!(!files.iter().any(|file| file.path.starts_with(".git/")));
        let resource = files
            .iter()
            .find(|file| file.path == "references/guide.md")
            .unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(&resource.content_base64)
                .unwrap(),
            b"guide\n"
        );
        assert!(read_upload_file(&directory.path().join("SKILL.md"), 2).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn local_upload_rejects_nested_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), directory.path().join("outside.txt")).unwrap();
        assert!(upload_local_source(directory.path()).is_err());
    }
}
