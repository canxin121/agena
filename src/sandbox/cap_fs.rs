use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::Dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathKeyMode {
    Exact,
    AsciiCaseInsensitive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PathPolicy {
    key_mode: PathKeyMode,
}

impl PathPolicy {
    pub(crate) const fn new(key_mode: PathKeyMode) -> Self {
        Self { key_mode }
    }

    pub(crate) const fn exact() -> Self {
        Self::new(PathKeyMode::Exact)
    }

    pub(crate) const fn ascii_case_insensitive() -> Self {
        Self::new(PathKeyMode::AsciiCaseInsensitive)
    }

    pub(crate) fn normalize_paths(self, paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
        dedupe_canonical_paths(paths, self.key_mode)
    }

    pub(crate) fn validate_and_dedupe<E>(
        self,
        paths: impl IntoIterator<Item = PathBuf>,
        mut validate: impl FnMut(&Path) -> Result<PathBuf, E>,
    ) -> Result<Vec<PathBuf>, E> {
        let mut validated = Vec::new();
        for path in paths {
            validated.push(validate(&path)?);
        }
        Ok(dedupe_paths(validated, self.key_mode))
    }
}

pub(crate) fn canonicalize_path(path: &Path) -> io::Result<PathBuf> {
    let absolute = absolute_path(path)?;
    canonicalize_absolute(&absolute)
}

pub(crate) fn canonicalize_or_original(path: &Path) -> PathBuf {
    canonicalize_path(path).unwrap_or_else(|_| path.to_path_buf())
}

pub(crate) fn dedupe_canonical_paths(
    paths: impl IntoIterator<Item = PathBuf>,
    mode: PathKeyMode,
) -> Vec<PathBuf> {
    let canonicalized = paths
        .into_iter()
        .map(|path| canonicalize_or_original(&path))
        .collect::<Vec<_>>();
    dedupe_paths(canonicalized, mode)
}

fn dedupe_paths(paths: impl IntoIterator<Item = PathBuf>, mode: PathKeyMode) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for path in paths {
        let key = path_key(&path, mode);
        if seen.insert(key) {
            out.push(path);
        }
    }
    out
}

pub(crate) fn path_exists(path: &Path) -> bool {
    let Ok(absolute) = absolute_path(path) else {
        return false;
    };

    if let Some((parent, leaf)) = parent_and_leaf(&absolute) {
        let Ok(dir) = Dir::open_ambient_dir(parent, ambient_authority()) else {
            return false;
        };

        return dir.try_exists(leaf).unwrap_or(false);
    }

    Dir::open_ambient_dir(&absolute, ambient_authority()).is_ok()
}

pub(crate) fn is_dir(path: &Path) -> bool {
    let Ok(absolute) = absolute_path(path) else {
        return false;
    };

    if let Some((parent, leaf)) = parent_and_leaf(&absolute) {
        let Ok(dir) = Dir::open_ambient_dir(parent, ambient_authority()) else {
            return false;
        };

        return dir
            .metadata(leaf)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
    }

    Dir::open_ambient_dir(&absolute, ambient_authority()).is_ok()
}

