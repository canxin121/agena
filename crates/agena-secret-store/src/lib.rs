//! Application-managed credentials with a single private filesystem backend.
//!
//! Every namespace is one atomically replaced authenticated ciphertext. A stable sidecar
//! lock serializes read/modify/write across processes and independent handles.
//! Filesystem access is restricted to the running account; no operating-system
//! credential manager or fallback backend is involved. The random encryption
//! key is application-managed in the same private directory. This protects a
//! copied namespace file; copying the key and ciphertext together, or access
//! as the same operating-system account, remains outside that protection.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use agena_storage_layout::{StorageLayout, resolve_private_path, secure_directory, secure_file};
use ring::{
    aead::{self, Aad, LessSafeKey, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use sha2::{Digest, Sha256};

const CIPHERTEXT_MAGIC: &[u8] = b"AGENA-SECRET-V1\0";
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

pub const DEFAULT_NAMESPACE: &str = "agena";

#[derive(Debug, thiserror::Error)]
pub enum SecretStoreError {
    #[error("secret not found")]
    NotFound,
    #[error("secret storage unavailable: {0}")]
    Unavailable(String),
    #[error("secret storage error: {0}")]
    Other(String),
}

impl SecretStoreError {
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

/// Synchronous secret operations. Async callers must use a blocking worker.
/// Error messages and debug implementations must never include secret values.
pub trait SecretStore: Send + Sync {
    fn get_secret(&self, key: &str) -> Result<Option<String>, SecretStoreError>;
    fn set_secret(&self, key: &str, value: &str) -> Result<(), SecretStoreError>;
    fn delete_secret(&self, key: &str) -> Result<(), SecretStoreError>;
    fn list_secret_keys(&self) -> Result<Vec<String>, SecretStoreError> {
        Err(SecretStoreError::Other(
            "listing is unsupported by this store".to_owned(),
        ))
    }
    /// Atomically initialize a persistent identity and return the winning value.
    fn get_or_insert_secret(
        &self,
        _key: &str,
        _candidate: &str,
    ) -> Result<String, SecretStoreError> {
        Err(SecretStoreError::Other(
            "atomic initialization is unsupported by this store".to_owned(),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct FileSecretStore {
    root: PathBuf,
    namespace_digest: String,
}

impl FileSecretStore {
    pub fn new(namespace: impl AsRef<str>) -> Self {
        Self::at(StorageLayout::current().secrets_dir(), namespace)
    }

    pub fn at(root: impl Into<PathBuf>, namespace: impl AsRef<str>) -> Self {
        Self {
            root: root.into(),
            namespace_digest: hex::encode(Sha256::digest(namespace.as_ref().as_bytes())),
        }
    }

    fn path(&self) -> PathBuf {
        self.root.join(format!("{}.enc", self.namespace_digest))
    }

    fn locked<T>(
        &self,
        operation: impl FnOnce(&Self, &mut BTreeMap<String, String>) -> Result<T, SecretStoreError>,
    ) -> Result<T, SecretStoreError> {
        let store = Self {
            root: resolve_private_path(&self.root).map_err(storage_io)?,
            namespace_digest: self.namespace_digest.clone(),
        };
        secure_directory(&store.root).map_err(storage_io)?;
        // Recheck after creation, then use this same resolved path for every
        // lock, master-key operation, read and atomic replacement.
        resolve_private_path(&store.root).map_err(storage_io)?;
        let lock_path = store.root.join(format!("{}.lock", store.namespace_digest));
        let lock = private_open(&lock_path, true).map_err(storage_io)?;
        // Each operation opens its own lock handle, so threads in this process
        // obey the same locking protocol as separate processes.
        lock.lock().map_err(storage_io)?;
        let mut values = store.read()?;
        operation(&store, &mut values)
        // Dropping the handle releases the lock, including on any error path.
    }

    fn encryption_key(&self) -> Result<LessSafeKey, SecretStoreError> {
        let lock = private_open(&self.root.join("master.lock"), true).map_err(storage_io)?;
        lock.lock().map_err(storage_io)?;
        let path = self.root.join("master.key");
        let key_bytes = match private_open(&path, false) {
            Ok(mut file) => {
                let mut bytes = [0u8; KEY_LEN];
                file.read_exact(&mut bytes).map_err(|_| invalid_key())?;
                let mut extra = [0u8; 1];
                if file.read(&mut extra).map_err(storage_io)? != 0 {
                    return Err(invalid_key());
                }
                bytes
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // A missing master key is data loss, never an invitation to
                // replace the identity and overwrite existing ciphertext.
                for entry in fs::read_dir(&self.root).map_err(storage_io)? {
                    let entry = entry.map_err(storage_io)?;
                    if entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "enc")
                    {
                        return Err(SecretStoreError::Other(
                            "secret encryption key is missing while encrypted records exist"
                                .to_owned(),
                        ));
                    }
                }
                let mut bytes = [0u8; KEY_LEN];
                SystemRandom::new()
                    .fill(&mut bytes)
                    .map_err(|_| crypto_error())?;
                self.persist_bytes(&path, &bytes)?;
                bytes
            }
            Err(error) => return Err(storage_io(error)),
        };
        UnboundKey::new(&aead::CHACHA20_POLY1305, &key_bytes)
            .map(LessSafeKey::new)
            .map_err(|_| crypto_error())
    }

    fn read(&self) -> Result<BTreeMap<String, String>, SecretStoreError> {
        let mut file = match private_open(&self.path(), false) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(error) => return Err(storage_io(error)),
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(storage_io)?;
        if !bytes.starts_with(CIPHERTEXT_MAGIC)
            || bytes.len() < CIPHERTEXT_MAGIC.len() + NONCE_LEN + aead::CHACHA20_POLY1305.tag_len()
        {
            return Err(invalid_document());
        }
        let payload_start = CIPHERTEXT_MAGIC.len() + NONCE_LEN;
        let nonce_bytes: [u8; NONCE_LEN] = bytes[CIPHERTEXT_MAGIC.len()..payload_start]
            .try_into()
            .map_err(|_| invalid_document())?;
        let key = self.encryption_key()?;
        let plaintext = key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce_bytes),
                Aad::from(self.namespace_digest.as_bytes()),
                &mut bytes[payload_start..],
            )
            .map_err(|_| invalid_document())?;
        // A serde error may contain input fragments. Never propagate it from
        // the secret boundary, and never treat corrupted storage as empty.
        serde_json::from_slice(plaintext).map_err(|_| invalid_document())
    }

    fn write(&self, values: &BTreeMap<String, String>) -> Result<(), SecretStoreError> {
        let key = self.encryption_key()?;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce_bytes)
            .map_err(|_| crypto_error())?;
        let mut ciphertext = serde_json::to_vec(values)
            .map_err(|_| SecretStoreError::Other("cannot encode secret namespace".to_owned()))?;
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::from(self.namespace_digest.as_bytes()),
            &mut ciphertext,
        )
        .map_err(|_| crypto_error())?;
        let mut envelope =
            Vec::with_capacity(CIPHERTEXT_MAGIC.len() + NONCE_LEN + ciphertext.len());
        envelope.extend_from_slice(CIPHERTEXT_MAGIC);
        envelope.extend_from_slice(&nonce_bytes);
        envelope.extend_from_slice(&ciphertext);
        self.persist_bytes(&self.path(), &envelope)
    }

    fn persist_bytes(&self, path: &Path, bytes: &[u8]) -> Result<(), SecretStoreError> {
        let mut temporary = tempfile::Builder::new()
            .prefix(".secret-")
            .tempfile_in(&self.root)
            .map_err(storage_io)?;
        secure_file(temporary.path()).map_err(storage_io)?;
        temporary.write_all(bytes).map_err(storage_io)?;
        temporary.flush().map_err(storage_io)?;
        temporary.as_file().sync_all().map_err(storage_io)?;
        match private_open(path, false) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage_io(error)),
        }
        temporary
            .persist(path)
            .map_err(|error| storage_io(error.error))?;
        sync_directory(&self.root).map_err(storage_io)
    }
}

