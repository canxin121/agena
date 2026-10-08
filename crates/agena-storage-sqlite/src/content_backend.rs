//! Segmented content persistence, independent of SQL and client protocols.
//!
//! A manifest is published only after its record files are synced. Retrying a
//! failed batch truncates files to the last published lengths first, so a
//! partial append can never duplicate records or masquerade as committed.

use std::collections::{BTreeMap, HashMap};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use agena_domain::{
    ContentChunk, ContentCursor, ContentId, ContentPage, ContentResource, ContentState,
};
use agena_storage::content::{
    ContentArchive, ContentBackend, merge_range, page_from_records, validate_cursor,
};
use agena_storage::store::StoreError;
use async_trait::async_trait;
use lru::LruCache;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

#[derive(Debug, Clone)]
pub struct FileContentConfig {
    pub segment_bytes: usize,
    pub retained_segments: usize,
    pub cached_manifests: usize,
    pub max_retained_bytes: u64,
    pub max_store_bytes: u64,
}

impl Default for FileContentConfig {
    fn default() -> Self {
        Self {
            segment_bytes: 1024 * 1024,
            retained_segments: 16,
            cached_manifests: 256,
            max_retained_bytes: 64 * 1024 * 1024,
            max_store_bytes: 1024 * 1024 * 1024,
        }
    }
}

pub struct FileContentBackend {
    root: PathBuf,
    config: FileContentConfig,
    manifests: Mutex<ManifestCache>,
    quota: tokio::sync::OnceCell<Arc<quota::StoreQuota>>,
}

type ManifestHandle = Arc<tokio::sync::Mutex<Manifest>>;

/// The LRU bounds idle manifests. Weak handles preserve one commit gate even
/// when an in-use manifest is evicted by unrelated resource reads.
struct ManifestCache {
    hot: LruCache<ContentId, ManifestHandle>,
    handles: HashMap<ContentId, Weak<tokio::sync::Mutex<Manifest>>>,
}

impl ManifestCache {
    fn get(&mut self, id: &ContentId) -> Option<ManifestHandle> {
        if let Some(handle) = self.hot.get(id) {
            return Some(handle.clone());
        }
        let handle = self.handles.get(id)?.upgrade()?;
        self.hot.put(*id, handle.clone());
        Some(handle)
    }

    fn insert(&mut self, id: ContentId, handle: ManifestHandle) {
        if self.handles.len() >= self.hot.cap().get().saturating_mul(2) {
            self.handles.retain(|_, handle| handle.strong_count() > 0);
        }
        self.handles.insert(id, Arc::downgrade(&handle));
        self.hot.put(id, handle);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    resource: ContentResource,
    segments: Vec<Segment>,
    terminal_snapshot: Option<ContentCursor>,
    terminal_frame: Option<(ContentCursor, u16, u16, bool)>,
    document_snapshot: Option<ContentCursor>,
    document_cursor: Option<ContentCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Segment {
    first: u64,
    last: u64,
    file_bytes: u64,
    document_last: Option<u64>,
    terminal_last: Option<u64>,
}

impl FileContentBackend {
    pub fn new(root: impl Into<PathBuf>, config: FileContentConfig) -> Self {
        assert!(config.segment_bytes > 0);
        assert!(config.max_retained_bytes > 0);
        assert!(config.max_store_bytes > 0);
        assert!(
            config.retained_segments >= 2,
            "retain a startup segment and recent output"
        );
        let capacity = NonZeroUsize::new(config.cached_manifests).expect("nonempty manifest cache");
        Self {
            root: root.into(),
            config,
            quota: tokio::sync::OnceCell::new(),
            manifests: Mutex::new(ManifestCache {
                hot: LruCache::new(capacity),
                handles: HashMap::new(),
            }),
        }
    }

    fn directory(&self, id: ContentId) -> PathBuf {
        self.root.join(id.to_string())
    }

    fn segment_path(directory: &Path, first: u64) -> PathBuf {
        directory.join(format!("records-{first:020}.jsonl"))
    }

    async fn quota(&self) -> Result<&Arc<quota::StoreQuota>, StoreError> {
        self.quota
            .get_or_try_init(|| quota::StoreQuota::scan(&self.root, self.config.max_store_bytes))
            .await
    }

    async fn manifest(
        &self,
        id: ContentId,
        initial: Option<&ContentResource>,
    ) -> Result<Arc<tokio::sync::Mutex<Manifest>>, StoreError> {
        if let Some(cached) = self.manifests.lock().expect("manifest cache lock").get(&id) {
            return Ok(cached);
        }
        let path = self.directory(id).join("manifest.json");
        let loaded = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice::<Manifest>(&bytes)
                .map_err(|error| StoreError::Serialization(format!("content manifest: {error}")))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let resource = initial
                    .filter(|resource| resource.cursor.sequence == 0)
                    .ok_or_else(|| StoreError::not_found(format!("content {id}")))?;
                Manifest {
                    resource: resource.clone(),
                    segments: Vec::new(),
                    terminal_snapshot: None,
                    terminal_frame: None,
                    document_snapshot: None,
                    document_cursor: None,
                }
            }
            Err(error) => return Err(io_error(error)),
        };
        if loaded.resource.resource_id != id {
            return Err(StoreError::Constraint(
                "content manifest identity differs from resource".into(),
            ));
        }
        let mut cache = self.manifests.lock().expect("manifest cache lock");
        if let Some(cached) = cache.get(&id) {
            return Ok(cached);
        }
        let loaded = Arc::new(tokio::sync::Mutex::new(loaded));
        cache.insert(id, loaded.clone());
        Ok(loaded)
    }

