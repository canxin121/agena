use std::sync::atomic::{AtomicI64, Ordering};

/// Strict-monotonic global sequence allocator.
///
/// On startup, the host should call `init_from(max_seq_in_store)` so the next
/// allocation begins at `max_seq_in_store + 1`. Allocator is process-local;
/// only one instance per database is supported.
#[derive(Debug)]
pub struct SequenceAllocator {
    next: AtomicI64,
}

impl SequenceAllocator {
    pub fn new() -> Self {
        Self {
            next: AtomicI64::new(1),
        }
    }

    pub fn from_high_watermark(highest: i64) -> Self {
        Self {
            next: AtomicI64::new(highest.saturating_add(1).max(1)),
        }
    }

    pub fn init_from(&self, highest: i64) {
        self.next
            .store(highest.saturating_add(1).max(1), Ordering::SeqCst);
    }

    pub fn next(&self) -> i64 {
        self.next.fetch_add(1, Ordering::SeqCst)
    }

    pub fn peek(&self) -> i64 {
        self.next.load(Ordering::SeqCst)
    }
}

impl Default for SequenceAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_strict_monotonic() {
        let alloc = SequenceAllocator::new();
        assert_eq!(alloc.next(), 1);
        assert_eq!(alloc.next(), 2);
        assert_eq!(alloc.next(), 3);
    }

    #[test]
    fn init_from_resumes_after_highest() {
        let alloc = SequenceAllocator::from_high_watermark(42);
        assert_eq!(alloc.next(), 43);
        alloc.init_from(99);
        assert_eq!(alloc.next(), 100);
    }
}
