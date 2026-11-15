use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

#[test]
fn alias_matching_captures_the_canonical_root_identity() {
    let directory = std::env::temp_dir().join(format!(
        "agena-path-alias-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    std::fs::create_dir(&directory).unwrap();
    let directory = std::fs::canonicalize(directory).unwrap();
    let real = directory.join("real");
    let alias = directory.join("alias");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let pattern =
        absolute_alias_patterns("<home>/output/**", "home", vec![alias.clone()], "output/**")
            .unwrap();
    for target in [alias.join("output/a"), real.join("output/a")] {
        assert!(pattern.matches(&MatchContext::new(&directory, &target)));
    }
    assert!(!pattern.matches(&MatchContext::new(&directory, &real.join("elsewhere/a"))));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn all_temp_alias_roots_are_absolute_on_the_current_platform() {
    assert!(temp_roots().iter().all(|root| root.is_absolute()));
}
