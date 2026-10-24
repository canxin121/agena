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
