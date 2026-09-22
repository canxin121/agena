use super::tests::{params, settled, until};
use super::*;

fn child_pid(read: &TerminalRead) -> libc::pid_t {
    read.output
        .lines()
        .find_map(|line| {
            line.strip_prefix("CHILD=")
                .and_then(|pid| pid.trim().parse().ok())
        })
        .expect("fixture child pid")
}

fn assert_gone(pid: libc::pid_t) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        // SAFETY: signal zero checks existence without signalling the process.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return;
        }
        assert!(Instant::now() < deadline, "PTY leaked child {pid}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn stop_and_registry_drop_kill_jobs_in_other_process_groups() {
    for drop_registry in [false, true] {
        let registry = TerminalRegistry::default();
        let p = params(
            "import os,signal,time\npid=os.fork()\nif pid==0:\n os.setpgid(0,0)\n signal.signal(signal.SIGTERM,signal.SIG_IGN)\n print('CHILD='+str(os.getpid()),flush=True)\n while True: time.sleep(1)\nwhile True: time.sleep(1)",
        );
        let owner = p.owner.clone();
        let first = registry.start(p, 1000).unwrap();
        let id = &first.summary.process_id;
        let pid = child_pid(&until(&registry, id, &owner, "CHILD="));
        if !drop_registry {
            registry.signal(id, &owner, ShellSignal::Terminate).unwrap();
        }
        drop(registry);
        assert_gone(pid);
    }
}

#[test]
fn parent_exit_cleans_remaining_job_and_retains_final_output() {
    let registry = TerminalRegistry::default();
    let p = params(
        "import os,signal,time\nr,w=os.pipe()\npid=os.fork()\nif pid==0:\n os.close(r)\n os.setpgid(0,0)\n signal.signal(signal.SIGHUP,signal.SIG_IGN)\n print('CHILD='+str(os.getpid()),flush=True)\n os.write(w,b'1')\n while True: time.sleep(1)\nos.close(w)\nos.read(r,1)\nprint('FINAL_OUTPUT',flush=True)\nos._exit(0)",
    );
    let owner = p.owner.clone();
    let first = registry.start(p, 1000).unwrap();
    let id = &first.summary.process_id;
    let done = settled(&registry, id, &owner);
    assert_eq!(done.summary.exit_code, Some(0));
    let history = registry.read(id, &owner, Some(0), 0, None).unwrap();
    assert!(history.output.contains("FINAL_OUTPUT"));
    assert_gone(child_pid(&history));
}

#[test]
fn invalid_cursor_never_delivers_input() {
    let registry = TerminalRegistry::default();
    let p = params("value=input('READY> ');print('DELIVERED='+value,flush=True)");
    let owner = p.owner.clone();
    let first = registry.start(p, 1000).unwrap();
    let id = &first.summary.process_id;
    until(&registry, id, &owner, "READY> ");
    assert!(
        registry
            .write(id, &owner, "unwanted\r", Some(u64::MAX), 0)
            .is_err()
    );
    let history = registry.read(id, &owner, Some(0), 0, None).unwrap();
    assert!(!history.output.contains("unwanted"));
    assert_eq!(history.summary.status, ProcessStatus::Running);
    registry.stop_unscoped(id).unwrap();
}

#[test]
fn cancelled_queued_input_is_not_delivered_and_does_not_kill_terminal() {
    let registry = TerminalRegistry::default();
    let p = params("input('QUEUE_READY> ')");
    let owner = p.owner.clone();
    let first = registry.start(p, 1000).unwrap();
    let id = &first.summary.process_id;
    until(&registry, id, &owner, "QUEUE_READY> ");
    let state = registry.lookup(id, Some(&owner)).unwrap();
    let held = lock(&state.interaction);
    let cancel = CancellationToken::new();
    std::thread::scope(|scope| {
        let writer =
            scope.spawn(|| registry.write_cancellable(id, &owner, "UNWANTED\r", None, 0, &cancel));
        cancel.cancel();
        assert!(writer.join().unwrap().is_err());
    });
    drop(held);
    let history = registry.read(id, &owner, Some(0), 0, None).unwrap();
    assert!(!history.output.contains("UNWANTED"));
    assert_eq!(history.summary.status, ProcessStatus::Running);
    registry.stop_unscoped(id).unwrap();
}

