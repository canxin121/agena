//! Real native-backend smoke test: cargo run -p agena-process --example sandbox_smoke
//! This intentionally fails when the host cannot enforce MXC isolation.
use agena_process::sandbox::{SandboxPolicy, try_run_launcher, wrap_argv};
use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(code) = try_run_launcher() {
        std::process::exit(code);
    }
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--probe") => return probe(&args),
        Some("--sleep") => {
            std::thread::sleep(Duration::from_secs(5));
            return Ok(());
        }
        Some("--delayed-write") => {
            std::thread::sleep(Duration::from_millis(400));
            std::fs::write(&args[2], b"orphan")?;
            return Ok(());
        }
        Some("--background") => {
            std::process::Command::new(std::env::current_exe()?)
                .arg("--delayed-write")
                .arg(&args[2])
                .spawn()?;
            return Ok(());
        }
        _ => {}
    }
    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    for name in ["input", "output", "secret"] {
        std::fs::create_dir(root.path().join(name))?;
    }
    std::fs::write(root.path().join("input/data"), b"input-data")?;
    std::fs::write(root.path().join("secret/key"), b"secret-sentinel")?;
    std::fs::write(outside.path().join("hidden"), b"outside-sentinel")?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let policy = SandboxPolicy {
        readonly_paths: vec![root.path().to_path_buf()],
        readwrite_paths: vec![root.path().join("output")],
        denied_paths: vec![root.path().join("secret")],
        timeout_ms: Some(5_000),
        ..Default::default()
    };
    let output = invoke(
        &[
            "--probe".into(),
            root.path().display().to_string(),
            outside.path().join("hidden").display().to_string(),
            listener.local_addr()?.port().to_string(),
        ],
        &root.path().join("output"),
        &policy,
    )?;
    if !output.status.success() {
        return Err(format!(
            "native sandbox probe failed: status={} stdout={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        )
        .into());
    }
    assert_eq!(
        std::fs::read(root.path().join("input/data"))?,
        b"input-data"
    );
    assert_eq!(
        std::fs::read(root.path().join("output/result"))?,
        b"output-data"
    );
    println!(
        "PASS: input read, input write denied, output write, secret mask, external-path denial, host-loopback denial, clean environment/private home"
    );

    let mut timed = policy.clone();
    timed.timeout_ms = Some(100);
    let started = Instant::now();
    let output = invoke(&["--sleep".into()], &root.path().join("output"), &timed)?;
    assert_eq!(
        output.status.code(),
        Some(124),
        "timeout stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    println!("PASS: timeout terminates workload and reports status 124");

    let marker = root.path().join("output/orphan");
    let output = invoke(
        &["--background".into(), marker.display().to_string()],
        &root.path().join("output"),
        &policy,
    )?;
    assert!(
        output.status.success(),
        "descendant stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::thread::sleep(Duration::from_millis(600));
    assert!(
        !marker.exists(),
        "background descendant survived sandbox completion"
    );
    println!("PASS: detached workload descendants are removed at completion");
    Ok(())
}

fn invoke(args: &[String], cwd: &Path, policy: &SandboxPolicy) -> io::Result<std::process::Output> {
    let mut argv = vec![std::env::current_exe()?.display().to_string()];
    argv.extend_from_slice(args);
    let mut environment: HashMap<String, String> = std::env::vars().collect();
    environment.insert(
        "AGENA_SMOKE_SECRET".to_owned(),
        "must-not-be-inherited".to_owned(),
    );
    let argv = wrap_argv(&argv, cwd, &mut environment, policy)?;
    std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(cwd)
        .env_clear()
        .envs(environment)
        .output()
}

fn probe(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(&args[2]);
    assert_eq!(std::fs::read(root.join("input/data"))?, b"input-data");
    assert!(
        std::fs::write(root.join("input/data"), b"changed").is_err(),
        "readonly input was writable"
    );
    std::fs::write(root.join("output/result"), b"output-data")?;
    assert_ne!(
        std::fs::read(root.join("secret/key")).ok().as_deref(),
        Some(b"secret-sentinel".as_slice()),
        "denied secret was readable"
    );
    assert_ne!(
        std::fs::read(&args[3]).ok().as_deref(),
        Some(b"outside-sentinel".as_slice()),
        "external file was readable"
    );
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), args[4].parse()?);
    assert!(
        TcpStream::connect_timeout(&address, Duration::from_millis(400)).is_err(),
        "host loopback was reachable"
    );
    assert!(
        std::env::var_os("AGENA_SMOKE_SECRET").is_none(),
        "ambient secret environment leaked"
    );
    let home = PathBuf::from(std::env::var("HOME")?);
    assert!(
        !home.starts_with(&root),
        "private home overlaps the input/output fixture"
    );
    std::fs::write(home.join("private-cache"), b"private")?;
    Ok(())
}