pub(crate) fn is_file(path: &Path) -> bool {
    let Ok(absolute) = absolute_path(path) else {
        return false;
    };

    let Some((parent, leaf)) = parent_and_leaf(&absolute) else {
        return false;
    };

    let Ok(dir) = Dir::open_ambient_dir(parent, ambient_authority()) else {
        return false;
    };

    dir.metadata(leaf)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

pub(crate) fn is_symlink(path: &Path) -> io::Result<bool> {
    let absolute = absolute_path(path)?;

    let Some((parent, leaf)) = parent_and_leaf(&absolute) else {
        return Ok(false);
    };

    let dir = Dir::open_ambient_dir(parent, ambient_authority())?;
    let metadata = dir.symlink_metadata(leaf)?;
    Ok(metadata.file_type().is_symlink())
}

pub(crate) fn child_directories(path: &Path, limit: usize) -> io::Result<Vec<PathBuf>> {
    let absolute = absolute_path(path)?;
    let dir = Dir::open_ambient_dir(&absolute, ambient_authority())?;

    let mut out = Vec::new();
    for entry in dir.entries()?.take(limit) {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        out.push(absolute.join(entry.file_name()));
    }

    Ok(out)
}

fn canonicalize_absolute(absolute: &Path) -> io::Result<PathBuf> {
    if let Some((parent, leaf)) = parent_and_leaf(absolute) {
        let dir = Dir::open_ambient_dir(parent, ambient_authority())?;
        let relative = dir.canonicalize(leaf)?;
        Ok(parent.join(relative))
    } else {
        absolute.canonicalize()
    }
}

fn absolute_path(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn parent_and_leaf(path: &Path) -> Option<(&Path, &OsStr)> {
    Some((path.parent()?, path.file_name()?))
}

fn path_key(path: &Path, mode: PathKeyMode) -> String {
    match mode {
        PathKeyMode::Exact => path.to_string_lossy().to_string(),
        PathKeyMode::AsciiCaseInsensitive => path.to_string_lossy().to_ascii_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        PathKeyMode, PathPolicy, canonicalize_path, child_directories, dedupe_canonical_paths,
        is_dir, is_file, is_symlink, path_exists,
    };

    #[test]
    fn canonicalize_path_matches_std_for_directory() {
        let temp = unique_temp_path("cap-fs-canonicalize");
        fs::create_dir_all(&temp).expect("temp directory should be created");

        let expected = temp.canonicalize().expect("std canonicalize should work");
        let actual = canonicalize_path(&temp).expect("cap canonicalize should work");

        assert_eq!(
            actual
                .canonicalize()
                .expect("cap-derived path should canonicalize"),
            expected
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn path_exists_and_is_dir_reflect_directory_presence() {
        let temp = unique_temp_path("cap-fs-exists");
        fs::create_dir_all(&temp).expect("temp directory should be created");

        assert!(path_exists(&temp));
        assert!(is_dir(&temp));

        let _ = fs::remove_dir_all(&temp);
        assert!(!path_exists(&temp));
        assert!(!is_dir(&temp));
    }

    #[test]
    fn is_symlink_false_for_regular_file() {
        let temp = unique_temp_path("cap-fs-symlink");
        fs::create_dir_all(&temp).expect("temp directory should be created");
        let file = temp.join("plain.txt");
        fs::write(&file, "ok").expect("temp file should be created");

        let value = is_symlink(&file).expect("symlink query should succeed");
        assert!(!value);

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn is_file_true_for_regular_file() {
        let temp = unique_temp_path("cap-fs-file");
        fs::create_dir_all(&temp).expect("temp directory should be created");
        let file = temp.join("plain.txt");
        fs::write(&file, "ok").expect("temp file should be created");

        assert!(is_file(&file));
        assert!(!is_file(&temp));

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn child_directories_returns_only_directories() {
        let temp = unique_temp_path("cap-fs-children");
        fs::create_dir_all(&temp).expect("temp directory should be created");
        fs::create_dir_all(temp.join("a")).expect("dir a should be created");
        fs::create_dir_all(temp.join("b")).expect("dir b should be created");
        fs::write(temp.join("file.txt"), "x").expect("file should be created");

        let children = child_directories(&temp, 10).expect("children query should succeed");
        let mut names = children
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        names.sort();

        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn dedupe_canonical_paths_respects_key_mode() {
        let temp = unique_temp_path("cap-fs-dedupe");
        fs::create_dir_all(&temp).expect("temp directory should be created");

        let sample = temp.join("MiXeD-Case-Entry");
        let upper = std::path::PathBuf::from(sample.to_string_lossy().to_string().to_uppercase());
        let lower = std::path::PathBuf::from(sample.to_string_lossy().to_string().to_lowercase());

        let exact = dedupe_canonical_paths(vec![upper.clone(), lower.clone()], PathKeyMode::Exact);
        assert_eq!(exact.len(), 2);

        let insensitive =
            dedupe_canonical_paths(vec![upper, lower], PathKeyMode::AsciiCaseInsensitive);
        assert_eq!(insensitive.len(), 1);

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn path_policy_validate_and_dedupe_uses_mode() {
        let policy = PathPolicy::ascii_case_insensitive();
        let paths = vec![
            std::path::PathBuf::from(r"C:\\Temp\\Value"),
            std::path::PathBuf::from(r"c:\\temp\\value"),
        ];

        let deduped = policy
            .validate_and_dedupe(paths, |path| Ok::<_, ()>(path.to_path_buf()))
            .expect("validation should succeed");
        assert_eq!(deduped.len(), 1);
    }

    fn unique_temp_path(prefix: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        std::env::temp_dir().join(format!("agena-{prefix}-{nonce}"))
    }
}
