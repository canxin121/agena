use super::*;
use agena_process::pty::PtyProcess;
use std::{io, sync::atomic::Ordering};

pub(super) fn run(
    state: &State,
    params: TerminalStartParams,
    controls: mpsc::Receiver<Request>,
    launch_cancel: CancellationToken,
) {
    if state.stopping.load(Ordering::Acquire) != 0 || launch_cancel.is_cancelled() {
        state.finish(ProcessStatus::Stopped, None, "cancelled_before_spawn");
        return;
    }
    let mut process = match PtyProcess::spawn(
        &params.command,
        &params.workdir,
        &params.env,
        params.rows,
        params.cols,
    ) {
        Ok(process) => process,
        Err(error) => {
            state.append(format!("Terminal launch failed: {error}\r\n").as_bytes());
            state.finish(ProcessStatus::Failed, None, "spawn_failed");
            return;
        }
    };
    let deadline = params
        .timeout_ms
        .and_then(|ms| Instant::now().checked_add(Duration::from_millis(ms)));
    loop {
        if launch_cancel.is_cancelled() {
            cleanup(
                state,
                &mut process,
                true,
                ProcessStatus::Stopped,
                "launch_cancelled",
            );
            return;
        }
        let stopping = state.stopping.load(Ordering::Acquire);
        let timed_out = deadline.is_some_and(|deadline| Instant::now() >= deadline);
        if stopping != 0 || timed_out {
            cleanup(
                state,
                &mut process,
                stopping == 2,
                if timed_out {
                    ProcessStatus::TimedOut
                } else {
                    ProcessStatus::Stopped
                },
                if timed_out {
                    "timeout"
                } else {
                    "explicit_stop"
                },
            );
            return;
        }
        if let Err(error) = drain(state, &mut process) {
            state.append(format!("Terminal output failed: {error}\r\n").as_bytes());
            cleanup(
                state,
                &mut process,
                true,
                ProcessStatus::Failed,
                "output_failed",
            );
            return;
        }
        let replies = match state.take_protocol_replies() {
            Ok(replies) => replies,
            Err(error) => {
                state.append(format!("Terminal protocol error: {error}\r\n").as_bytes());
                cleanup(
                    state,
                    &mut process,
                    true,
                    ProcessStatus::Failed,
                    "protocol_overflow",
                );
                return;
            }
        };
        if !replies.is_empty()
            && let Err(error) = write(
                state,
                &mut process,
                &replies,
                Instant::now() + Duration::from_millis(250),
                &launch_cancel,
            )
        {
            state.append(format!("Terminal query reply failed: {error}\r\n").as_bytes());
            cleanup(
                state,
                &mut process,
                true,
                ProcessStatus::Failed,
                "protocol_write_failed",
            );
            return;
        }
        match process.try_wait() {
            Ok(Some(exit)) => {
                // A foreground shell may have exited while jobs still own the
                // PTY. End its whole session before publishing completion.
                if let Err(error) = process.kill() {
                    state.append(format!("Terminal cleanup failed: {error}\r\n").as_bytes());
                    state.finish(ProcessStatus::Failed, None, "cleanup_failed");
                    return;
                }
                final_drain(state, &mut process);
                let code = exit
                    .signal()
                    .is_none()
                    .then(|| i32::try_from(exit.exit_code()).unwrap_or(i32::MAX));
                let reason = exit
                    .signal()
                    .map(|s| format!("process_signal: {s}"))
                    .unwrap_or_else(|| "process_exit".into());
                state.finish(
                    if exit.success() {
                        ProcessStatus::Exited
                    } else {
                        ProcessStatus::Failed
                    },
                    code,
                    &reason,
                );
                return;
            }
            Err(error) => {
                state.append(format!("Terminal wait failed: {error}\r\n").as_bytes());
                cleanup(
                    state,
                    &mut process,
                    true,
                    ProcessStatus::Failed,
                    "wait_failed",
                );
                return;
            }
            Ok(None) => {}
        }
        match controls.recv_timeout(Duration::from_millis(10)) {
            Ok(request) => {
                let result = if request.cancel.is_cancelled() || Instant::now() >= request.deadline
                {
                    Err("terminal action expired before execution; nothing was sent".to_owned())
                } else {
                    match request.control {
                        Control::Write(bytes) => write(
                            state,
                            &mut process,
                            &bytes,
                            request.deadline,
                            &request.cancel,
                        ),
                        Control::Resize(rows, cols) => process
                            .resize(rows, cols)
                            .map(|()| state.resize_screen(rows, cols))
                            .map_err(|error| format!("terminal resize failed: {error}")),
                        Control::Interrupt => process
                            .interrupt()
                            .map_err(|error| format!("terminal interrupt failed: {error}")),
                    }
                };
                if request.reply.send(result).is_err() {
                    state.request_stop(true);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                state.request_stop(true);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn drain(state: &State, process: &mut PtyProcess) -> io::Result<bool> {
    let mut bytes = [0; 4096];
    // A noisy process cannot starve input, cancellation or the exit check.
    for _ in 0..64 {
        match process.read(&mut bytes) {
            Ok(0) => return Ok(true),
            Ok(n) => state.append(&bytes[..n]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

fn write(
    state: &State,
    process: &mut PtyProcess,
    bytes: &[u8],
    deadline: Instant,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let mut offset = 0;
    let result: io::Result<()> = (|| {
        while offset < bytes.len() {
            if state.stopping.load(Ordering::Acquire) != 0
                || cancel.is_cancelled()
                || Instant::now() >= deadline
            {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "input delivery interrupted or timed out",
                ));
            }
            match process.write(&bytes[offset..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => offset += n,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    drain(state, process)?;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    })();
    result.map_err(|error| {
        // Retrying the whole input after a partial write could execute it twice.
        // Fail closed and terminate this terminal, not just the request future.
        state.request_stop(true);
        format!("terminal input failed after {offset}/{} acknowledged bytes: {error}; terminal termination requested, do not retry this input on the same session", bytes.len())
    })
}

fn cleanup(
    state: &State,
    process: &mut PtyProcess,
    force: bool,
    status: ProcessStatus,
    reason: &str,
) {
    let mut failure = if force {
        process.kill().err()
    } else {
        process.terminate().err()
    };
    let grace = Instant::now() + Duration::from_millis(if force { 0 } else { 150 });
    while Instant::now() < grace {
        let _ = drain(state, process);
        std::thread::sleep(Duration::from_millis(5));
    }
    // Always clean all groups, even if the direct child exited during the grace period.
    if let Err(error) = process.kill() {
        failure.get_or_insert(error);
    }
    let reap_deadline = Instant::now() + Duration::from_secs(2);
    let mut exit_code = None;
    loop {
        let _ = drain(state, process);
        match process.try_wait() {
            Ok(Some(exit)) => {
                exit_code = exit
                    .signal()
                    .is_none()
                    .then(|| i32::try_from(exit.exit_code()).unwrap_or(i32::MAX));
                break;
            }
            Ok(None) if Instant::now() < reap_deadline => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Ok(None) => {
                failure.get_or_insert_with(|| {
                    io::Error::other("child did not exit after forced termination")
                });
                break;
            }
            Err(error) => {
                failure.get_or_insert(error);
                break;
            }
        }
    }
    final_drain(state, process);
    if let Some(error) = failure {
        state.append(format!("Terminal cleanup failed: {error}\r\n").as_bytes());
        state.finish(ProcessStatus::Failed, exit_code, "cleanup_failed");
    } else {
        state.finish(status, exit_code, reason);
    }
}

fn final_drain(state: &State, process: &mut PtyProcess) {
    let deadline = Instant::now() + Duration::from_millis(150);
    loop {
        match drain(state, process) {
            Ok(true) | Err(_) => break,
            Ok(false) if Instant::now() >= deadline => break,
            Ok(false) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}