#[test]
fn cancelling_a_wait_is_prompt_and_leaves_terminal_alive() {
    let registry = TerminalRegistry::default();
    let p = params("input('WAIT_READY> ')");
    let owner = p.owner.clone();
    let first = registry.start(p, 1000).unwrap();
    let id = &first.summary.process_id;
    until(&registry, id, &owner, "WAIT_READY> ");
    registry.read(id, &owner, None, 0, None).unwrap();
    let cancel = CancellationToken::new();
    let started = Instant::now();
    std::thread::scope(|scope| {
        let reader =
            scope.spawn(|| registry.read_cancellable(id, &owner, None, 30_000, None, &cancel));
        std::thread::sleep(Duration::from_millis(50));
        cancel.cancel();
        assert!(reader.join().unwrap().is_err());
    });
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        registry
            .read(id, &owner, None, 0, None)
            .unwrap()
            .summary
            .status,
        ProcessStatus::Running
    );
    registry.stop_unscoped(id).unwrap();
}

#[test]
fn cancelled_launch_cannot_leave_an_unclaimed_process() {
    let registry = TerminalRegistry::default();
    let p = params("import time;time.sleep(30)");
    let owner = p.owner.clone();
    let cancel = CancellationToken::new();
    std::thread::scope(|scope| {
        let launcher = scope.spawn(|| registry.start_cancellable(p, 30_000, cancel.clone()));
        let deadline = Instant::now() + Duration::from_secs(2);
        while registry.list().is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        cancel.cancel();
        assert!(launcher.join().unwrap().is_err());
    });
    let id = registry.list().pop().unwrap().process_id;
    assert_eq!(
        settled(&registry, &id, &owner).summary.status,
        ProcessStatus::Stopped
    );
}