impl Default for FileSecretStore {
    fn default() -> Self {
        Self::new(DEFAULT_NAMESPACE)
    }
}

impl SecretStore for FileSecretStore {
    fn get_secret(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
        self.locked(|_, values| Ok(values.get(key).cloned()))
    }

    fn set_secret(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
        self.locked(|store, values| {
            values.insert(key.to_owned(), value.to_owned());
            store.write(values)
        })
    }

    fn delete_secret(&self, key: &str) -> Result<(), SecretStoreError> {
        self.locked(|store, values| {
            if values.remove(key).is_some() {
                store.write(values)?;
            }
            Ok(())
        })
    }

    fn list_secret_keys(&self) -> Result<Vec<String>, SecretStoreError> {
        self.locked(|_, values| Ok(values.keys().cloned().collect()))
    }

    fn get_or_insert_secret(&self, key: &str, candidate: &str) -> Result<String, SecretStoreError> {
        self.locked(|store, values| {
            if let Some(value) = values.get(key) {
                return Ok(value.clone());
            }
            values.insert(key.to_owned(), candidate.to_owned());
            store.write(values)?;
            Ok(candidate.to_owned())
        })
    }
}

fn invalid_document() -> SecretStoreError {
    SecretStoreError::Other("invalid or unauthenticated encrypted secret document".to_owned())
}

