use std::{
    fs::{self, File},
    io::{self, Read},
    path::Path,
    process::Stdio,
    time::Duration,
};

use agena_plugin_marketplace::agent_packages::npm_selector;
use serde_json::{Map, Value};

const LOCKFILE_LIMIT: u64 = 16 * 1024 * 1024;
const DEPENDENCY_FIELDS: [&str; 3] = ["dependencies", "optionalDependencies", "peerDependencies"];

/// Check the lockfile and install locked Claude plugin dependencies when requested.
pub(super) fn prepare(package: &Path, preview: bool) -> anyhow::Result<()> {
    if !package.join("package.json").is_file() || package.join("node_modules").is_dir() {
        return Ok(());
    }
    let manifest: Map<String, Value> =
        serde_json::from_slice(&read(&package.join("package.json"))?)?;
    let mut input = Map::new();
    for field in [
        "name",
        "version",
        "os",
        "cpu",
        "libc",
        "peerDependenciesMeta",
    ] {
        if let Some(value) = manifest.get(field) {
            input.insert(field.into(), value.clone());
        }
    }
    let mut has_dependencies = false;
    for field in DEPENDENCY_FIELDS {
        if let Some(value) = manifest.get(field) {
            let entries = value
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("{field} must be an object"))?;
            validate_dependencies(value)?;
            has_dependencies |= !entries.is_empty();
            input.insert(field.into(), value.clone());
        }
    }
    if !has_dependencies {
        return Ok(());
    }
    if let Some(dev) = manifest.get("devDependencies") {
        validate_dependencies(dev)?;
        input.insert("devDependencies".into(), dev.clone());
    }

    let (lockfile, bun) = [
        ("bun.lock", true),
        ("npm-shrinkwrap.json", false),
        ("package-lock.json", false),
    ]
    .into_iter()
    .find(|(name, _)| package.join(name).is_file())
    .ok_or_else(|| anyhow::anyhow!("runtime dependencies require an npm or text Bun lockfile"))?;
    let lock = read(&package.join(lockfile))?;
    if bun {
        anyhow::ensure!(
            !manifest.contains_key("patchedDependencies"),
            "Bun dependency patches are not supported"
        );
        inspect_bun(&json5::from_str(std::str::from_utf8(&lock)?)?)?;
    } else {
        anyhow::ensure!(
            !manifest.contains_key("overrides"),
            "npm dependency overrides are not supported"
        );
        inspect_npm(&serde_json::from_slice(&lock)?)?;
    }

    if preview {
        return Ok(());
    }
    let staging = tempfile::Builder::new().prefix("node-deps-").tempdir_in(
        package
            .parent()
            .ok_or_else(|| anyhow::anyhow!("package has no staging directory"))?,
    )?;
    let work = staging.path();
    fs::write(work.join("package.json"), serde_json::to_vec(&input)?)?;
    fs::write(work.join(lockfile), lock)?;
    let mut command = if bun {
        let mut command = tokio::process::Command::new("bun");
        let config = work.join("bunfig.toml");
        File::create(&config)?;
        command.args([
            "install",
            "--production",
            "--frozen-lockfile",
            "--ignore-scripts",
            "--linker",
            "hoisted",
            "--backend",
            "copyfile",
        ]);
        let mut config_arg = std::ffi::OsString::from("--config=");
        config_arg.push(config);
        command.arg(config_arg);
        command
    } else {
        let mut command =
            tokio::process::Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" });
        let user_config = work.join("user-npmrc");
        let global_config = work.join("global-npmrc");
        File::create(&user_config)?;
        File::create(&global_config)?;
        command
            .args([
                "ci",
                "--omit=dev",
                "--ignore-scripts",
                "--no-audit",
                "--no-fund",
                "--global=false",
            ])
            .arg("--cache")
            .arg(work.join("cache"))
            .arg("--userconfig")
            .arg(user_config)
            .arg("--globalconfig")
            .arg(global_config);
        command
    };
    // The staging directory contains only the validated manifests. Keep login
    // tokens and package-manager options from the service out of this process.
    command.env_clear().current_dir(work).stdin(Stdio::null());
    if bun {
        command.env("BUN_INSTALL_CACHE_DIR", work.join("cache"));
    }
    for name in ["PATH", "SystemRoot", "WINDIR", "ComSpec"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let output = tokio::runtime::Handle::current().block_on(agena_process::output(
        command,
        Duration::from_secs(60),
        64 * 1024,
    ))?;
    let details = String::from_utf8_lossy(&output.stderr);
    anyhow::ensure!(
        output.status.success(),
        "{} dependency install exited with {}. {}",
        if bun { "Bun" } else { "npm" },
        output.status,
        details.trim().chars().take(1_024).collect::<String>()
    );
    let modules = work.join("node_modules");
    if modules.is_dir() {
        anyhow::ensure!(!modules.is_symlink(), "node_modules must be a directory");
        fs::rename(modules, package.join("node_modules"))?;
    }
    Ok(())
}

