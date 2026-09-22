use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) fn params(script: &str) -> TerminalStartParams {
    let workdir = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    TerminalStartParams {
        process_id: None,
        owner: TerminalOwner {
            workspace: workdir.clone(),
            session_id: Some(41),
        },
        command: vec![
            "/usr/bin/python3".into(),
            "-u".into(),
            "-c".into(),
            script.into(),
        ],
        display_command: script.into(),
        description: "terminal regression".into(),
        workdir,
        env: std::env::vars()
            .chain([("TERM".into(), "xterm-256color".into())])
            .collect(),
        rows: 24,
        cols: 80,
        timeout_ms: Some(10_000),
    }
}

#[cfg(unix)]
pub(super) fn until(
    registry: &TerminalRegistry,
    id: &str,
    owner: &TerminalOwner,
    needle: &str,
) -> TerminalRead {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let read = registry.read(id, owner, Some(0), 100, None).unwrap();
        if read.output.contains(needle) {
            return read;
        }
        assert!(
            read.summary.status == ProcessStatus::Running && Instant::now() < deadline,
            "missing {needle:?}, terminal was {read:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
pub(super) fn settled(
    registry: &TerminalRegistry,
    id: &str,
    owner: &TerminalOwner,
) -> TerminalRead {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let read = registry.read(id, owner, None, 100, None).unwrap();
        if read.summary.status != ProcessStatus::Running {
            return read;
        }
        assert!(
            Instant::now() < deadline,
            "terminal failed to settle: {read:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn interactive_input_enter_ctrl_c_and_eof_across_calls() {
    let registry = TerminalRegistry::default();
    let p = params(
        "import sys\nprint('TTY='+str(sys.stdin.isatty()), flush=True)\nwhile True:\n try: value=input('PROMPT> ')\n except KeyboardInterrupt:\n  print('INTERRUPTED', flush=True); continue\n except EOFError:\n  print('EOF', flush=True); break\n print('VALUE='+repr(value), flush=True)",
    );
    let owner = p.owner.clone();
    let first = registry.start(p, 1000).unwrap();
    let id = &first.summary.process_id;
    until(&registry, id, &owner, "PROMPT> ");
    let typed = registry.write(id, &owner, " 你好 ", None, 100).unwrap();
    assert!(!typed.output.contains("VALUE="));
    registry.write(id, &owner, "\r", None, 100).unwrap();
    let read = until(&registry, id, &owner, "VALUE=' 你好 '");
    assert!(read.output.contains("TTY=True"));
    registry.write(id, &owner, "\x03", None, 100).unwrap();
    until(&registry, id, &owner, "INTERRUPTED");
    registry.write(id, &owner, "next\r", None, 100).unwrap();
    until(&registry, id, &owner, "VALUE='next'");
    registry.write(id, &owner, "\x04", None, 100).unwrap();
    let done = settled(&registry, id, &owner);
    assert_eq!(done.summary.status, ProcessStatus::Exited);
    assert_eq!(done.summary.exit_code, Some(0));
    let history = registry.read(id, &owner, Some(0), 0, None).unwrap();
    assert!(history.output.contains("EOF"));
    assert!(registry.write(id, &owner, "again", None, 0).is_err());
    assert!(registry.write(id, &owner, "", None, 0).is_ok());
}

#[cfg(unix)]
#[test]
fn resize_updates_kernel_size_and_screen() {
    let registry = TerminalRegistry::default();
    let p = params(
        "import os,sys,signal,time\ndef report(*args):\n s=os.get_terminal_size(); print('SIZE=%sx%s'%(s.columns,s.lines),flush=True)\nsignal.signal(signal.SIGWINCH,report)\nreport()\nwhile True: time.sleep(1)",
    );
    let owner = p.owner.clone();
    let first = registry.start(p, 1000).unwrap();
    let id = &first.summary.process_id;
    until(&registry, id, &owner, "SIZE=80x24");
    let resized = registry.resize(id, &owner, 35, 100).unwrap();
    assert_eq!((resized.screen.rows, resized.screen.cols), (35, 100));
    until(&registry, id, &owner, "SIZE=100x35");
    registry.stop_unscoped(id).unwrap();
}

#[cfg(unix)]
#[test]
fn signal_interrupt_reaches_raw_mode_foreground_without_closing_session() {
    let registry = TerminalRegistry::default();
    let p = params(
        "import sys,tty,signal,time\ntty.setraw(sys.stdin.fileno())\nsignal.signal(signal.SIGINT,lambda *args: print('GOT_SIGINT',flush=True))\nprint('RAW_READY',flush=True)\nwhile True: time.sleep(1)",
    );
    let owner = p.owner.clone();
    let first = registry.start(p, 1000).unwrap();
    let id = &first.summary.process_id;
    until(&registry, id, &owner, "RAW_READY");
    registry.signal(id, &owner, ShellSignal::Interrupt).unwrap();
    assert_eq!(
        until(&registry, id, &owner, "GOT_SIGINT").summary.status,
        ProcessStatus::Running
    );
    registry.signal(id, &owner, ShellSignal::Kill).unwrap();
}

#[cfg(unix)]
#[test]
fn quiet_prompt_is_running_until_explicit_lifetime_timeout() {
    let registry = TerminalRegistry::default();
    let mut p = params("input('NO_NEWLINE> ')");
    p.timeout_ms = Some(300);
    let owner = p.owner.clone();
    let first = registry.start(p, 0).unwrap();
    let id = &first.summary.process_id;
    let prompt = until(&registry, id, &owner, "NO_NEWLINE> ");
    assert_eq!(prompt.summary.status, ProcessStatus::Running);
    assert_eq!(
        settled(&registry, id, &owner).summary.status,
        ProcessStatus::TimedOut
    );
}

#[cfg(unix)]
#[test]
fn other_sessions_and_workspaces_cannot_read_or_control_a_terminal() {
    let registry = TerminalRegistry::default();
    let p = params("input('OWNED> ')");
    let owner = p.owner.clone();
    let first = registry.start(p, 1000).unwrap();
    let id = &first.summary.process_id;
    for foreign in [
        TerminalOwner {
            session_id: Some(42),
            ..owner.clone()
        },
        TerminalOwner {
            session_id: None,
            ..owner.clone()
        },
        TerminalOwner {
            workspace: PathBuf::from("/a-different-workspace"),
            ..owner.clone()
        },
    ] {
        assert!(!registry.is_owned(id, &foreign));
        assert!(registry.read(id, &foreign, None, 0, None).is_err());
        assert!(registry.write(id, &foreign, "x\r", None, 0).is_err());
        assert!(registry.resize(id, &foreign, 30, 90).is_err());
        assert!(registry.signal(id, &foreign, ShellSignal::Kill).is_err());
    }
    registry.stop_unscoped(id).unwrap();
}

#[cfg(unix)]
#[test]
fn launch_replay_is_idempotent_and_exit_notifies_once() {
    #[derive(Debug, Default)]
    struct Listener {
        starts: AtomicUsize,
        finishes: AtomicUsize,
    }
    impl MonitorListener for Listener {
        fn on_started(&self, _: &ProcessSummary) {
            self.starts.fetch_add(1, Ordering::SeqCst);
        }
        fn on_finished(&self, _: &ProcessSummary) {
            self.finishes.fetch_add(1, Ordering::SeqCst);
        }
    }
    let listener = Arc::new(Listener::default());
    let mut registry = TerminalRegistry::default();
    registry.set_listener(listener.clone());
    let mut p = params("input('ONCE> ')");
    p.process_id = Some("pty-idempotent".into());
    let first = registry.start(p.clone(), 1000).unwrap();
    let second = registry.start(p, 0).unwrap();
    assert_eq!(first.summary.process_id, second.summary.process_id);
    registry.stop_unscoped(&first.summary.process_id).unwrap();
    registry.stop_unscoped(&first.summary.process_id).unwrap();
    assert_eq!(listener.starts.load(Ordering::SeqCst), 1);
    // Finish notification is emitted after state publication, so synchronize
    // on the callback rather than racing the driver's final instructions.
    let deadline = Instant::now() + Duration::from_secs(1);
    while listener.finishes.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(listener.finishes.load(Ordering::SeqCst), 1);
}

#[cfg(unix)]
#[test]
fn blocked_writer_is_bounded_and_terminal_is_terminated() {
    let registry = TerminalRegistry::default();
    let p = params(
        "import tty,sys,time\ntty.setraw(sys.stdin.fileno())\nprint('BLOCKED_READY',flush=True)\ntime.sleep(30)",
    );
    let owner = p.owner.clone();
    let first = registry.start(p, 1000).unwrap();
    let id = &first.summary.process_id;
    until(&registry, id, &owner, "BLOCKED_READY");
    let result = registry.write(id, &owner, &"x".repeat(MAX_INPUT_BYTES), None, 0);
    assert!(
        result.is_err(),
        "a non-reading raw terminal must apply backpressure"
    );
    let done = settled(&registry, id, &owner);
    assert_ne!(done.summary.status, ProcessStatus::Running);
}

#[test]
fn utf8_partial_prompt_and_ansi_screen_are_preserved() {
    let (tx, _rx) = mpsc::sync_channel(1);
    let state = State::new("unit".into(), &params("unit"), tx, None);
    state.append(b"\xe4");
    state.append(b"\xbd");
    state.append(b"\xa0> ");
    let first = state.read(None, 0, None).unwrap();
    assert_eq!(first.output, "你> ");
    assert!(first.screen.text.contains("你> "));
    assert!(state.read(None, 0, None).unwrap().output.is_empty());
    state.append(b"\x1b[2J\x1b[Hone\r\ntwo\x1b[1;1HZ");
    let read = state.read(None, 0, None).unwrap();
    assert_eq!(read.screen.text, "Zne\ntwo");
    assert_eq!((read.screen.cursor_row, read.screen.cursor_col), (0, 1));
    state.append(b"\x1b[?1049hALT");
    assert!(state.read(None, 0, None).unwrap().screen.alternate_screen);
    state.append(b"\x1b[?1049l");
    assert_eq!(state.read(None, 0, None).unwrap().screen.text, "Zne\ntwo");
}

#[test]
fn noisy_output_has_bounded_capture_paging_and_explicit_drop_counts() {
    let (tx, _rx) = mpsc::sync_channel(1);
    let state = State::new("noisy".into(), &params("unit"), tx, None);
    for _ in 0..400 {
        state.append(&[b'x'; 4096]);
    }
    let first = state.read(Some(0), 0, None).unwrap();
    assert!(first.dropped_bytes > 0 && first.summary.dropped_lines > 0);
    assert!(first.output.len() <= MAX_OUTPUT_BYTES);
    assert!(first.has_more);
    let next = state.read(Some(first.last_seq), 0, None).unwrap();
    assert!(next.last_seq > first.last_seq);
    assert!(state.read(Some(u64::MAX), 0, None).is_err());
}

#[test]
fn malformed_dimensions_waits_and_oversized_inputs_fail_before_spawn() {
    let registry = TerminalRegistry::default();
    for (rows, cols) in [(0, 80), (24, 0), (201, 80), (200, 400)] {
        let mut p = params("must not execute");
        p.rows = rows;
        p.cols = cols;
        assert!(registry.start(p, 0).is_err());
    }
    assert!(registry.start(params("must not execute"), 30_001).is_err());
    let owner = params("unit").owner;
    assert!(
        registry
            .write("unknown", &owner, &"x".repeat(MAX_INPUT_BYTES + 1), None, 0)
            .is_err()
    );
    assert!(registry.list().is_empty());
}
