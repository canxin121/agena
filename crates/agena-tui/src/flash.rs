//! Ephemeral terminal notification presentation state.

use std::time::{Duration, Instant};

/// Default lifetime for a transient terminal notification.
pub const DEFAULT_FLASH_DURATION: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashLevel {
    Success,
    Warning,
    Error,
    Info,
}

#[derive(Debug, Clone)]
pub struct FlashMessage {
    pub text: String,
    pub level: FlashLevel,
    pub expires_at: Instant,
}

impl FlashMessage {
    pub fn new(level: FlashLevel, text: impl Into<String>) -> Self {
        Self::with_lifetime(level, text, DEFAULT_FLASH_DURATION)
    }

    pub fn with_lifetime(level: FlashLevel, text: impl Into<String>, lifetime: Duration) -> Self {
        Self {
            text: text.into(),
            level,
            expires_at: Instant::now() + lifetime,
        }
    }

    pub fn is_expired_at(&self, now: Instant) -> bool {
        now >= self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::{FlashLevel, FlashMessage};
    use std::time::Duration;

    #[test]
    fn flash_expiration_is_a_presentation_policy() {
        let flash = FlashMessage::with_lifetime(FlashLevel::Info, "saved", Duration::ZERO);
        assert_eq!(flash.text, "saved");
        assert!(flash.is_expired_at(flash.expires_at));
    }
}