fn read(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(LOCKFILE_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LOCKFILE_LIMIT {
        return Err(io::Error::other(format!("{} is too large", path.display())));
    }
    Ok(bytes)
}

fn validate_dependencies(value: &Value) -> anyhow::Result<()> {
    let entries = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("invalid dependency list"))?;
    for (name, version) in entries {
        anyhow::ensure!(
            npm_selector(name).is_ok_and(|(_, version)| version.is_none()),
            "invalid dependency name {name:?}"
        );
        let version = version
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("invalid version for {name}"))?;
        let range = if let Some(alias) = version.strip_prefix("npm:") {
            npm_selector(alias)?.1.unwrap_or("*")
        } else {
            version
        };
        anyhow::ensure!(
            range
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b" ^~<>=.*-+|".contains(&byte)),
            "{name} needs a registry dependency"
        );
    }
    Ok(())
}

fn https_resolution(value: &Value) -> anyhow::Result<()> {
    let url = value
        .as_str()
        .and_then(|url| url::Url::parse(url).ok())
        .filter(|url| {
            url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
        });
    anyhow::ensure!(
        url.is_some(),
        "dependency lockfile requires HTTPS registry downloads"
    );
    Ok(())
}

fn inspect_npm(lock: &Value) -> anyhow::Result<()> {
    anyhow::ensure!(
        matches!(lock["lockfileVersion"].as_u64(), Some(2 | 3)),
        "unsupported npm lockfile version"
    );
    let packages = lock["packages"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("npm lockfile needs a packages map"))?;
    anyhow::ensure!(
        packages.contains_key(""),
        "npm lockfile has no root package"
    );
    for (path, package) in packages {
        for field in DEPENDENCY_FIELDS {
            if let Some(entries) = package.get(field) {
                validate_dependencies(entries)?;
            }
        }
        if path.is_empty() {
            continue;
        }
        anyhow::ensure!(
            path.starts_with("node_modules/")
                && !path
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
                && !path.contains(['\\', ':', '\0'])
                && package["link"] != true,
            "npm lockfile package {path:?} needs a registry dependency"
        );
        anyhow::ensure!(
            package["version"]
                .as_str()
                .is_some_and(|version| semver::Version::parse(version).is_ok()),
            "npm lockfile package {path:?} needs an exact version"
        );
        if let Some(resolved) = package.get("resolved") {
            https_resolution(resolved)?;
        }
    }
    Ok(())
}

fn inspect_bun(lock: &Value) -> anyhow::Result<()> {
    anyhow::ensure!(
        matches!(lock["lockfileVersion"].as_u64(), Some(0..=2)),
        "unsupported Bun lockfile version"
    );
    let workspaces = lock["workspaces"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Bun lockfile needs a root workspace"))?;
    anyhow::ensure!(
        workspaces.len() == 1 && workspaces.contains_key(""),
        "Bun lockfile must describe a single package"
    );
    anyhow::ensure!(
        lock.get("patchedDependencies").is_none(),
        "Bun lockfile contains patches"
    );
    let packages = lock["packages"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Bun lockfile needs a packages map"))?;
    for (name, package) in packages {
        let resolution = package[0]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("invalid Bun lockfile entry for {name}"))?;
        let (_, version) = npm_selector(resolution)?;
        anyhow::ensure!(
            version.is_some_and(|version| semver::Version::parse(version).is_ok()),
            "Bun lockfile package {name:?} needs an exact version"
        );
        if let Some(remote) = package.get(1)
            && remote != ""
        {
            https_resolution(remote)?;
        }
        for field in DEPENDENCY_FIELDS {
            if let Some(entries) = package[2].get(field) {
                validate_dependencies(entries)?;
            }
        }
    }
    Ok(())
}
