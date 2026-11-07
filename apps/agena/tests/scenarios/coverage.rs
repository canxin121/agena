//! This lightweight Cargo test prevents a newly added Rust crate from being
//! silently omitted from the repository-wide review inventory. It does *not*
//! misrepresent the inventory as execution or line coverage.

use std::{collections::BTreeSet, fs, path::PathBuf};

#[test]
fn every_rust_package_is_listed_once_in_the_coverage_inventory() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|parent| parent.parent())
        .expect("Agena application resides under apps/agena")
        .to_path_buf();
    let mut actual = BTreeSet::new();
    // The workspace also contains independently compiled example and tooling
    // packages. Omitting them would make this coverage gate silently miss
    // additions listed under Cargo workspace.members.
    for group in ["crates", "apps", "examples", "tools"] {
        for result in fs::read_dir(root.join(group)).expect("Rust workspace root is readable") {
            let entry = result.expect("Rust workspace directory entry is readable");
            let path = entry.path();
            if !path.join("Cargo.toml").is_file() || !path.join("src").is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_str().expect("Rust package directory is UTF-8");
            actual.insert(format!("{group}/{name}"));
        }
    }

    let inventory = fs::read_to_string(root.join("docs/testing/coverage-audit.md"))
        .expect("coverage audit is retained in the repository");
    let mut listed = BTreeSet::new();
    for line in inventory.lines() {
        let Some(rest) = line.strip_prefix("| `") else {
            continue;
        };
        let Some((name, _)) = rest.split_once("` |") else {
            continue;
        };
        assert!(
            listed.insert(name.to_owned()),
            "duplicate coverage inventory entry: {name}"
        );
    }
    let missing: Vec<_> = actual.difference(&listed).collect();
    let stale: Vec<_> = listed.difference(&actual).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "Rust coverage inventory must list every package exactly once; missing={missing:?}; stale={stale:?}"
    );
}
