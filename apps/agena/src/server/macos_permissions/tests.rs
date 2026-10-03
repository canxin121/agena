use super::*;
use std::{
    io::{BufRead as _, BufReader},
    os::unix::process::CommandExt as _,
    time::{Duration, Instant},
};

fn signed_files(bundle: &Path) -> Vec<Vec<u8>> {
    [
        "Contents/MacOS/Agena",
        "Contents/Info.plist",
        "Contents/_CodeSignature/CodeResources",
    ]
    .iter()
    .map(|path| fs::read(bundle.join(path)).unwrap())
    .collect()
}

#[test]
fn upgrades_preserve_the_signed_host_while_running_the_new_backend() {
    let directory = tempfile::tempdir().unwrap();
    let bundle = directory.path().join("Agena.app");
    assert!(ensure_bundle_at(&bundle).unwrap());
    let original = signed_files(&bundle);
    let backend = directory.path().join("server with spaces");
    for version in ["before", "after"] {
        fs::write(
            &backend,
            format!("#!/bin/sh\nprintf '%s\\n' '{version}' \"$1\"\nexit 23\n"),
        )
        .unwrap();
        fs::set_permissions(&backend, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!ensure_bundle_at(&bundle).unwrap());
        let output = Command::new(executable(&bundle))
            .arg(&backend)
            .arg("literal $value; space & <tag>")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(23));
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{version}\nliteral $value; space & <tag>\n")
        );
        assert_eq!(signed_files(&bundle), original);
    }
}

#[test]
fn damaged_host_is_reported_without_silently_changing_its_identity() {
    let directory = tempfile::tempdir().unwrap();
    let bundle = directory.path().join("Agena.app");
    ensure_bundle_at(&bundle).unwrap();
    let info = bundle.join("Contents/Info.plist");
    let damaged = format!("{}\n", fs::read_to_string(&info).unwrap());
    fs::write(&info, &damaged).unwrap();
    assert!(
        ensure_bundle_at(&bundle)
            .unwrap_err()
            .to_string()
            .contains("signature is invalid")
    );
    assert_eq!(fs::read_to_string(&info).unwrap(), damaged);
}

#[test]
fn host_reports_spawn_errors_and_rejects_relative_executables() {
    let directory = tempfile::tempdir().unwrap();
    let bundle = directory.path().join("Agena.app");
    ensure_bundle_at(&bundle).unwrap();
    for (path, expected) in [("relative-server", 64), ("/nonexistent-agena-backend", 71)] {
        let result = Command::new(executable(&bundle))
            .arg(path)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(expected));
    }
}

#[test]
fn host_forwards_termination_and_reaps_the_server() {
    struct Process(std::process::Child);
    impl Drop for Process {
        fn drop(&mut self) {
            // SAFETY: this test created this child's private process group.
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.wait();
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let bundle = directory.path().join("Agena.app");
    ensure_bundle_at(&bundle).unwrap();
    let mut host = Process(
        Command::new(executable(&bundle))
            .args([
                "/bin/sh",
                "-c",
                "trap 'exit 29' TERM; echo ready; while :; do /bin/sleep 0.05; done",
            ])
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap(),
    );
    let mut ready = String::new();
    BufReader::new(host.0.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready, "ready\n");
    // SAFETY: signal the live supervisor created by this test, not its child.
    assert_eq!(unsafe { libc::kill(host.0.id() as i32, libc::SIGTERM) }, 0);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = host.0.try_wait().unwrap() {
            assert_eq!(status.code(), Some(29));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "host failed to forward SIGTERM/reap child"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
