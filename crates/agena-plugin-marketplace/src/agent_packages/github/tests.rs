use std::{collections::BTreeMap, sync::Mutex};

use serde_json::{Value, json};

#[cfg(unix)]
use crate::agent_packages::file_mode;
use crate::agent_packages::{
    AgentEcosystem, AgentPackageInstaller, AgentPackageRequest, Catalog, CatalogFiles,
};

use super::*;

const COMMIT: &str = "1111111111111111111111111111111111111111";
const ROOT: &str = "2222222222222222222222222222222222222222";
const PLUGINS: &str = "3333333333333333333333333333333333333333";
const PACKAGE: &str = "4444444444444444444444444444444444444444";

#[derive(Default)]
struct Fixture {
    replies: BTreeMap<String, Vec<u8>>,
    requests: Mutex<Vec<String>>,
}

impl HttpFetcher for Fixture {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, MarketplaceError> {
        self.requests.lock().unwrap().push(url.into());
        Ok(self
            .replies
            .get(url)
            .unwrap_or_else(|| panic!("unexpected fetch {url}"))
            .clone())
    }
}

impl Fixture {
    fn json(&mut self, url: &str, value: Value) {
        self.replies
            .insert(url.into(), serde_json::to_vec(&value).unwrap());
    }

    fn tree(&mut self, repo: &str, sha: &str, recursive: bool, entries: Vec<Value>) {
        let query = if recursive { "?recursive=1" } else { "" };
        self.json(
            &format!("https://api.github.com/repos/{repo}/git/trees/{sha}{query}"),
            json!({"sha":sha, "truncated":false, "tree":entries}),
        );
    }

    fn file(
        &mut self,
        repo: &str,
        directory: &str,
        relative: &str,
        raw_relative: &str,
        bytes: &[u8],
        mode: &str,
    ) -> Value {
        let url =
            format!("https://raw.githubusercontent.com/{repo}/{COMMIT}/{directory}/{raw_relative}");
        self.replies.insert(url, bytes.to_vec());
        blob(relative, bytes, mode)
    }
}

fn subtree(path: &str, sha: &str) -> Value {
    json!({"path":path, "sha":sha, "mode":"040000", "type":"tree"})
}

fn blob(path: &str, bytes: &[u8], mode: &str) -> Value {
    let oid = Oid::hash_object(ObjectType::Blob, bytes)
        .unwrap()
        .to_string();
    json!({"path":path, "sha":oid, "size":bytes.len(), "mode":mode, "type":"blob"})
}

fn run_command(_: &str, _: &Path, _: Duration) -> Result<PathBuf, MarketplaceError> {
    panic!("the fixture does not execute a marketplace install command")
}

#[test]
fn official_marketplace_search_and_install_only_read_the_catalog_and_selected_tree() {
    let repo = "openai/plugins";
    let head = format!("https://api.github.com/repos/{repo}/commits?per_page=1");
    let index = format!(
        "https://raw.githubusercontent.com/{repo}/{COMMIT}/.agents/plugins/marketplace.json"
    );
    let mut fetcher = Fixture::default();
    fetcher.json(
        &head,
        json!([{"sha":COMMIT,"commit":{"tree":{"sha":ROOT}}}]),
    );
    fetcher.json(
        &index,
        json!({
            "name":"openai-curated",
            "metadata":{"pluginRoot":"./plugins"},
            "plugins":[
                {"name":"review","source":{"source":"local","path":"review"}},
                {"name":"unselected","source":{"source":"local","path":"unselected"}}
            ]
        }),
    );
    fetcher.tree(repo, ROOT, false, vec![subtree("plugins", PLUGINS)]);
    fetcher.tree(
        repo,
        PLUGINS,
        false,
        vec![subtree("review", PACKAGE), subtree("unselected", ROOT)],
    );
    let skill = fetcher.file(
        repo,
        "plugins/review",
        "skills/check/SKILL.md",
        "skills/check/SKILL.md",
        b"# Review",
        "100644",
    );
    let script = fetcher.file(
        repo,
        "plugins/review",
        "scripts/check #1.sh",
        "scripts/check%20%231.sh",
        b"#!/bin/sh\n",
        "100755",
    );
    fetcher.tree(repo, PACKAGE, true, vec![skill, script]);

    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let installer = AgentPackageInstaller {
        root: &cache,
        fetcher: &fetcher,
        command: &run_command,
    };
    let search = installer
        .search(AgentEcosystem::Codex, temp.path(), None, "review")
        .unwrap();
    assert_eq!(search.entries.len(), 1);
    assert_eq!(search.entries[0].identity, "review@openai-curated");
    assert_eq!(*fetcher.requests.lock().unwrap(), [head, index]);

    let prepared = installer
        .prepare(AgentPackageRequest {
            ecosystem: AgentEcosystem::Codex,
            selector: "review",
            marketplace: None,
            package_source: None,
            cwd: temp.path(),
            accept_command: None,
        })
        .unwrap();
    assert_eq!(prepared.source.registry.as_deref(), Some(repo));
    assert_eq!(prepared.identity, "review@openai-curated");
    let script = prepared.path.join("scripts/check #1.sh");
    #[cfg(unix)]
    assert_eq!(file_mode(&fs::metadata(&script).unwrap()), 0o111);
    assert_eq!(fs::read(script).unwrap(), b"#!/bin/sh\n");
    assert_eq!(
        fs::read(prepared.path.join("skills/check/SKILL.md")).unwrap(),
        b"# Review"
    );
    assert!(
        prepared
            .publish()
            .unwrap()
            .path
            .join(".codex-plugin/plugin.json")
            .is_file()
    );
    assert_eq!(
        fetcher.requests.lock().unwrap().len(),
        fetcher.replies.len() + 2
    );
}

