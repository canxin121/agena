//! Resolve declared Cordis packages and the dependencies staged with them.

use super::{BTreeSet, Map, Path, PathBuf, Url, Value, package_export, resolve_module};
use agena_plugin_marketplace::agent_packages::{
    npm_dependency_request, npm_dependency_version_matches, npm_selector,
};

use crate::agent_plugin::{
    MAX_MANIFEST_BYTES, package_path, read_package_bytes, read_package_object,
};

const MAX_BINARY_PACKAGE_BYTES: u64 = 2 * 1024 * 1024;

pub(super) fn read_bytes(
    root: &Path,
    input: &str,
    watch: &mut BTreeSet<PathBuf>,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    let path = if input.starts_with("file:") {
        Url::parse(input)
            .ok()
            .and_then(|url| url.to_file_path().ok())
            .ok_or("invalid Cordis file URL")?
    } else {
        root.join(input)
    };
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Cordis file is outside its package")?;
    let path = package_path(root, relative)?;
    if watch.len() >= 256 && !watch.contains(&path) {
        return Err("too many Cordis package file reads".into());
    }
    let bytes = read_package_bytes(root, relative, max_bytes)?
        .ok_or("Cordis package file is missing")?;
    watch.insert(path);
    Ok(bytes)
}

pub(super) fn read_text(
    root: &Path,
    input: &str,
    watch: &mut BTreeSet<PathBuf>,
) -> rquickjs::Result<String> {
    let mut read = || -> Result<String, String> {
        let bytes = read_bytes(root, input, watch, agena_skills::MAX_DOCUMENT_BYTES as u64)?;
        String::from_utf8(bytes).map_err(|_| "Cordis package file is not UTF-8".into())
    };
    read().map_err(|error| {
        rquickjs::Error::new_from_js_message("Cordis package file", "UTF-8", error)
    })
}

pub(super) fn read_base64(
    root: &Path,
    input: &str,
    watch: &mut BTreeSet<PathBuf>,
) -> rquickjs::Result<String> {
    use base64::Engine as _;
    read_bytes(root, input, watch, MAX_BINARY_PACKAGE_BYTES)
        .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
        .map_err(|error| {
            rquickjs::Error::new_from_js_message("Cordis package file", "bytes", error)
        })
}

pub(super) fn name(manifest: &Map<String, Value>) -> &str {
    manifest
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

pub(super) fn dependency<'manifest, 'specifier>(
    manifest: &'manifest Map<String, Value>,
    specifier: &'specifier str,
    allow_peer: bool,
) -> Option<(&'specifier str, &'specifier str, &'manifest str)> {
    let (slot, subpath) = package_specifier(specifier)?;
    let mut fields = ["optionalDependencies", "dependencies", "peerDependencies"].into_iter();
    let request = fields
        .by_ref()
        .take(if allow_peer { 3 } else { 2 })
        .find_map(|field| manifest.get(field)?.get(slot))?
        .as_str()?;
    Some((slot, subpath, request))
}

fn package_specifier(specifier: &str) -> Option<(&str, &str)> {
    let slash = specifier.find('/');
    let prefix = if specifier.starts_with('@') {
        slash? + 1
    } else {
        0
    };
    let (slot, subpath) = specifier[prefix..].find('/').map_or((specifier, ""), |i| {
        (&specifier[..prefix + i], &specifier[prefix + i + 1..])
    });
    let (_, version) = npm_selector(slot).ok()?;
    (version.is_none()
        && !subpath.contains(['?', '#', '\\', '\0', '%'])
        && !subpath
            .split('/')
            .any(|part| matches!(part, "." | ".." | "node_modules")))
    .then_some((slot, subpath))
}

fn manifest_at(
    root: &Path,
    package: &Path,
    watch: &mut BTreeSet<PathBuf>,
) -> Result<Option<Map<String, Value>>, String> {
    let path = package.join("package.json");
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Cordis package escapes the bundle")?;
    let manifest = read_package_object(root, relative, MAX_MANIFEST_BYTES)?;
    watch.insert(path);
    Ok(manifest)
}

pub(super) fn resolve_import(
    root: &Path,
    importer: &Path,
    specifier: &str,
    watch: &mut BTreeSet<PathBuf>,
) -> Result<PathBuf, String> {
    let directories = importer
        .ancestors()
        .skip(1)
        .take_while(|path| path.starts_with(root));
    for package in directories {
        if let Some(manifest) = manifest_at(root, package, watch)?
            && !name(&manifest).is_empty()
        {
            return resolve(root, package, &manifest, specifier, true, watch);
        }
    }
    Err("Cordis import has no owning package manifest".into())
}

pub(super) fn resolve(
    root: &Path,
    package: &Path,
    manifest: &Map<String, Value>,
    specifier: &str,
    allow_peer: bool,
    watch: &mut BTreeSet<PathBuf>,
) -> Result<PathBuf, String> {
    let own_name = name(manifest);
    if package_export(own_name, specifier).is_some() {
        return resolve_module(package, manifest, own_name, specifier);
    }
    let (slot, subpath, request) =
        dependency(manifest, specifier, allow_peer).ok_or_else(|| {
            format!("Cordis import {specifier:?} is not a declared package dependency")
        })?;
    let (target, version) =
        npm_dependency_request(slot, request).map_err(|error| error.to_string())?;
    for directory in package
        .ancestors()
        .take_while(|path| path.starts_with(root))
    {
        if directory
            .file_name()
            .is_some_and(|name| name == "node_modules")
        {
            continue;
        }
        let dependency = directory.join("node_modules").join(slot);
        let Some(manifest) = manifest_at(root, &dependency, watch)? else {
            continue;
        };
        if name(&manifest) != target
            || !manifest
                .get("version")
                .and_then(Value::as_str)
                .is_some_and(|installed| npm_dependency_version_matches(installed, version))
        {
            return Err(format!(
                "Cordis dependency {slot:?} does not match {request:?}"
            ));
        }
        let module = if subpath.is_empty() {
            target.to_owned()
        } else {
            format!("{target}/{subpath}")
        };
        return resolve_module(&dependency, &manifest, target, &module);
    }
    Err(format!(
        "Cordis dependency {slot:?} is missing from the installed package"
    ))
}