    async fn publish(directory: &Path, manifest: &Manifest) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec(manifest)
            .map_err(|error| StoreError::Serialization(error.to_string()))?;
        let temporary = directory.join("manifest-next.json");
        let mut file = tokio::fs::File::create(&temporary)
            .await
            .map_err(io_error)?;
        file.write_all(&bytes).await.map_err(io_error)?;
        file.sync_all().await.map_err(io_error)?;
        drop(file);
        tokio::fs::rename(temporary, directory.join("manifest.json"))
            .await
            .map_err(io_error)?;
        // The directory entry is part of the durable publication boundary.
        #[cfg(unix)]
        {
            let directory = directory.to_path_buf();
            tokio::task::spawn_blocking(move || std::fs::File::open(directory)?.sync_all())
                .await
                .map_err(|error| StoreError::Io(error.to_string()))?
                .map_err(io_error)?;
        }
        Ok(())
    }

    async fn read_segment(
        directory: &Path,
        segment: &Segment,
    ) -> Result<Vec<Arc<ContentChunk>>, StoreError> {
        let file = tokio::fs::File::open(Self::segment_path(directory, segment.first))
            .await
            .map_err(io_error)?;
        let mut bytes = Vec::new();
        file.take(segment.file_bytes)
            .read_to_end(&mut bytes)
            .await
            .map_err(io_error)?;
        if bytes.len() as u64 != segment.file_bytes {
            return Err(StoreError::Io(
                "committed content segment is incomplete".into(),
            ));
        }
        bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| {
                serde_json::from_slice(line)
                    .map(Arc::new)
                    .map_err(|error| StoreError::Serialization(error.to_string()))
            })
            .collect()
    }

    /// Only recovery scans the directory. Normal commits use the manifest and
    /// byte reservations, without per-record filesystem or database queries.
    async fn recover_unpublished(
        &self,
        directory: &Path,
        manifest: &mut Manifest,
    ) -> Result<(), StoreError> {
        match tokio::fs::read(directory.join("manifest.json")).await {
            Ok(bytes) => {
                let published: Manifest = serde_json::from_slice(&bytes)
                    .map_err(|error| StoreError::Serialization(error.to_string()))?;
                if published.resource.resource_id != manifest.resource.resource_id
                    || published.resource.cursor.epoch != manifest.resource.cursor.epoch
                    || published.resource.part_id != manifest.resource.part_id
                    || published.resource.owner_session_id != manifest.resource.owner_session_id
                    || published.resource.kind != manifest.resource.kind
                {
                    return Err(StoreError::Constraint(
                        "content recovery ownership changed".into(),
                    ));
                }
                *manifest = published;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
        let mut entries = match tokio::fs::read_dir(directory).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io_error(error)),
        };
        while let Some(entry) = entries.next_entry().await.map_err(io_error)? {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name == "manifest-next.json" {
                tokio::fs::remove_file(entry.path())
                    .await
                    .map_err(io_error)?;
            } else if let Some(first) = name
                .strip_prefix("records-")
                .and_then(|name| name.strip_suffix(".jsonl"))
                .and_then(|first| first.parse::<u64>().ok())
            {
                if let Some(segment) = manifest
                    .segments
                    .iter()
                    .find(|segment| segment.first == first)
                {
                    let file = tokio::fs::OpenOptions::new()
                        .write(true)
                        .open(entry.path())
                        .await
                        .map_err(io_error)?;
                    let length = file.metadata().await.map_err(io_error)?.len();
                    if length < segment.file_bytes {
                        return Err(StoreError::Io(
                            "committed content segment is incomplete".into(),
                        ));
                    }
                    if length > segment.file_bytes {
                        file.set_len(segment.file_bytes).await.map_err(io_error)?;
                        file.sync_data().await.map_err(io_error)?;
                    }
                } else {
                    tokio::fs::remove_file(entry.path())
                        .await
                        .map_err(io_error)?;
                }
            }
        }
        #[cfg(unix)]
        {
            let directory = directory.to_path_buf();
            tokio::task::spawn_blocking(move || std::fs::File::open(directory)?.sync_all())
                .await
                .map_err(|error| StoreError::Io(error.to_string()))?
                .map_err(io_error)?;
        }
        Ok(())
    }
}

