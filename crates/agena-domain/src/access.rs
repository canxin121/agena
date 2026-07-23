#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessSelector {
    Read,
    Write,
    Any,
}

#[cfg(test)]
mod tests {
    use super::{AccessKind, AccessSelector};

    #[test]
    fn access_values_distinguish_read_write_and_any() {
        assert_ne!(AccessKind::Read, AccessKind::Write);
        assert_ne!(AccessSelector::Read, AccessSelector::Write);
        assert_ne!(AccessSelector::Any, AccessSelector::Read);
    }
}
