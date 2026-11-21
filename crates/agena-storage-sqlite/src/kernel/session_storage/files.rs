use std::{
    collections::BTreeMap,
    io,
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

use agena_storage::kernel::{
    SessionFileKind, SessionStorageFile, SharedStorage, StorageProtection,
};
use agena_storage_layout::StorageLayout;
use cap_fs_ext::DirExt;
use cap_std::fs::{Dir, Metadata};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PARKED_CACHE_PREFIX: &str = "web-cache.cleaning-";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FileStamp {
    bytes: u64,
    modified_ns: Option<u128>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed_at: (i64, i64),
}

fn stamp(metadata: &Metadata) -> FileStamp {
    #[cfg(unix)]
    use cap_std::fs::MetadataExt;
    FileStamp {
        bytes: metadata.len(),
        modified_ns: metadata
            .modified()
            .ok()
            .and_then(|time| time.into_std().duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos()),
        #[cfg(unix)]
        device: metadata.dev(),
        #[cfg(unix)]
        inode: metadata.ino(),
        #[cfg(unix)]
        changed_at: (metadata.ctime(), metadata.ctime_nsec()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct File {
    pub path: PathBuf,
    pub kind: SessionFileKind,
    pub stamp: FileStamp,
}

impl File {
    pub fn bytes(&self) -> u64 {
        self.stamp.bytes
    }

    pub fn id(&self) -> String {
        hex::encode(Sha256::digest(self.path.as_os_str().as_encoded_bytes()))
    }

    pub fn name(&self) -> &str {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
    }

    pub fn resource(&self, protection: Option<StorageProtection>) -> SessionStorageFile {
        SessionStorageFile {
            id: self.id(),
            name: self.name().to_owned(),
            kind: self.kind,
            bytes: self.bytes(),
            protection,
        }
    }
}

fn open_root(layout: &StorageLayout) -> io::Result<Option<Dir>> {
    let path = agena_storage_layout::resolve_private_path(layout.root())?;
    match Dir::open_ambient_dir(path, cap_std::ambient_authority()) {
        Ok(directory) => Ok(Some(directory)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn directory(root: &Dir, path: &Path) -> io::Result<Dir> {
    let mut current = root.try_clone()?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid managed storage path",
            ));
        };
        current = current.open_dir_nofollow(name)?;
    }
    Ok(current)
}

fn absent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

fn visit(root: &Dir, relative: &Path, mut found: impl FnMut(PathBuf, Metadata)) -> io::Result<()> {
    let dir = match directory(root, relative) {
        Ok(dir) => dir,
        Err(error) if absent(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut pending = vec![(dir, relative.to_path_buf())];
    while let Some((dir, path)) = pending.pop() {
        for entry in dir.entries()? {
            let entry = entry?;
            let name = entry.file_name();
            if name.to_str().is_none() {
                continue;
            }
            let child = path.join(&name);
            let metadata = match dir.symlink_metadata(&name) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if metadata.is_file() {
                found(child, metadata);
            } else if metadata.is_dir() {
                match dir.open_dir_nofollow(&name) {
                    Ok(nested) => pending.push((nested, child)),
                    Err(error) if absent(&error) => {}
                    Err(error) => return Err(error),
                }
            }
        }
    }
    Ok(())
}

fn child_directories(root: &Dir, path: &Path) -> io::Result<Vec<PathBuf>> {
    let directory = match directory(root, path) {
        Ok(directory) => directory,
        Err(error) if absent(&error) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut children = Vec::new();
    for entry in directory.entries()? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            children.push(path.join(entry.file_name()));
        }
    }
    Ok(children)
}

fn collect(
    root: &Dir,
    path: &Path,
    kind: SessionFileKind,
    files: &mut Vec<File>,
) -> io::Result<()> {
    visit(root, path, |path, metadata| {
        files.push(File {
            path,
            kind,
            stamp: stamp(&metadata),
        })
    })
}

fn parked_cache_name(name: &str) -> bool {
    name.strip_prefix(PARKED_CACHE_PREFIX).is_some_and(|token| {
        token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn size(root: &Dir, path: &Path) -> io::Result<u64> {
    let mut bytes = 0u64;
    visit(root, path, |_, metadata| {
        bytes = bytes.saturating_add(metadata.len());
    })?;
    Ok(bytes)
}

pub(super) struct Scanner {
    layout: StorageLayout,
    root: Dir,
    workspaces: Vec<PathBuf>,
    generated: BTreeMap<i64, Vec<PathBuf>>,
}

impl Scanner {
    pub fn open(layout: StorageLayout) -> io::Result<Option<Self>> {
        let Some(root) = open_root(&layout)? else {
            return Ok(None);
        };
        let workspaces = child_directories(&root, Path::new("workspaces"))?;
        let mut generated = BTreeMap::<i64, Vec<PathBuf>>::new();
        for workspace in &workspaces {
            for path in child_directories(&root, &workspace.join("generated_images"))? {
                if let Some(name) = path.file_name().and_then(|value| value.to_str())
                    && let Ok(id) = name.parse::<i64>()
                    && id > 0
                    && id.to_string() == name
                {
                    generated.entry(id).or_default().push(path);
                }
            }
        }
        Ok(Some(Self {
            layout,
            root,
            workspaces,
            generated,
        }))
    }

    pub fn session(&self, session: i64) -> io::Result<Vec<File>> {
        let layout = &self.layout;
        let locations = [
            (layout.session_uploads_dir(session), SessionFileKind::Upload),
            (
                layout.session_artifacts_dir(session),
                SessionFileKind::Artifact,
            ),
            (
                layout.session_web_cache_dir(session),
                SessionFileKind::Cache,
            ),
            (
                layout
                    .plugin_storage_dir()
                    .join("session")
                    .join(session.to_string()),
                SessionFileKind::Plugin,
            ),
        ];
        let mut files = Vec::new();
        for (path, kind) in locations {
            let path = path.strip_prefix(layout.root()).map_err(io::Error::other)?;
            collect(&self.root, path, kind, &mut files)?;
        }
        for path in self.generated.get(&session).into_iter().flatten() {
            collect(&self.root, path, SessionFileKind::Artifact, &mut files)?;
        }
        let assets = layout.session_assets_dir(session);
        let relative = assets
            .strip_prefix(layout.root())
            .map_err(io::Error::other)?;
        for path in child_directories(&self.root, relative)? {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(parked_cache_name)
            {
                collect(&self.root, &path, SessionFileKind::Cache, &mut files)?;
            }
        }
        files.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    pub fn shared(&self) -> io::Result<SharedStorage> {
        let root = &self.root;
        let mut usage = SharedStorage {
            memory: size(root, Path::new("cache/memory"))?,
            logs: size(root, Path::new("logs"))?
                .saturating_add(size(root, Path::new("workspace-preview/logs"))?),
            plugins: size(root, Path::new("plugin-data/workspace"))?
                .saturating_add(size(root, Path::new("plugin-data/global"))?),
            ..Default::default()
        };
        for workspace in &self.workspaces {
            for path in ["uploads", "artifacts"] {
                usage.workspace_files = usage
                    .workspace_files
                    .saturating_add(size(root, &workspace.join(path))?);
            }
            usage.memory = usage
                .memory
                .saturating_add(size(root, &workspace.join("memory"))?);
        }
        for project in child_directories(root, Path::new("projects"))? {
            usage.web = usage.web.saturating_add(size(root, &project.join("web"))?);
        }
        Ok(usage)
    }

    pub fn channel(&self, binding: &str) -> io::Result<u64> {
        if uuid::Uuid::parse_str(binding).is_err() {
            return Ok(0);
        }
        Ok(
            size(&self.root, &Path::new("channel-scopes").join(binding))?.saturating_add(size(
                &self.root,
                &Path::new("channel-speakers").join(binding),
            )?),
        )
    }
}

pub(super) fn park_cache(
    layout: &StorageLayout,
    session: i64,
    token: &str,
    expected: &[File],
) -> io::Result<Option<Vec<File>>> {
    let Some(root) = open_root(layout)? else {
        return Ok(None);
    };
    let cache = layout.session_web_cache_dir(session);
    let live = cache
        .strip_prefix(layout.root())
        .map_err(io::Error::other)?;
    let parent = live
        .parent()
        .ok_or_else(|| io::Error::other("cache parent is absent"))?;
    let dir = match directory(&root, parent) {
        Ok(dir) => dir,
        Err(error) if absent(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    let name = format!("{PARKED_CACHE_PREFIX}{token}");
    let parked = parent.join(&name);
    let already_parked = match dir.symlink_metadata(&name) {
        Ok(metadata) if metadata.is_dir() => true,
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };

    let current_root = if already_parked { &parked } else { live };
    let mut current = Vec::new();
    collect(&root, current_root, SessionFileKind::Cache, &mut current)?;
    current.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    if expected.len() != current.len()
        || expected.iter().zip(&current).any(|(old, new)| {
            old.stamp != new.stamp
                || old.path.strip_prefix(live).ok() != new.path.strip_prefix(current_root).ok()
        })
    {
        return Ok(None);
    }
    // Move an intact cache out of the producer's path before removing any files.
    // A resumed job can find the parked copy even after an interrupted commit.
    if !already_parked {
        dir.rename("web-cache", &dir, &name)?;
        for file in &mut current {
            file.path = parked.join(file.path.strip_prefix(live).map_err(io::Error::other)?);
        }
    }
    Ok(Some(current))
}

fn remove_empty_cache_directories(root: &Dir, file: &File) {
    let Some(parent) = file.path.parent() else {
        return;
    };
    for path in parent
        .ancestors()
        .take_while(|path| path.components().count() >= 3)
    {
        let is_parked = path
            .components()
            .nth(2)
            .and_then(|part| part.as_os_str().to_str())
            .is_some_and(parked_cache_name);
        if !is_parked {
            break;
        }
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            break;
        };
        if directory(root, parent)
            .and_then(|dir| dir.remove_dir(name))
            .is_err()
        {
            break;
        }
    }
}

pub(super) enum Removal {
    Removed,
    Gone,
    Changed,
}

pub(super) fn unlink(layout: &StorageLayout, file: &File) -> io::Result<Removal> {
    let Some(root) = open_root(layout)? else {
        return Ok(Removal::Gone);
    };
    let parent = file
        .path
        .parent()
        .ok_or_else(|| io::Error::other("file parent is absent"))?;
    let name = file
        .path
        .file_name()
        .ok_or_else(|| io::Error::other("file name is absent"))?;
    let dir = match directory(&root, parent) {
        Ok(dir) => dir,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Removal::Gone),
        Err(error) => return Err(error),
    };
    let metadata = match dir.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Removal::Gone),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || stamp(&metadata) != file.stamp {
        return Ok(Removal::Changed);
    }
    match dir.remove_file(name) {
        Ok(()) => {
            if file.kind == SessionFileKind::Cache {
                remove_empty_cache_directories(&root, file);
            }
            Ok(Removal::Removed)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Removal::Gone),
        Err(error) => Err(error),
    }
}
