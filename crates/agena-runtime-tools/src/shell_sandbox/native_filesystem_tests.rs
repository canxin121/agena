use super::*;

fn executor(workspace: &Path, permission: serde_json::Value) -> ToolExecutor {
    ToolExecutor::new(
        workspace,
        crate::authorization::ExecutionPrincipal::new(
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        ),
        agena_plugin_host::PluginHost::new_empty(),
        None,
        None,
    )
    .with_permission_config(&serde_json::from_value(permission).unwrap())
}

fn check_access(executor: &ToolExecutor, cwd: &Path, cases: &[(&Path, &str)]) {
    let script = "set -eu\nfor target do\nif cat \"$target\" >/dev/null 2>&1; then r=1; else r=0; fi\nif (printf checked >> \"$target\") 2>/dev/null; then w=1; else w=0; fi\nprintf '%s%s\\n' \"$r\" \"$w\"\ndone";
    let input: ShellCommandInput = serde_json::from_value(serde_json::json!({
        "command": script, "workdir": cwd, "timeout_ms": 20_000
    }))
    .unwrap();
    assert!(input.reads.is_empty() && input.writes.is_empty() && input.network.is_empty());
    let mut environment = HashMap::new();
    if let Ok(path) = std::env::var("PATH") {
        environment.insert("PATH".into(), path);
    }
    let mut argv = vec![
        "/bin/sh".into(),
        "-c".into(),
        script.into(),
        "agena-native-permissions".into(),
    ];
    argv.extend(
        cases
            .iter()
            .map(|(path, _)| path.to_str().unwrap().to_owned()),
    );
    let argv = protect(executor, argv, cwd, &input, &mut environment)
        .unwrap_or_else(|error| panic!("native policy for {cases:?}: {error}"));
    let output = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(cwd)
        .env_clear()
        .envs(environment)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "native sandbox for {cases:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: Vec<_> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    let expected: Vec<_> = cases
        .iter()
        .map(|(_, mode)| match *mode {
            "readwrite" => "11",
            "readonly" => "10",
            "deny" => "00",
            _ => panic!("unknown fixture mode"),
        })
        .collect();
    assert_eq!(actual, expected, "access mismatch for {cases:?}");
}

fn file(path: &Path) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "permission fixture\n").unwrap();
    path.to_owned()
}

#[test]
#[ignore = "requires AGENA_TEST_SANDBOX_LAUNCHER and native MXC prerequisites"]
fn native_filesystem_defaults_and_nested_rules_match_saved_permissions() {
    let launcher = PathBuf::from(
        std::env::var_os("AGENA_TEST_SANDBOX_LAUNCHER")
            .expect("set AGENA_TEST_SANDBOX_LAUNCHER to an existing Agena executable"),
    );
    agena_process::sandbox::configure_launcher(&launcher).unwrap();
    // Ordinary outside-workspace data must be outside both the host's shared
    // temp root and the launcher's protected installation directory.
    let fixture = tempfile::Builder::new()
        .prefix("agena-permission-fixture-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    let root = fixture.path().canonicalize().unwrap();
    let control = root.join("control");
    std::fs::create_dir(&control).unwrap();
    let outside = file(&root.join("outside/data.txt"));
    let private = tempfile::Builder::new()
        .prefix("agena-sandbox-other-test-")
        .tempdir()
        .unwrap();
    let other_task = file(&private.path().join("workload.json"));
    for workspace_mode in ["deny", "readonly", "readwrite"] {
        for external_mode in ["deny", "readonly", "readwrite"] {
            let workspace = root.join(format!("workspace-{workspace_mode}-{external_mode}"));
            let data = file(&workspace.join("data.txt"));
            let executor = executor(
                &workspace,
                serde_json::json!({
                    "path": {
                        "workspace": workspace_mode,
                        "external": external_mode,
                        "rules": {control.to_str().unwrap(): "readonly"}
                    },
                    "network": {"default": "deny"}
                }),
            );
            assert!(executor.sandbox_enabled());
            check_access(
                &executor,
                &control,
                &[
                    (&data, workspace_mode),
                    (&outside, external_mode),
                    (&other_task, "deny"),
                ],
            );
        }
    }

    let workspace = root.join("nested-workspace");
    let writable = file(&workspace.join("data.txt"));
    let readonly = file(&workspace.join("reference/data.txt"));
    let reopened = file(&workspace.join("reference/cache/data.txt"));
    let locked = file(&workspace.join("reference/cache/locked/data.txt"));
    let secret = file(&workspace.join("secrets/data.txt"));
    let executor = executor(
        &workspace,
        serde_json::json!({
            "path": {
                "workspace": "readwrite", "external": "readonly",
                "rules": {
                    "<workspace>/reference": "readonly",
                    "<workspace>/reference/cache": "readwrite",
                    "<workspace>/reference/cache/locked": "readonly",
                    "<workspace>/secrets": "deny"
                }
            },
            "network": {"default": "deny"}
        }),
    );
    check_access(
        &executor,
        &workspace,
        &[
            (&writable, "readwrite"),
            (&readonly, "readonly"),
            (&reopened, "readwrite"),
            (&locked, "readonly"),
            (&secret, "deny"),
            (&outside, "readonly"),
            (&other_task, "deny"),
        ],
    );
}