#[async_trait]
impl ContentBackend for FileContentBackend {
    async fn restore(&self, archive: &ContentArchive) -> Result<(), StoreError> {
        archive.validate()?;
        let id = archive.resource.resource_id;
        let destination = self.directory(id);
        if tokio::fs::try_exists(&destination)
            .await
            .map_err(io_error)?
        {
            return Err(StoreError::Conflict(
                "content resource already exists".into(),
            ));
        }
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(io_error)?;
        let directory = self.root.join(format!(".import-{id}"));
        tokio::fs::create_dir(&directory).await.map_err(io_error)?;
        let mut published = false;
        let result = async {
            let mut manifest = Manifest {
                resource: archive.resource.clone(),
                segments: Vec::new(),
                terminal_snapshot: None,
                terminal_frame: None,
                document_snapshot: None,
                document_cursor: None,
            };
            let mut writes = BTreeMap::<u64, Vec<u8>>::new();
            for chunk in &archive.chunks {
                let mut encoded = serde_json::to_vec(chunk)
                    .map_err(|error| StoreError::Serialization(error.to_string()))?;
                encoded.push(b'\n');
                if manifest.segments.last().is_none_or(|segment| {
                    segment.last + 1 != chunk.cursor.sequence
                        || segment.file_bytes + encoded.len() as u64
                            > self.config.segment_bytes as u64
                }) {
                    manifest.segments.push(Segment {
                        first: chunk.cursor.sequence,
                        last: chunk.cursor.sequence,
                        file_bytes: 0,
                        document_last: None,
                        terminal_last: None,
                    });
                }
                let segment = manifest
                    .segments
                    .last_mut()
                    .expect("archive segment exists");
                segment.last = chunk.cursor.sequence;
                segment.file_bytes += encoded.len() as u64;
                mark_checkpoint_record(segment, chunk);
                writes.entry(segment.first).or_default().extend(encoded);
                match &chunk.payload {
                    agena_domain::ContentPayload::StructuredSnapshot { .. } => {
                        manifest.document_snapshot = Some(chunk.cursor);
                        manifest.document_cursor = Some(chunk.cursor);
                    }
                    agena_domain::ContentPayload::Structured { .. } => {
                        manifest.document_cursor = Some(chunk.cursor)
                    }
                    agena_domain::ContentPayload::Terminal { screen } => {
                        manifest.terminal_snapshot = Some(chunk.cursor);
                        manifest.terminal_frame = Some((
                            chunk.cursor,
                            screen.rows,
                            screen.cols,
                            screen.alternate_screen,
                        ));
                    }
                    agena_domain::ContentPayload::TerminalPatch { screen, .. } => {
                        manifest.terminal_frame = Some((
                            chunk.cursor,
                            screen.rows,
                            screen.cols,
                            screen.alternate_screen,
                        ));
                    }
                    _ => {}
                }
            }
            if manifest
                .segments
                .iter()
                .map(|segment| segment.file_bytes)
                .sum::<u64>()
                > self.config.max_retained_bytes
                || manifest.segments.len()
                    > self.config.retained_segments.max(
                        if manifest.document_snapshot.is_some()
                            || manifest.terminal_snapshot.is_some()
                        {
                            34
                        } else {
                            0
                        },
                    )
            {
                return Err(StoreError::Constraint(
                    "content archive exceeds the retained-segment budget".into(),
                ));
            }
            let bytes = manifest.file_bytes()?;
            let lease = self.quota().await?.reserve(id, bytes, &self.root).await?;
            for (first, bytes) in writes {
                let mut file = tokio::fs::File::create(Self::segment_path(&directory, first))
                    .await
                    .map_err(io_error)?;
                file.write_all(&bytes).await.map_err(io_error)?;
                file.sync_all().await.map_err(io_error)?;
            }
            Self::publish(&directory, &manifest).await?;
            tokio::fs::rename(&directory, &destination)
                .await
                .map_err(io_error)?;
            published = true;
            #[cfg(unix)]
            {
                let root = self.root.clone();
                tokio::task::spawn_blocking(move || std::fs::File::open(root)?.sync_all())
                    .await
                    .map_err(|error| StoreError::Io(error.to_string()))?
                    .map_err(io_error)?;
            }
            lease.finish(bytes);
            Ok(())
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_dir_all(directory).await;
            if published {
                let _ = tokio::fs::remove_dir_all(destination).await;
            }
        }
        result
    }

