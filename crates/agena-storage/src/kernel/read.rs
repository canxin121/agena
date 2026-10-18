//! Shared read allocations retain their accounting for their entire lifetime.
//! The model is deliberately conservative (serialized value nodes plus string
//! capacity allowance), not a measurement of allocator RSS or SQLite caches.

use std::{
    collections::HashMap,
    ops::Deref,
    sync::{Arc, Mutex},
};

use serde::{Serialize, Serializer, ser};

use super::{KernelError, Result, SessionId};
use agena_domain::{BlobId, ContentId};

pub const READ_VIEW_BYTES: usize = 256 * 1024 * 1024;
pub const SESSION_READ_VIEW_BYTES: usize = 192 * 1024 * 1024;
pub const WORKSPACE_READ_VIEW_BYTES: usize = 16 * 1024 * 1024;
pub const SOURCE_READ_VIEW_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadOwner {
    Session(SessionId),
    /// Zero is the shared owner for global listings and change subscriptions.
    Workspace(i64),
}

impl ReadOwner {
    fn byte_limit(self) -> usize {
        match self {
            Self::Session(_) => SESSION_READ_VIEW_BYTES,
            Self::Workspace(_) => WORKSPACE_READ_VIEW_BYTES,
        }
    }
    fn allocation_limit(self) -> usize {
        match self {
            Self::Session(_) => 2048,
            Self::Workspace(_) => 512,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadSource {
    Content(ContentId),
    Blob(BlobId),
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ReadBudgetSnapshot {
    pub bytes: usize,
    pub peak_bytes: usize,
    pub allocations: usize,
    pub peak_allocations: usize,
    pub subscribers: usize,
    pub sessions: usize,
    pub workspaces: usize,
    pub sources: usize,
    pub peak_source_bytes: usize,
    pub rejected: u64,
}

#[derive(Default)]
struct Usage {
    bytes: usize,
    allocations: usize,
    subscribers: usize,
    large_bytes: usize,
}

#[derive(Default)]
struct State {
    totals: ReadBudgetSnapshot,
    owners: HashMap<ReadOwner, Usage>,
    sources: HashMap<ReadSource, Usage>,
}

/// Infrastructure resource owner, shared by caches, returned views and watches.
#[derive(Clone, Default)]
pub struct ReadBudget(Arc<Mutex<State>>);

impl ReadBudget {
    pub fn snapshot(&self) -> ReadBudgetSnapshot {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).totals
    }

    pub fn wrap<T: Serialize>(&self, session: SessionId, value: T) -> Result<ReadView<T>> {
        let bytes = modeled_read_bytes(&value)?;
        self.wrap_charged(session, value, bytes)
    }

    /// For byte buffers whose Vec capacity is known, without estimating each
    /// individual byte as a separately allocated JSON value.
    pub fn wrap_charged<T>(
        &self,
        session: SessionId,
        value: T,
        bytes: usize,
    ) -> Result<ReadView<T>> {
        let lease = self.reserve_view(session, bytes.max(std::mem::size_of::<T>()))?;
        Ok(ReadView::from_lease(value, lease))
    }

    pub fn reserve_view(&self, session: SessionId, bytes: usize) -> Result<ReadLease> {
        self.reserve_owned(ReadOwner::Session(session), bytes)
    }

    pub fn reserve_owned(&self, owner: ReadOwner, bytes: usize) -> Result<ReadLease> {
        if matches!(owner, ReadOwner::Workspace(id) if id < 0) {
            return Err(KernelError::Invalid("invalid workspace".into()));
        }
        self.reserve(owner, None, bytes.saturating_add(256), false)
    }

    pub fn reserve_source(
        &self,
        session: SessionId,
        source: ReadSource,
        bytes: usize,
    ) -> Result<ReadLease> {
        self.reserve(
            ReadOwner::Session(session),
            Some(source),
            bytes.saturating_add(256),
            false,
        )
    }

    pub fn subscribe_content(&self, session: SessionId, content: ContentId) -> Result<ReadLease> {
        self.reserve(
            ReadOwner::Session(session),
            Some(ReadSource::Content(content)),
            4096,
            true,
        )
    }

    pub fn subscribe_global(&self) -> Result<ReadLease> {
        self.reserve(ReadOwner::Workspace(0), None, 4096, true)
    }

    pub fn subscribe(&self, session: SessionId) -> Result<ReadLease> {
        // Covers receiver state and a maximum 256 replay handles. Payloads in
        // the shared feed ring are bounded separately, without a per-watch copy.
        self.reserve(ReadOwner::Session(session), None, 4096, true)
    }

    fn reserve(
        &self,
        read_owner: ReadOwner,
        source: Option<ReadSource>,
        bytes: usize,
        subscriber: bool,
    ) -> Result<ReadLease> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let owner = state.owners.get(&read_owner);
        let source_usage = source.and_then(|source| state.sources.get(&source));
        let large = source.is_some() && bytes > SOURCE_READ_VIEW_BYTES && !subscriber;
        let source_rejected = source_usage.is_some_and(|usage| {
            if large {
                usage.large_bytes > 0
            } else {
                usage.bytes - usage.large_bytes + bytes > SOURCE_READ_VIEW_BYTES
            }
        }) || source_usage.is_some_and(|usage| {
            if subscriber {
                usage.subscribers >= 64
            } else {
                usage.allocations >= 256
            }
        });
        let rejected = source_rejected
            || state.totals.bytes.saturating_add(bytes) > READ_VIEW_BYTES
            || owner
                .is_some_and(|owner| owner.bytes.saturating_add(bytes) > read_owner.byte_limit())
            || (!subscriber
                && (state.totals.allocations >= 8192
                    || owner
                        .is_some_and(|owner| owner.allocations >= read_owner.allocation_limit())))
            || (subscriber
                && (state.totals.subscribers >= 1024
                    || owner.is_some_and(|owner| owner.subscribers >= 512)));
        if rejected || bytes > read_owner.byte_limit() {
            state.totals.rejected += 1;
            return Err(KernelError::Busy);
        }
        if !state.owners.contains_key(&read_owner) {
            match read_owner {
                ReadOwner::Session(_) => state.totals.sessions += 1,
                ReadOwner::Workspace(_) => state.totals.workspaces += 1,
            }
        }
        let owner = state.owners.entry(read_owner).or_default();
        owner.bytes += bytes;
        if subscriber {
            owner.subscribers += 1;
        } else {
            owner.allocations += 1;
        }
        if let Some(source) = source {
            let source = state.sources.entry(source).or_default();
            source.bytes += bytes;
            if subscriber {
                source.subscribers += 1;
            } else {
                source.allocations += 1;
            }
            if large {
                source.large_bytes += bytes;
            }
            let source_bytes = source.bytes;
            state.totals.peak_source_bytes = state.totals.peak_source_bytes.max(source_bytes);
        }
        state.totals.sources = state.sources.len();
        state.totals.bytes += bytes;
        if subscriber {
            state.totals.subscribers += 1;
        } else {
            state.totals.allocations += 1;
        }
        state.totals.peak_bytes = state.totals.peak_bytes.max(state.totals.bytes);
        state.totals.peak_allocations = state.totals.peak_allocations.max(state.totals.allocations);
        Ok(ReadLease {
            budget: self.clone(),
            owner: read_owner,
            bytes,
            subscriber,
            source,
            large,
        })
    }
}

/// An opaque infrastructure lifetime token. Drop releases resources; it never
/// performs I/O. Product callers cannot detach it from a kernel ReadView.
pub struct ReadLease {
    budget: ReadBudget,
    owner: ReadOwner,
    bytes: usize,
    subscriber: bool,
    source: Option<ReadSource>,
    large: bool,
}

impl ReadLease {
    pub fn wrap<T: Serialize>(&self, value: T) -> Result<ReadView<T>> {
        let bytes = modeled_read_bytes(&value)?;
        let lease =
            self.budget
                .reserve(self.owner, self.source, bytes.saturating_add(256), false)?;
        Ok(ReadView::from_lease(value, lease))
    }
}

impl Drop for ReadLease {
    fn drop(&mut self) {
        let mut state = self.budget.0.lock().unwrap_or_else(|e| e.into_inner());
        let owner = state.owners.get_mut(&self.owner).expect("live read owner");
        owner.bytes -= self.bytes;
        if self.subscriber {
            owner.subscribers -= 1;
        } else {
            owner.allocations -= 1;
        }
        if owner.bytes == 0 {
            state.owners.remove(&self.owner);
            match self.owner {
                ReadOwner::Session(_) => state.totals.sessions -= 1,
                ReadOwner::Workspace(_) => state.totals.workspaces -= 1,
            }
        }
        if let Some(source) = self.source {
            let usage = state.sources.get_mut(&source).expect("live read source");
            usage.bytes -= self.bytes;
            if self.subscriber {
                usage.subscribers -= 1;
            } else {
                usage.allocations -= 1;
            }
            if self.large {
                usage.large_bytes -= self.bytes;
            }
            if usage.bytes == 0 {
                state.sources.remove(&source);
            }
        }
        state.totals.sources = state.sources.len();
        state.totals.bytes -= self.bytes;
        if self.subscriber {
            state.totals.subscribers -= 1;
        } else {
            state.totals.allocations -= 1;
        }
    }
}

struct Allocation<T> {
    value: T,
    lease: ReadLease,
}

/// An immutable shared value with no detachable raw Arc or mutable dereference.
/// Cloning a view shares its allocation and lease. Explicitly deep-copying T is
/// a consumer allocation, outside the kernel's retained-view budget.
pub struct ReadView<T>(Arc<Allocation<T>>);

impl<T> Clone for ReadView<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> Deref for ReadView<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0.value
    }
}
impl<T: std::fmt::Debug> std::fmt::Debug for ReadView<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.value.fmt(f)
    }
}
impl<T: PartialEq> PartialEq for ReadView<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0.value == other.0.value
    }
}
impl<T: Eq> Eq for ReadView<T> {}
impl<T: Serialize> Serialize for ReadView<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.0.value.serialize(serializer)
    }
}
impl<T> ReadView<T> {
    pub fn from_lease(value: T, lease: ReadLease) -> Self {
        Self(Arc::new(Allocation { value, lease }))
    }
    pub fn modeled_bytes(&self) -> usize {
        self.0.lease.bytes
    }
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Walks borrowed values without producing a JSON tree, String or byte buffer.
/// Each serialized node gets 128 bytes for enum/container/map allocation cost;
/// strings get twice their length for decoded capacity. These are model units,
/// not a promise about a particular allocator's resident pages.
pub fn modeled_read_bytes<T: Serialize>(value: &T) -> Result<usize> {
    let mut estimator = Estimate {
        bytes: std::mem::size_of::<T>(),
    };
    value
        .serialize(&mut estimator)
        .map_err(|e| KernelError::Storage(e.to_string()))?;
    Ok(estimator.bytes)
}

struct Estimate {
    bytes: usize,
}
impl Estimate {
    fn node(&mut self) {
        self.bytes = self.bytes.saturating_add(128);
    }
}

macro_rules! scalar {
    ($($name:ident($ty:ty)),* $(,)?) => { $(fn $name(self,_:$ty)->std::result::Result<(),Self::Error> { self.node(); Ok(()) })* };
}
impl Serializer for &mut Estimate {
    type Ok = ();
    type Error = serde_json::Error;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;
    scalar!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_i128(i128),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_u128(u128),
        serialize_f32(f32),
        serialize_f64(f64),
        serialize_char(char)
    );
    fn serialize_str(self, v: &str) -> std::result::Result<(), Self::Error> {
        self.node();
        self.bytes = self.bytes.saturating_add(v.len().saturating_mul(2));
        Ok(())
    }
    fn serialize_bytes(self, v: &[u8]) -> std::result::Result<(), Self::Error> {
        self.node();
        self.bytes = self.bytes.saturating_add(v.len().saturating_mul(2));
        Ok(())
    }
    fn serialize_none(self) -> std::result::Result<(), Self::Error> {
        self.node();
        Ok(())
    }
    fn serialize_some<T: ?Sized + Serialize>(self, v: &T) -> std::result::Result<(), Self::Error> {
        v.serialize(self)
    }
    fn serialize_unit(self) -> std::result::Result<(), Self::Error> {
        self.node();
        Ok(())
    }
    fn serialize_unit_struct(self, _: &'static str) -> std::result::Result<(), Self::Error> {
        self.serialize_unit()
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> std::result::Result<(), Self::Error> {
        self.serialize_unit()
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        v: &T,
    ) -> std::result::Result<(), Self::Error> {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        v: &T,
    ) -> std::result::Result<(), Self::Error> {
        self.node();
        v.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> std::result::Result<Self, Self::Error> {
        self.node();
        Ok(self)
    }
    fn serialize_tuple(self, len: usize) -> std::result::Result<Self, Self::Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        len: usize,
    ) -> std::result::Result<Self, Self::Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        len: usize,
    ) -> std::result::Result<Self, Self::Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_map(self, len: Option<usize>) -> std::result::Result<Self, Self::Error> {
        self.serialize_seq(len)
    }
    fn serialize_struct(
        self,
        _: &'static str,
        len: usize,
    ) -> std::result::Result<Self, Self::Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        len: usize,
    ) -> std::result::Result<Self, Self::Error> {
        self.serialize_seq(Some(len))
    }
}
macro_rules! sequence {
    ($trait:ident,$method:ident) => {
        impl ser::$trait for &mut Estimate {
            type Ok = ();
            type Error = serde_json::Error;
            fn $method<T: ?Sized + Serialize>(
                &mut self,
                v: &T,
            ) -> std::result::Result<(), Self::Error> {
                v.serialize(&mut **self)
            }
            fn end(self) -> std::result::Result<(), Self::Error> {
                Ok(())
            }
        }
    };
}
sequence!(SerializeSeq, serialize_element);
sequence!(SerializeTuple, serialize_element);
sequence!(SerializeTupleStruct, serialize_field);
sequence!(SerializeTupleVariant, serialize_field);
impl ser::SerializeMap for &mut Estimate {
    type Ok = ();
    type Error = serde_json::Error;
    fn serialize_key<T: ?Sized + Serialize>(
        &mut self,
        v: &T,
    ) -> std::result::Result<(), Self::Error> {
        v.serialize(&mut **self)
    }
    fn serialize_value<T: ?Sized + Serialize>(
        &mut self,
        v: &T,
    ) -> std::result::Result<(), Self::Error> {
        v.serialize(&mut **self)
    }
    fn end(self) -> std::result::Result<(), Self::Error> {
        Ok(())
    }
}
macro_rules! structure {
    ($trait:ident) => {
        impl ser::$trait for &mut Estimate {
            type Ok = ();
            type Error = serde_json::Error;
            fn serialize_field<T: ?Sized + Serialize>(
                &mut self,
                _: &'static str,
                v: &T,
            ) -> std::result::Result<(), Self::Error> {
                v.serialize(&mut **self)
            }
            fn end(self) -> std::result::Result<(), Self::Error> {
                Ok(())
            }
        }
    };
}
structure!(SerializeStruct);
structure!(SerializeStructVariant);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_one_accounted_allocation_until_the_last_view_drops() {
        let budget = ReadBudget::default();
        let session = SessionId::try_from(1).unwrap();
        let view = budget.wrap(session, vec!["界".repeat(1000)]).unwrap();
        let clone = view.clone();
        assert!(view.ptr_eq(&clone));
        assert_eq!(budget.snapshot().allocations, 1);
        assert_eq!(budget.snapshot().bytes, view.modeled_bytes());
        assert!(view.modeled_bytes() > 6000);
        drop(view);
        assert_eq!(budget.snapshot().allocations, 1);
        drop(clone);
        assert_eq!(budget.snapshot().bytes, 0);
        assert_eq!(budget.snapshot().sessions, 0);
    }

    #[test]
    fn retained_views_and_subscriptions_share_global_and_session_admission() {
        let budget = ReadBudget::default();
        let a = SessionId::try_from(1).unwrap();
        let b = SessionId::try_from(2).unwrap();
        let c = SessionId::try_from(3).unwrap();
        let full = budget
            .reserve_view(a, SESSION_READ_VIEW_BYTES - 256)
            .unwrap();
        assert!(matches!(budget.subscribe(a), Err(KernelError::Busy)));
        let other = budget.reserve_view(b, 32 * 1024 * 1024).unwrap();
        assert!(matches!(
            budget.reserve_view(c, 80 * 1024 * 1024),
            Err(KernelError::Busy)
        ));
        drop(full);
        let mut subscriptions = Vec::new();
        for _ in 0..512 {
            subscriptions.push(budget.subscribe(a).unwrap());
        }
        assert!(matches!(budget.subscribe(a), Err(KernelError::Busy)));
        drop(subscriptions.pop());
        subscriptions.push(budget.subscribe(a).unwrap());
        drop((subscriptions, other));
        assert_eq!(budget.snapshot().bytes, 0);
        assert_eq!(budget.snapshot().subscribers, 0);
    }

    #[test]
    fn model_charges_small_json_nodes_as_well_as_encoded_text() {
        let text = serde_json::json!({"body":"x".repeat(8000)});
        let nodes = serde_json::json!({"body":vec![0; 4000]});
        assert!(modeled_read_bytes(&nodes).unwrap() > modeled_read_bytes(&text).unwrap() * 10);
    }

    #[test]
    fn workspace_views_have_their_own_owner_and_share_the_global_budget() {
        let budget = ReadBudget::default();
        let session = SessionId::try_from(1).unwrap();
        let workspace = budget
            .reserve_owned(ReadOwner::Workspace(1), WORKSPACE_READ_VIEW_BYTES - 256)
            .unwrap();
        assert!(matches!(
            budget.reserve_owned(ReadOwner::Workspace(1), 1),
            Err(KernelError::Busy)
        ));
        let session_lease = budget
            .reserve_view(session, SESSION_READ_VIEW_BYTES - 256)
            .unwrap();
        let other_workspace = budget.reserve_owned(ReadOwner::Workspace(2), 1024).unwrap();
        assert_eq!(budget.snapshot().sessions, 1);
        assert_eq!(budget.snapshot().workspaces, 2);
        drop(workspace);
        assert_eq!(budget.snapshot().workspaces, 1);
        assert!(budget.reserve_owned(ReadOwner::Workspace(1), 1024).is_ok());
        assert!(matches!(
            budget.reserve_owned(ReadOwner::Workspace(-1), 1),
            Err(KernelError::Invalid(_))
        ));
        let global = budget.reserve_owned(ReadOwner::Workspace(0), 1).unwrap();
        assert_eq!(budget.snapshot().workspaces, 2);
        drop(global);
        drop((session_lease, other_workspace));
        assert_eq!(budget.snapshot().bytes, 0);
        assert_eq!(budget.snapshot().sessions, 0);
        assert_eq!(budget.snapshot().workspaces, 0);
    }

    #[test]
    fn one_source_cannot_exhaust_other_sources_and_a_large_atomic_view_keeps_its_lease() {
        let budget = ReadBudget::default();
        let session = SessionId::try_from(1).unwrap();
        let a = ContentId::new();
        let b = ContentId::new();
        let full = budget
            .reserve_source(
                session,
                ReadSource::Content(a),
                SOURCE_READ_VIEW_BYTES - 256,
            )
            .unwrap();
        assert!(matches!(
            budget.reserve_source(session, ReadSource::Content(a), 1),
            Err(KernelError::Busy)
        ));
        let other = budget
            .reserve_source(
                session,
                ReadSource::Content(b),
                SOURCE_READ_VIEW_BYTES - 256,
            )
            .unwrap();
        let mut watchers = Vec::new();
        drop(full);
        for _ in 0..64 {
            watchers.push(budget.subscribe_content(session, a).unwrap());
        }
        assert!(matches!(
            budget.subscribe_content(session, a),
            Err(KernelError::Busy)
        ));
        let frame = budget
            .reserve_source(session, ReadSource::Content(a), 64 * 1024 * 1024)
            .unwrap();
        assert!(matches!(
            budget.reserve_source(session, ReadSource::Content(a), 64 * 1024 * 1024),
            Err(KernelError::Busy)
        ));
        let head = budget
            .reserve_source(session, ReadSource::Content(a), 1024)
            .unwrap();
        drop((other, watchers, head, frame));
        assert_eq!(budget.snapshot().bytes, 0);
        assert_eq!(budget.snapshot().sources, 0);
    }

    #[test]
    fn a_valid_fully_styled_terminal_screen_fits_the_large_view_admission() {
        use agena_domain::{TerminalAttributes, TerminalCellRun, TerminalColor, TerminalSnapshot};
        let attributes = TerminalAttributes {
            foreground: Some(TerminalColor::Rgb {
                red: 255,
                green: 128,
                blue: 64,
            }),
            background: Some(TerminalColor::Rgb {
                red: 64,
                green: 128,
                blue: 255,
            }),
            bold: true,
            dim: true,
            italic: true,
            underline: true,
            inverse: true,
        };
        let screen = TerminalSnapshot {
            rows: 200,
            cols: 200,
            cursor_row: 0,
            cursor_col: 0,
            cursor_visible: true,
            alternate_screen: false,
            bracketed_paste: false,
            application_cursor: false,
            cells: (0..200)
                .flat_map(|row| (0..200).map(move |col| (row, col)))
                .map(|(row, col)| TerminalCellRun {
                    row,
                    col,
                    text: "x".into(),
                    attributes: attributes.clone(),
                })
                .collect(),
        };
        screen.validate(None).unwrap();
        assert!(serde_json::to_vec(&screen).unwrap().len() < 16 * 1024 * 1024);
        let budget = ReadBudget::default();
        let session = SessionId::try_from(1).unwrap();
        let content = ContentId::new();
        let bytes = modeled_read_bytes(&screen).unwrap();
        // The old 96-MiB session model was too small for this valid shape.
        assert!(bytes > 96 * 1024 * 1024);
        let watch = budget.subscribe_content(session, content).unwrap();
        let lease = budget
            .reserve_source(session, ReadSource::Content(content), bytes)
            .unwrap();
        let view = ReadView::from_lease(screen, lease);
        assert_eq!(view.cells.len(), 40_000);
        drop((view, watch));
        assert_eq!(budget.snapshot().bytes, 0);
    }
}
