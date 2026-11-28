//! Vendor registry dependencies into the immutable DeepSeek package snapshot.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    AgentPackageInstaller, Map, MarketplaceError, PackageCopyBudget, Path, PathBuf, Value,
    copy_package_with_budget, fs, invalid, npm_dependency_request, npm_dependency_version_matches,
    read_object,
};

const MAX_DEPENDENCIES: usize = 128;
const MAX_DEPTH: usize = 16;

pub(super) fn materialize(
    installer: &AgentPackageInstaller<'_>,
    root: &Path,
    manifest: &Map<String, Value>,
    registry: Option<&str>,
    staging: &Path,
    budget: PackageCopyBudget,
) -> Vec<String> {
    let mut vendor = Vendor {
        installer,
        root,
        registry,
        staging,
        budget,
        visited: BTreeSet::from([root.to_owned()]),
        attempts: 0,
        diagnostics: Vec::new(),
    };
    vendor.dependencies(root, manifest, 0);
    vendor.diagnostics
}

struct Vendor<'a, 'host> {
    installer: &'a AgentPackageInstaller<'host>,
    root: &'a Path,
    registry: Option<&'a str>,
    staging: &'a Path,
    budget: PackageCopyBudget,
    visited: BTreeSet<PathBuf>,
    attempts: usize,
    diagnostics: Vec<String>,
}

impl Vendor<'_, '_> {
    fn dependencies(&mut self, package: &Path, manifest: &Map<String, Value>, depth: usize) {
        let mut requests = BTreeMap::new();
        for field in ["dependencies", "optionalDependencies"] {
            match manifest.get(field) {
                None => {}
                Some(Value::Object(entries)) => requests.extend(entries),
                Some(_) => self.diagnostics.push(format!(
                    "DeepSeek npm package {}: {field} must be an object",
                    package_label(self.root, package),
                )),
            }
        }
        for (name, request) in requests {
            if depth >= MAX_DEPTH || self.attempts >= MAX_DEPENDENCIES {
                let message = "DeepSeek npm dependency graph exceeds its size or depth limit";
                if !self.diagnostics.iter().any(|item| item == message) {
                    self.diagnostics.push(message.to_owned());
                }
                return;
            }
            self.attempts += 1;
            let dependency = request
                .as_str()
                .ok_or_else(|| invalid("expected a version string"))
                .and_then(|request| self.dependency(package, name, request));
            match dependency {
                Ok((path, manifest)) if self.visited.insert(path.clone()) => {
                    self.dependencies(&path, &manifest, depth + 1);
                }
                Ok(_) => {}
                Err(error) => self.diagnostics.push(format!(
                    "DeepSeek npm dependency {name:?} in {}: {error}",
                    package_label(self.root, package),
                )),
            }
        }
    }

    fn dependency(
        &mut self,
        package: &Path,
        name: &str,
        request: &str,
    ) -> Result<(PathBuf, Map<String, Value>), MarketplaceError> {
        let (target, version) = npm_dependency_request(name, request)?;
        let destination = package.join("node_modules").join(name);
        for directory in package
            .ancestors()
            .take_while(|dir| dir.starts_with(self.root))
        {
            if directory
                .file_name()
                .is_some_and(|name| name == "node_modules")
            {
                continue;
            }
            let candidate = directory.join("node_modules").join(name);
            if !candidate.try_exists()? {
                continue;
            }
            let manifest = read_object(&candidate.join("package.json"))?;
            if matches_package(&manifest, target, version) {
                return Ok((candidate, manifest));
            }
            if candidate == destination {
                return Err(invalid("the bundled copy does not match its declaration"));
            }
            // Only the closest ancestor can satisfy the Node lookup.
            break;
        }
        if version.is_some_and(|version| version.contains(':') || version.starts_with('.')) {
            return Err(invalid(
                "this dependency source needs a bundled node_modules copy",
            ));
        }
        let stage = tempfile::Builder::new()
            .prefix("npm-dependency-")
            .tempdir_in(self.staging)?;
        let source = self
            .installer
            .npm(target, version, self.registry, stage.path())?;
        let manifest = read_object(&source.join("package.json"))?;
        let copy = stage.path().join("package");
        copy_package_with_budget(&source, &copy, &mut self.budget)?;
        fs::create_dir_all(destination.parent().expect("npm package has a parent"))?;
        fs::rename(copy, &destination)?;
        Ok((destination, manifest))
    }
}

fn matches_package(manifest: &Map<String, Value>, target: &str, version: Option<&str>) -> bool {
    manifest.get("name").and_then(Value::as_str) == Some(target)
        && manifest
            .get("version")
            .and_then(Value::as_str)
            .is_some_and(|installed| npm_dependency_version_matches(installed, version))
}

fn package_label(root: &Path, package: &Path) -> String {
    if root == package {
        "the root package".to_owned()
    } else {
        package
            .strip_prefix(root)
            .unwrap_or(package)
            .display()
            .to_string()
    }
}
