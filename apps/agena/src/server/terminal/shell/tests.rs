use super::*;

#[test]
fn powershell_loads_profiles_and_shell_paths_remain_single_arguments() {
    let command = interactive_shell_command("C:\\Program Files\\PowerShell\\7\\pwsh.exe", ".");
    assert_eq!(command.get_argv().len(), 2);
    assert_eq!(command.get_argv()[1], "-NoLogo");
    assert!(!command.get_argv().iter().any(|arg| arg == "-NoProfile"));
    let shell = interactive_shell_command("/a folder/zsh", ".");
    let command = tmux_command("fixture", ".", &shell);
    let args = command.get_argv();
    assert!(args.iter().any(|arg| arg == "--"));
    assert!(args.ends_with(shell.get_argv()));
    assert!(command.get_env("TMUX").is_none());
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        io::{Read, Write},
        os::unix::fs::PermissionsExt,
        path::Path,
        sync::mpsc,
        time::{Duration, Instant},
    };

    struct ShellFixture {
        _temp: tempfile::TempDir,
        home: String,
        command: CommandBuilder,
    }

    fn fixture() -> ShellFixture {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("shell home");
        let bin = home.join("custom bin");
        std::fs::create_dir_all(&bin).unwrap();
        let codex = bin.join("codex");
        std::fs::write(&codex, "#!/bin/sh\nprintf 'fixture-codex-found\\n'\n").unwrap();
        std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
        let profile = format!(
            "export PATH='{}':/usr/bin:/bin\nexport AGENA_FIXTURE_PROFILE=login-export\n",
            bin.display().to_string().replace('\'', "'\"'\"'")
        );
        let rc = "export AGENA_FIXTURE_RC=interactive-export\n/bin/stty -echo\n";
        let shell = if Path::new("/bin/zsh").is_file() {
            std::fs::write(home.join(".zprofile"), &profile).unwrap();
            std::fs::write(
                home.join(".zshrc"),
                format!("{rc}PROMPT='AGENA_FIXTURE_READY> '\n"),
            )
            .unwrap();
            "/bin/zsh"
        } else {
            std::fs::write(
                home.join(".bash_profile"),
                format!("{profile}. \"$HOME/.bashrc\"\n"),
            )
            .unwrap();
            std::fs::write(
                home.join(".bashrc"),
                format!("{rc}PS1='AGENA_FIXTURE_READY> '\n"),
            )
            .unwrap();
            "/bin/bash"
        };
        let home = home.to_string_lossy().into_owned();
        let mut command = interactive_shell_command(shell, &home);
        // Simulate a GUI/service launch with only a minimal PATH. Values are
        // applied to this child; the test process's HOME/PATH are untouched.
        command.env_clear();
        command.env("HOME", &home);
        command.env("ZDOTDIR", &home);
        command.env("PATH", "/usr/bin:/bin");
        command.env("SHELL", shell);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        ShellFixture {
            _temp: temp,
            home,
            command,
        }
    }

    fn assert_environment(command: CommandBuilder) {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut writer = pair.master.take_writer().unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let (tx, rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if tx.send(buffer[..count].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut output = String::new();
        let mut sent = false;
        loop {
            let chunk = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
            let Ok(chunk) = chunk else {
                break;
            };
            output.push_str(&String::from_utf8_lossy(&chunk));
            if !sent && output.contains("AGENA_FIXTURE_READY> ") {
                writer.write_all(b"printf 'PROFILE=%s RC=%s\\n' \"$AGENA_FIXTURE_PROFILE\" \"$AGENA_FIXTURE_RC\"; codex; exit\r").unwrap();
                writer.flush().unwrap();
                sent = true;
            }
            if output.contains("fixture-codex-found") {
                break;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        drop(writer);
        drop(pair.master);
        reader.join().unwrap();
        assert!(
            output.contains("PROFILE=login-export RC=interactive-export"),
            "{output}"
        );
        assert!(output.contains("fixture-codex-found"), "{output}");
    }

    #[test]
    fn native_pty_loads_login_and_interactive_exports_and_finds_codex() {
        let fixture = fixture();
        assert_environment(fixture.command);
    }

    #[test]
    fn existing_tmux_server_uses_the_terminal_shell_and_current_startup_environment() {
        assert_tmux_environment(true);
    }

    #[test]
    fn existing_tmux_server_does_not_restore_an_unset_startup_directory() {
        assert_tmux_environment(false);
    }

    fn assert_tmux_environment(keep_zdotdir: bool) {
        if !std::process::Command::new("tmux")
            .arg("-V")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return;
        }
        let mut fixture = fixture();
        if !keep_zdotdir {
            fixture.command.env_remove("ZDOTDIR");
        }
        let socket = format!("agena-test-{}", uuid::Uuid::new_v4());
        struct TmuxCleanup(String);
        impl Drop for TmuxCleanup {
            fn drop(&mut self) {
                let _ = std::process::Command::new("tmux")
                    .args(["-L", &self.0, "kill-server"])
                    .output();
            }
        }
        let _cleanup = TmuxCleanup(socket.clone());
        // Pre-existing server deliberately has the wrong shell/startup path.
        let started = std::process::Command::new("tmux")
            .args([
                "-L",
                &socket,
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "keepalive",
                "/bin/sh",
            ])
            .env("SHELL", "/bin/sh")
            .env("ZDOTDIR", "/nonexistent-agena-startup")
            .output()
            .unwrap();
        assert!(
            started.status.success(),
            "{}",
            String::from_utf8_lossy(&started.stderr)
        );
        let mut command = tmux_command("fixture", &fixture.home, &fixture.command);
        command
            .get_argv_mut()
            .splice(1..1, ["-L".into(), socket.into()]);
        assert_environment(command);
    }
}