fn invalid_key() -> SecretStoreError {
    SecretStoreError::Other("invalid secret encryption key".to_owned())
}

fn crypto_error() -> SecretStoreError {
    SecretStoreError::Unavailable("secret cryptographic operation failed".to_owned())
}

fn storage_io(error: io::Error) -> SecretStoreError {
    // io::Error from custom platform backends can carry arbitrary strings.
    // Restrict the public error to the operation class and OS error kind.
    SecretStoreError::Unavailable(format!("filesystem operation failed ({:?})", error.kind()))
}

fn private_open(path: &Path, create: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT prevents following a final symlink.
        options.custom_flags(0x0020_0000);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe secret file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // Reject hard links and files owned by another account before any
        // permission changes or access to their content.
        if metadata.nlink() != 1 || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unsafe secret file owner or links",
            ));
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    secure_file(path)?;
    Ok(file)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_temp_aliases_share_one_encrypted_store_and_reject_user_links() {
        for base in ["/tmp", "/var/tmp"] {
            let directory = tempfile::tempdir_in(base).unwrap();
            let root = directory.path().join("secrets");
            let store = FileSecretStore::at(&root, "macos-system-alias");
            assert_eq!(
                store.get_or_insert_secret("identity", "original").unwrap(),
                "original"
            );
            let canonical = FileSecretStore::at(root.canonicalize().unwrap(), "macos-system-alias");
            assert_eq!(
                canonical
                    .get_or_insert_secret("identity", "replacement")
                    .unwrap(),
                "original"
            );
            canonical.set_secret("next", "value").unwrap();
            assert_eq!(store.get_secret("next").unwrap().as_deref(), Some("value"));

            let alias = directory.path().join("user-alias");
            std::os::unix::fs::symlink(&root, &alias).unwrap();
            assert!(
                FileSecretStore::at(alias, "macos-system-alias")
                    .set_secret("identity", "changed")
                    .is_err()
            );
            assert_eq!(
                canonical.get_secret("identity").unwrap().as_deref(),
                Some("original")
            );
        }
    }

    #[test]
    fn namespaces_are_isolated_and_delete_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let first = FileSecretStore::at(directory.path(), "one");
        let second = FileSecretStore::at(directory.path(), "../two");
        first.set_secret("token", "sensitive-value").unwrap();
        assert_eq!(
            first.get_secret("token").unwrap().as_deref(),
            Some("sensitive-value")
        );
        assert_eq!(second.get_secret("token").unwrap(), None);
        assert_eq!(first.list_secret_keys().unwrap(), vec!["token"]);
        first.delete_secret("token").unwrap();
        first.delete_secret("token").unwrap();
        assert_eq!(first.get_secret("token").unwrap(), None);
    }

    #[test]
    fn independent_handles_do_not_lose_concurrent_updates() {
        let directory = tempfile::tempdir().unwrap();
        let barrier = Arc::new(Barrier::new(8));
        let handles = (0..8)
            .map(|index| {
                let store = FileSecretStore::at(directory.path(), "shared");
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.set_secret(&index.to_string(), "value").unwrap();
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(
            FileSecretStore::at(directory.path(), "shared")
                .list_secret_keys()
                .unwrap()
                .len(),
            8
        );
    }

    #[test]
    fn concurrent_identity_initialization_returns_one_persistent_winner() {
        let directory = tempfile::tempdir().unwrap();
        let barrier = Arc::new(Barrier::new(8));
        let handles = (0..8)
            .map(|index| {
                let store = FileSecretStore::at(directory.path(), "identity");
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store
                        .get_or_insert_secret("signing-key", &index.to_string())
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let winners = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert!(winners.iter().all(|winner| winner == &winners[0]));
        assert_eq!(
            FileSecretStore::at(directory.path(), "identity")
                .get_secret("signing-key")
                .unwrap()
                .as_ref(),
            Some(&winners[0])
        );
    }

    #[test]
    fn child_process_writer() {
        let Some(root) = std::env::var_os("AGENA_SECRET_STORE_TEST_ROOT") else {
            return;
        };
        let writer = std::env::var("AGENA_SECRET_STORE_TEST_WRITER").unwrap();
        let store = FileSecretStore::at(PathBuf::from(root), "multiprocess");
        for index in 0..16 {
            store
                .set_secret(&format!("{writer}-{index}"), "value")
                .unwrap();
        }
    }

    #[test]
    fn processes_serialize_atomic_namespace_updates() {
        let directory = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let mut children = (0..3)
            .map(|writer| {
                std::process::Command::new(&executable)
                    .args(["--exact", "tests::child_process_writer", "--quiet"])
                    .env("AGENA_SECRET_STORE_TEST_ROOT", directory.path())
                    .env("AGENA_SECRET_STORE_TEST_WRITER", writer.to_string())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }
        let store = FileSecretStore::at(directory.path(), "multiprocess");
        assert_eq!(store.list_secret_keys().unwrap().len(), 48);
    }

    #[test]
    fn encrypted_records_hide_values_and_bind_the_namespace() {
        let directory = tempfile::tempdir().unwrap();
        let first = FileSecretStore::at(directory.path(), "first");
        let second = FileSecretStore::at(directory.path(), "second");
        let secret = "this-value-must-never-appear-on-disk";
        first.set_secret("private-token-name", secret).unwrap();
        let encrypted = fs::read(first.path()).unwrap();
        assert!(
            !encrypted
                .windows(secret.len())
                .any(|window| window == secret.as_bytes())
        );
        assert!(
            !encrypted
                .windows("private-token-name".len())
                .any(|window| window == b"private-token-name")
        );
        fs::copy(first.path(), second.path()).unwrap();
        assert!(second.get_secret("private-token-name").is_err());
        first.set_secret("private-token-name", secret).unwrap();
        assert_ne!(encrypted, fs::read(first.path()).unwrap());
        let mut modified = fs::read(first.path()).unwrap();
        *modified.last_mut().unwrap() ^= 1;
        fs::write(first.path(), &modified).unwrap();
        assert!(first.get_secret("private-token-name").is_err());
        assert!(first.set_secret("replacement", "value").is_err());
        assert_eq!(fs::read(first.path()).unwrap(), modified);
    }

    #[test]
    fn missing_master_key_never_replaces_an_existing_identity() {
        let directory = tempfile::tempdir().unwrap();
        let first = FileSecretStore::at(directory.path(), "first");
        first.set_secret("key", "secret").unwrap();
        fs::remove_file(directory.path().join("master.key")).unwrap();
        assert!(first.get_secret("key").is_err());
        let second = FileSecretStore::at(directory.path(), "second");
        assert!(second.set_secret("key", "value").is_err());
        assert!(!directory.path().join("master.key").exists());
    }

    #[test]
    fn corrupt_store_does_not_overwrite_or_disclose_values() {
        let directory = tempfile::tempdir().unwrap();
        let store = FileSecretStore::at(directory.path(), "corrupt");
        fs::write(store.path(), r#"{"value":"do-not-disclose", broken}"#).unwrap();
        let error = store.set_secret("next", "value").unwrap_err();
        assert!(!error.to_string().contains("do-not-disclose"));
        assert!(
            fs::read_to_string(store.path())
                .unwrap()
                .contains("do-not-disclose")
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_permissions_and_symlink_rejection() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = tempfile::tempdir().unwrap();
        let store = FileSecretStore::at(directory.path().join("secrets"), "namespace");
        store.set_secret("key", "value").unwrap();
        assert_eq!(
            fs::metadata(&store.root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(store.root.join("master.key"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let outside = directory.path().join("outside");
        fs::write(&outside, "unchanged").unwrap();
        fs::remove_file(store.path()).unwrap();
        symlink(&outside, store.path()).unwrap();
        assert!(store.set_secret("key", "new").is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "unchanged");
    }
    #[cfg(unix)]
    #[test]
    fn rejects_lock_symlinks_hardlinks_and_directory_symlinks() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside");
        fs::write(&outside, "untouched").unwrap();
        let store = FileSecretStore::at(directory.path().join("secrets"), "namespace");
        secure_directory(&store.root).unwrap();
        let lock = store.root.join(format!("{}.lock", store.namespace_digest));
        symlink(&outside, &lock).unwrap();
        assert!(store.get_secret("key").is_err());
        fs::remove_file(lock).unwrap();
        fs::hard_link(&outside, store.path()).unwrap();
        assert!(store.set_secret("key", "value").is_err());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "untouched");
        let linked_root = directory.path().join("linked");
        symlink(&store.root, &linked_root).unwrap();
        assert!(
            FileSecretStore::at(linked_root, "namespace")
                .get_secret("key")
                .is_err()
        );
    }
}