#[test]
fn github_marketplace_package_subdir_uses_its_pinned_commit_even_with_a_ref() {
    let repo = "fixture/toolkit";
    let index = "https://fixture.invalid/marketplace.json";
    let mut fetcher = Fixture::default();
    fetcher.json(
        index,
        json!({
            "name":"claude-market",
            "plugins":[{"name":"review","source":{
                "source":"git-subdir",
                "url":"https://github.com/fixture/toolkit.git",
                "path":"packages/review",
                "ref":"main",
                "sha":COMMIT
            }}]
        }),
    );
    fetcher.json(
        &format!("https://api.github.com/repos/{repo}/git/commits/{COMMIT}"),
        json!({"sha":COMMIT,"tree":{"sha":ROOT}}),
    );
    fetcher.tree(repo, ROOT, false, vec![subtree("packages", PLUGINS)]);
    fetcher.tree(repo, PLUGINS, false, vec![subtree("review", PACKAGE)]);
    let command = fetcher.file(
        repo,
        "packages/review",
        "commands/review.md",
        "commands/review.md",
        b"Review this change.",
        "100644",
    );
    fetcher.tree(repo, PACKAGE, true, vec![command]);
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let installer = AgentPackageInstaller {
        root: &cache,
        fetcher: &fetcher,
        command: &run_command,
    };
    let prepared = installer
        .prepare(AgentPackageRequest {
            ecosystem: AgentEcosystem::Claude,
            selector: "review",
            marketplace: Some(index),
            package_source: None,
            cwd: temp.path(),
            accept_command: None,
        })
        .unwrap();
    assert_eq!(prepared.identity, "review@claude-market");
    assert_eq!(
        fs::read(prepared.path.join("commands/review.md")).unwrap(),
        b"Review this change."
    );
    assert!(prepared.path.join(".claude-plugin/plugin.json").is_file());
    assert_eq!(
        fetcher.requests.lock().unwrap().len(),
        fetcher.replies.len()
    );
}

#[test]
fn github_package_stage_rejects_wrong_blob_and_parent_paths() {
    let repo = "fixture/toolkit";
    let snapshot = Snapshot {
        repository: repo.into(),
        commit: oid(COMMIT).unwrap(),
        tree: oid(ROOT).unwrap(),
    };
    let temp = tempfile::tempdir().unwrap();
    let mut fetcher = Fixture::default();
    fetcher.tree(repo, ROOT, false, vec![subtree("package", PACKAGE)]);
    let bad = blob("config.json", b"approved", "100644");
    fetcher.file(
        repo,
        "package",
        "config.json",
        "config.json",
        b"modified",
        "100644",
    );
    fetcher.tree(repo, PACKAGE, true, vec![bad]);
    let dest = temp.path().join("bad-blob");
    let result = snapshot.stage_package(&fetcher, Path::new("package"), &dest);
    assert!(result.unwrap_err().to_string().contains("does not match"));
    assert!(!dest.join("config.json").exists());

    fetcher.tree(
        repo,
        PACKAGE,
        true,
        vec![blob("../other", b"approved", "100644")],
    );
    let dest = temp.path().join("unsafe-path");
    let result = snapshot.stage_package(&fetcher, Path::new("package"), &dest);
    assert!(result.unwrap_err().to_string().contains("unsafe path"));
    assert!(!dest.exists());
}

#[test]
fn github_package_failure_falls_back_to_the_catalog_commit() {
    struct Unavailable;
    impl HttpFetcher for Unavailable {
        fn fetch(&self, _: &str) -> Result<Vec<u8>, MarketplaceError> {
            Err(MarketplaceError::Http("GitHub API unavailable".into()))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let repository = temp.path().join("origin");
    let git = git2::Repository::init(&repository).unwrap();
    fs::create_dir(repository.join("plugin")).unwrap();
    let signature = git2::Signature::now("Fixture", "fixture@example.test").unwrap();
    let commit = |body, parents: &[&git2::Commit<'_>]| {
        fs::write(repository.join("plugin/SKILL.md"), body).unwrap();
        let mut index = git.index().unwrap();
        index.add_path(Path::new("plugin/SKILL.md")).unwrap();
        let tree = git.find_tree(index.write_tree().unwrap()).unwrap();
        git.commit(Some("HEAD"), &signature, &signature, body, &tree, parents)
            .unwrap()
    };
    let original = git.find_commit(commit("Original", &[])).unwrap();
    commit("Updated", &[&original]);
    let catalog = Catalog {
        name: "fixture".into(),
        source: Url::from_directory_path(repository).unwrap().to_string(),
        files: CatalogFiles::GitHub(Snapshot {
            repository: "fixture/toolkit".into(),
            commit: original.id(),
            tree: original.tree_id(),
        }),
        revision: String::new(),
        document: json!({"plugins":[{"name":"review","source":"./plugin"}]}),
    };
    let installer = AgentPackageInstaller {
        root: temp.path(),
        fetcher: &Unavailable,
        command: &run_command,
    };
    let path = installer
        .entry_root(
            &catalog,
            catalog.entry("review").unwrap(),
            "review@fixture",
            None,
            temp.path(),
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(path.join("SKILL.md")).unwrap(),
        "Original"
    );
}