#[test]
fn explicit_shutdown_closes_admission_even_when_registry_is_retained() {
    let registry = TerminalRegistry::default();
    let p = params("input('SHUTDOWN_READY> ')");
    let owner = p.owner.clone();
    let first = registry.start(p, 1000).unwrap();
    registry.shutdown();
    registry.shutdown();
    assert!(registry.start(params("must not execute"), 0).is_err());
    let done = settled(&registry, &first.summary.process_id, &owner);
    assert_eq!(done.summary.status, ProcessStatus::Stopped, "{done:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completion_observer_can_spawn_on_the_owning_runtime() {
    #[derive(Debug)]
    struct Listener(tokio::sync::mpsc::UnboundedSender<()>);
    impl MonitorListener for Listener {
        fn on_finished(&self, _: &ProcessSummary) {
            let tx = self.0.clone();
            tokio::spawn(async move {
                tx.send(()).unwrap();
            });
        }
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut registry = TerminalRegistry::default();
    registry.set_listener(Arc::new(Listener(tx)));
    registry.start(params("print('done')"), 0).unwrap();
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn real_terminal_query_receives_virtual_cursor_reply() {
    let registry = TerminalRegistry::default();
    let p = params(
        "import os,sys,tty\ntty.setraw(0)\nos.write(1,b'\\x1b[3;9H\\x1b[6n')\nreply=b''\nwhile not reply.endswith(b'R'): reply+=os.read(0,1)\nprint('REPLY='+reply.hex(),flush=True)",
    );
    let owner = p.owner.clone();
    let first = registry.start(p, 1000).unwrap();
    let id = &first.summary.process_id;
    until(&registry, id, &owner, "REPLY=1b5b333b3952");
    assert_eq!(settled(&registry, id, &owner).summary.exit_code, Some(0));
}

#[test]
fn full_screen_curses_application_handles_arrow_enter_and_exit() {
    let registry = TerminalRegistry::default();
    let p = params(
        "import curses\ndef main(s):\n s.keypad(True)\n selected=0\n while True:\n  s.erase();s.addstr(0,0,'MENU_READY');s.addstr(1,0,'SELECTED='+['one','two'][selected]);s.refresh()\n  key=s.getch()\n  if key==curses.KEY_DOWN: selected=1\n  elif key in (10,13):\n   s.addstr(2,0,'CONFIRMED');s.refresh()\n  elif key==ord('q'): break\ncurses.wrapper(main)\nprint('CURSES_EXITED',flush=True)",
    );
    let owner = p.owner.clone();
    let first = registry.start(p, 1000).unwrap();
    let id = &first.summary.process_id;
    until(&registry, id, &owner, "MENU_READY");
    registry.write(id, &owner, "\x1bOB", None, 100).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let read = registry.read(id, &owner, None, 100, None).unwrap();
        if read.screen.text.contains("SELECTED=two") {
            break;
        }
        assert!(Instant::now() < deadline, "arrow not handled: {read:?}");
    }
    registry.write(id, &owner, "\r", None, 100).unwrap();
    assert!(
        until(&registry, id, &owner, "CONFIRMED")
            .screen
            .alternate_screen
    );
    registry.write(id, &owner, "q", None, 100).unwrap();
    let done = settled(&registry, id, &owner);
    assert_eq!(done.summary.exit_code, Some(0));
    assert!(!done.screen.alternate_screen);
}

#[test]
fn interactive_bash_preserves_environment_after_interrupting_its_foreground_job() {
    let registry = TerminalRegistry::default();
    let mut p = params("interactive bash fixture");
    p.command = vec![
        "/bin/bash".into(),
        "--noprofile".into(),
        "--norc".into(),
        "-i".into(),
    ];
    p.env.insert("PS1".into(), "BASH_READY> ".into());
    let owner = p.owner.clone();
    let first = registry.start(p, 1000).unwrap();
    let id = &first.summary.process_id;
    until(&registry, id, &owner, "BASH_READY> ");
    registry
        .write(
            id,
            &owner,
            "export AGENA_PTY_VALUE=hello; printf 'STATE:%s\\n' \"$AGENA_PTY_VALUE\"\r",
            None,
            100,
        )
        .unwrap();
    until(&registry, id, &owner, "STATE:hello");
    // The readiness text is assembled by the foreground job, so echoed input
    // cannot masquerade as evidence that bash has handed over job control.
    registry
        .write(
            id,
            &owner,
            "sh -c 'printf \"JOB_%s\\n\" READY; exec sleep 30'\r",
            None,
            100,
        )
        .unwrap();
    until(&registry, id, &owner, "JOB_READY");
    registry.signal(id, &owner, ShellSignal::Interrupt).unwrap();
    registry
        .write(
            id,
            &owner,
            "printf 'AFTER:%s\\n' \"$AGENA_PTY_VALUE\"\r",
            None,
            100,
        )
        .unwrap();
    let read = until(&registry, id, &owner, "AFTER:hello");
    assert_eq!(read.summary.status, ProcessStatus::Running);
    registry.write(id, &owner, "exit 0\r", None, 100).unwrap();
    assert_eq!(settled(&registry, id, &owner).summary.exit_code, Some(0));
}

#[test]
fn ending_one_agena_session_leaves_other_sessions_terminal_running() {
    let registry = TerminalRegistry::default();
    let a = params("input('A_READY> ')");
    let mut b = params("input('B_READY> ')");
    b.owner.session_id = Some(42);
    let owner_a = a.owner.clone();
    let owner_b = b.owner.clone();
    let a = registry.start(a, 1000).unwrap();
    let b = registry.start(b, 1000).unwrap();
    registry.stop_session(41);
    assert_eq!(
        settled(&registry, &a.summary.process_id, &owner_a)
            .summary
            .status,
        ProcessStatus::Stopped
    );
    assert_eq!(
        registry
            .read(&b.summary.process_id, &owner_b, None, 0, None)
            .unwrap()
            .summary
            .status,
        ProcessStatus::Running
    );
    registry.stop_unscoped(&b.summary.process_id).unwrap();
}
