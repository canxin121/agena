pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

pub(crate) fn is_true(value: &bool) -> bool {
    *value
}

#[cfg(test)]
mod tests {
    use super::{default_true, is_false, is_true};

    #[test]
    fn serializes_boolean_defaults_consistently() {
        assert!(default_true());
        assert!(is_false(&false));
        assert!(!is_false(&true));
        assert!(is_true(&true));
        assert!(!is_true(&false));
    }
}