    async fn prune(
        &self,
        protected: &std::collections::HashSet<ContentId>,
    ) -> Result<usize, StoreError> {
        let mut entries = match tokio::fs::read_dir(&self.root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(io_error(error)),
        };
        let mut count = 0;
        let mut abandoned_imports = 0;
        while let Some(entry) = entries.next_entry().await.map_err(io_error)? {
            let name = entry.file_name();
            if name
                .to_str()
                .is_some_and(|name| name.starts_with(".import-"))
            {
                if abandoned_imports < 64
                    && entry.file_type().await.map_err(io_error)?.is_dir()
                    && entry
                        .metadata()
                        .await
                        .map_err(io_error)?
                        .modified()
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age >= std::time::Duration::from_secs(300))
                {
                    tokio::fs::remove_dir_all(entry.path())
                        .await
                        .map_err(io_error)?;
                    if let (Some(quota), Some(id)) = (
                        self.quota.get(),
                        name.to_str()
                            .and_then(|name| name.strip_prefix(".import-"))
                            .and_then(|name| {
                                serde_json::from_value::<ContentId>(serde_json::Value::String(
                                    name.into(),
                                ))
                                .ok()
                            }),
                    ) {
                        quota.mark_dirty(id);
                    }
                    abandoned_imports += 1;
                }
                continue;
            }
            let Some(id) = entry.file_name().to_str().and_then(|name| {
                serde_json::from_value::<ContentId>(serde_json::Value::String(name.into())).ok()
            }) else {
                continue;
            };
            if protected.contains(&id) {
                continue;
            }
            // Another server may have just created a source but not yet
            // committed its Part reference. A grace period covers that window.
            let metadata = match tokio::fs::metadata(entry.path().join("manifest.json")).await {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error(error)),
            };
            if metadata
                .modified()
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_none_or(|age| age < std::time::Duration::from_secs(300))
            {
                continue;
            }
            self.delete(id).await?;
            count += 1;
        }
        Ok(count)
    }

    async fn delete(&self, id: ContentId) -> Result<(), StoreError> {
        // Readers and retention use the same per-resource gate.
        let handle = match self.manifest(id, None).await {
            Ok(handle) => handle,
            Err(StoreError::NotFound(_)) => {
                if let Some(quota) = self.quota.get() {
                    quota.deleted(id);
                }
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let _gate = handle.lock().await;
        match tokio::fs::remove_dir_all(self.directory(id)).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
        let mut cache = self.manifests.lock().expect("manifest cache lock");
        cache.hot.pop(&id);
        cache.handles.remove(&id);
        drop(cache);
        if let Some(quota) = self.quota.get() {
            quota.deleted(id);
        }
        Ok(())
    }

    async fn describe(&self, id: ContentId) -> Result<ContentResource, StoreError> {
        Ok(self.manifest(id, None).await?.lock().await.resource.clone())
    }

    async fn commit(
        &self,
        mut resource: ContentResource,
        chunks: &[Arc<ContentChunk>],
    ) -> Result<ContentResource, StoreError> {
        let handle = self.manifest(resource.resource_id, Some(&resource)).await?;
        let mut previous = handle.lock().await;
        let directory = self.directory(resource.resource_id);
        let quota = self.quota().await?;
        if quota.needs_reconciliation(resource.resource_id) {
            self.recover_unpublished(&directory, &mut previous).await?;
            quota.reconcile(resource.resource_id, &self.root).await?;
        }
        if previous.resource.cursor.epoch != resource.cursor.epoch
            || previous.resource.part_id != resource.part_id
            || previous.resource.owner_session_id != resource.owner_session_id
            || previous.resource.kind != resource.kind
        {
            return Err(StoreError::Constraint(
                "content ownership or generation changed".into(),
            ));
        }
        if resource.cursor.sequence < previous.resource.committed_cursor.sequence {
            return Err(StoreError::Conflict(
                "content commit is older than the published cursor".into(),
            ));
        }
        if previous.resource.state != ContentState::Active {
            // A manifest rename can succeed before the directory sync fails.
            // Recovery has now synced that publication. Retention is backend
            // owned and may differ from the source's pre-publication view.
            resource.retained_ranges = previous.resource.retained_ranges.clone();
            if previous.resource == resource {
                return Ok(previous.resource.clone());
            }
            return Err(StoreError::InvalidState(
                "committed content is sealed".into(),
            ));
        }
        let mut next = previous.clone();
        let mut writes = BTreeMap::<u64, Vec<u8>>::new();
        let mut expected = previous.resource.committed_cursor.sequence;
        for chunk in chunks
            .iter()
            .filter(|chunk| chunk.cursor.sequence > previous.resource.committed_cursor.sequence)
        {
            chunk
                .payload
                .validate()
                .map_err(|error| StoreError::Constraint(error.to_owned()))?;
            if chunk.cursor.epoch != resource.cursor.epoch
                || expected.checked_add(1) != Some(chunk.cursor.sequence)
                || !resource.kind.accepts(chunk.payload.kind())
            {
                return Err(StoreError::Constraint(
                    "content batch is not a contiguous resource stream".into(),
                ));
            }
            expected = chunk.cursor.sequence;
            use agena_domain::ContentPayload;
            match &chunk.payload {
                ContentPayload::Structured { base_cursor, .. } => {
                    if next.document_cursor != Some(*base_cursor) {
                        return Err(StoreError::Constraint(
                            "document mutation does not extend the committed document".into(),
                        ));
                    }
                    next.document_cursor = Some(chunk.cursor);
                }
                ContentPayload::StructuredSnapshot { .. } => {
                    next.document_cursor = Some(chunk.cursor)
                }
                ContentPayload::TerminalPatch {
                    base_cursor,
                    screen,
                    ..
                } => {
                    if next.terminal_frame
                        != Some((
                            *base_cursor,
                            screen.rows,
                            screen.cols,
                            screen.alternate_screen,
                        ))
                    {
                        return Err(StoreError::Constraint(
                            "terminal patch does not extend the committed screen".into(),
                        ));
                    }
                    next.terminal_frame = Some((
                        chunk.cursor,
                        screen.rows,
                        screen.cols,
                        screen.alternate_screen,
                    ));
                }
                ContentPayload::Terminal { screen } => {
                    next.terminal_frame = Some((
                        chunk.cursor,
                        screen.rows,
                        screen.cols,
                        screen.alternate_screen,
                    ));
                }
                _ => {}
            }
            if matches!(chunk.payload, agena_domain::ContentPayload::Terminal { .. }) {
                next.terminal_snapshot = Some(chunk.cursor);
            }
            if matches!(
                chunk.payload,
                agena_domain::ContentPayload::StructuredSnapshot { .. }
            ) {
                next.document_snapshot = Some(chunk.cursor);
            }
            let mut encoded = serde_json::to_vec(chunk.as_ref())
                .map_err(|error| StoreError::Serialization(error.to_string()))?;
            encoded.push(b'\n');
            if next.segments.last().is_none_or(|last| {
                last.file_bytes > 0
                    && last.file_bytes + encoded.len() as u64 > self.config.segment_bytes as u64
            }) {
                next.segments.push(Segment {
                    first: expected,
                    last: expected,
                    file_bytes: 0,
                    document_last: None,
                    terminal_last: None,
                });
            }
            let segment = next.segments.last_mut().expect("record segment exists");
            segment.last = expected;
            segment.file_bytes += encoded.len() as u64;
            mark_checkpoint_record(segment, chunk);
            writes.entry(segment.first).or_default().extend(encoded);
        }
        if (expected != resource.cursor.sequence
            && !agena_storage::content::accounts_for_lost_tail(&resource, expected))
            || resource.committed_cursor != resource.cursor
        {
            return Err(StoreError::Constraint(
                "content manifest cursor does not cover the batch".into(),
            ));
        }
        let mut removed = Vec::new();
        while next.segments.len() > self.config.retained_segments
            || next
                .segments
                .iter()
                .map(|segment| segment.file_bytes)
                .sum::<u64>()
                > self.config.max_retained_bytes
        {
            let index = (1..next.segments.len().saturating_sub(1)).find(|index| {
                let segment = &next.segments[*index];
                next.document_snapshot.is_none_or(|cursor| {
                    segment
                        .document_last
                        .is_none_or(|sequence| sequence < cursor.sequence)
                }) && next.terminal_snapshot.is_none_or(|cursor| {
                    segment
                        .terminal_last
                        .is_none_or(|sequence| sequence < cursor.sequence)
                })
            });
            let Some(index) = index else { break };
            removed.push(next.segments.remove(index));
        }
        if next
            .segments
            .iter()
            .map(|segment| segment.file_bytes)
            .sum::<u64>()
            > self.config.max_retained_bytes
        {
            return Err(StoreError::Constraint(
                "content checkpoint and recent output exceed the retained-byte budget".into(),
            ));
        }
        // Decide retention before touching record files. A rejected batch
        // preserves the published manifest and all of its record lengths.
        if next.segments.len() > self.config.retained_segments.max(34) {
            return Err(StoreError::Constraint(
                "content checkpoint chain exceeds the retained-segment budget".into(),
            ));
        }
        resource.retained_ranges.clear();
        for segment in &next.segments {
            merge_range(
                &mut resource.retained_ranges,
                agena_domain::ContentRange {
                    first: segment.first,
                    last: segment.last,
                },
            );
        }
        next.resource = resource.clone();
        let next_bytes = next.file_bytes()?;
        let lease = if expected == previous.resource.committed_cursor.sequence
            && resource.state != ContentState::Active
            && resource
                .capture_error
                .as_ref()
                .is_none_or(|error| error.len() <= 1024)
        {
            quota
                .reserve_seal(resource.resource_id, next_bytes, &self.root)
                .await?
        } else {
            quota
                .reserve(resource.resource_id, next_bytes, &self.root)
                .await?
        };
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(io_error)?;
        for (first, bytes) in writes {
            if !next.segments.iter().any(|segment| segment.first == first) {
                continue;
            }
            let published_length = previous
                .segments
                .iter()
                .find(|segment| segment.first == first)
                .map_or(0, |segment| segment.file_bytes);
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(Self::segment_path(&directory, first))
                .await
                .map_err(io_error)?;
            // Removes a prior attempt's unpublished tail before retrying.
            file.set_len(published_length).await.map_err(io_error)?;
            file.seek(std::io::SeekFrom::Start(published_length))
                .await
                .map_err(io_error)?;
            file.write_all(&bytes).await.map_err(io_error)?;
            file.sync_data().await.map_err(io_error)?;
        }
        Self::publish(&directory, &next).await?;
        *previous = next;
        let mut cleanup_failed = false;
        for segment in removed {
            if let Err(error) =
                tokio::fs::remove_file(Self::segment_path(&directory, segment.first)).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(%error, "retired content segment cleanup failed");
                cleanup_failed = true;
            }
        }
        if cleanup_failed {
            lease.finish(quota::resource_bytes(&self.root, resource.resource_id).await?);
        } else {
            lease.finish(next_bytes);
        }
        Ok(resource)
    }

    async fn read(
        &self,
        id: ContentId,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> Result<ContentPage, StoreError> {
        let handle = self.manifest(id, None).await?;
        // Hold the manifest gate while reading its bounded files so retention
        // cannot delete a range this read has already selected.
        let manifest = handle.lock().await;
        validate_cursor(&manifest.resource, after)?;
        let after = after.or_else(|| {
            manifest
                .terminal_snapshot
                .or(manifest.document_snapshot)
                .map(|cursor| ContentCursor {
                    sequence: cursor.sequence - 1,
                    ..cursor
                })
        });
        let directory = self.directory(id);
        let mut records = Vec::new();
        let mut bytes = 0;
        if let Some(after) = after {
            for segment in manifest
                .segments
                .iter()
                .filter(|segment| segment.last > after.sequence)
            {
                for chunk in Self::read_segment(&directory, segment)
                    .await?
                    .into_iter()
                    .filter(|chunk| chunk.cursor.sequence > after.sequence)
                {
                    bytes += chunk.payload.byte_len();
                    records.push(chunk);
                    if bytes >= max_bytes {
                        break;
                    }
                }
                if bytes >= max_bytes {
                    break;
                }
            }
        } else {
            for segment in manifest.segments.iter().rev() {
                let segment_records = Self::read_segment(&directory, segment).await?;
                for chunk in segment_records.into_iter().rev() {
                    bytes += chunk.payload.byte_len();
                    records.push(chunk);
                    if bytes >= max_bytes {
                        break;
                    }
                }
                if bytes >= max_bytes {
                    break;
                }
            }
            records.reverse();
        }
        Ok(page_from_records(
            &manifest.resource,
            records.iter(),
            after,
            max_bytes,
        ))
    }
}

fn io_error(error: std::io::Error) -> StoreError {
    StoreError::Io(error.to_string())
}

impl Manifest {
    fn file_bytes(&self) -> Result<u64, StoreError> {
        Ok(self
            .segments
            .iter()
            .map(|segment| segment.file_bytes)
            .sum::<u64>()
            + serde_json::to_vec(self)
                .map_err(|error| StoreError::Serialization(error.to_string()))?
                .len() as u64)
    }
}

fn mark_checkpoint_record(segment: &mut Segment, chunk: &ContentChunk) {
    match &chunk.payload {
        agena_domain::ContentPayload::Structured { .. }
        | agena_domain::ContentPayload::StructuredSnapshot { .. } => {
            segment.document_last = Some(chunk.cursor.sequence)
        }
        agena_domain::ContentPayload::Terminal { .. }
        | agena_domain::ContentPayload::TerminalPatch { .. } => {
            segment.terminal_last = Some(chunk.cursor.sequence)
        }
        _ => {}
    }
}

mod quota;
#[cfg(test)]
mod tests;
