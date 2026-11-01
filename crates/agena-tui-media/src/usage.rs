//! Track whether one rendered entry is still waiting on changing media.
//! Text-only and completed entries must not reflow when another image finishes.

use std::cell::RefCell;

thread_local! {
    static USAGE: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn record() {
    USAGE.with(|usage| {
        if let Some(current) = usage.borrow_mut().last_mut() {
            *current = true;
        }
    });
}

/// Return the render result and whether it still awaits media availability.
/// Completed pixels and permanent failures do not depend on other images;
/// content, width and theme invalidation remain the caller's responsibility.
/// Nested scopes propagate dependencies and are cleaned up during unwinding.
pub fn track_media_usage<T>(render: impl FnOnce() -> T) -> (T, bool) {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            USAGE.with(|usage| {
                let mut usage = usage.borrow_mut();
                let observed = usage.pop().unwrap_or(false);
                if let Some(parent) = usage.last_mut() {
                    *parent |= observed;
                }
            });
        }
    }
    USAGE.with(|usage| usage.borrow_mut().push(false));
    let _guard = Guard;
    let result = render();
    let observed = USAGE.with(|usage| usage.borrow().last().copied().unwrap_or(false));
    (result, observed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_usage_is_local_and_propagates_to_its_parent() {
        assert_eq!(track_media_usage(|| 42), (42, false));
        let (_, outer) = track_media_usage(|| {
            assert!(!track_media_usage(|| ()).1);
            assert!(track_media_usage(record).1);
        });
        assert!(outer);
        assert!(!track_media_usage(|| ()).1);
    }

    #[test]
    fn failed_media_is_tracked_and_unwinding_does_not_leak_a_scope() {
        let (result, used) = track_media_usage(|| {
            crate::with_deferred_remote_images(|| {
                crate::render_markdown_image("https://usage-audit.invalid/not-downloaded.png")
            })
        });
        assert!(result.is_err());
        assert!(used);
        let _ = std::panic::catch_unwind(|| {
            track_media_usage(|| {
                record();
                panic!("synthetic rendering failure");
            })
        });
        assert!(!track_media_usage(|| ()).1);
    }
}
