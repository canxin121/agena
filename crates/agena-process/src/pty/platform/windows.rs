//! ConPTY uses synchronous pipes. Dedicated bounded workers turn those into
//! nonblocking driver operations; closing the job and console releases the pipes.
use portable_pty::{Child, MasterPty};
use std::{
    io::{self, Read, Write},
    sync::mpsc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    },
};

type WriteRequest = (Vec<u8>, mpsc::SyncSender<io::Result<usize>>);

pub(crate) struct PtyIo {
    output: mpsc::Receiver<io::Result<Vec<u8>>>,
    buffered: Vec<u8>,
    offset: usize,
    input: mpsc::SyncSender<WriteRequest>,
    pending: Option<mpsc::Receiver<io::Result<usize>>>,
}

impl PtyIo {
    pub(crate) fn new(master: &dyn MasterPty) -> io::Result<Self> {
        let mut reader = master.try_clone_reader().map_err(super::super::other)?;
        let mut writer = master.take_writer().map_err(super::super::other)?;
        let (output_tx, output) = mpsc::sync_channel(32);
        let (input, input_rx) = mpsc::sync_channel::<WriteRequest>(1);
        std::thread::Builder::new()
            .name("agena-conpty-read".into())
            .spawn(move || {
                let mut buffer = [0; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(n) => {
                            if output_tx.send(Ok(buffer[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => {
                            let _ = output_tx.send(Err(e));
                            break;
                        }
                    }
                }
            })?;
        std::thread::Builder::new()
            .name("agena-conpty-write".into())
            .spawn(move || {
                while let Ok((bytes, reply)) = input_rx.recv() {
                    let result = writer.write(&bytes);
                    let failed = result.is_err();
                    if reply.send(result).is_err() || failed {
                        break;
                    }
                }
            })?;
        Ok(Self {
            output,
            buffered: Vec::new(),
            offset: 0,
            input,
            pending: None,
        })
    }

    pub(crate) fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.offset == self.buffered.len() {
            self.buffered = match self.output.try_recv() {
                Ok(result) => result?,
                Err(mpsc::TryRecvError::Empty) => return Err(io::ErrorKind::WouldBlock.into()),
                Err(mpsc::TryRecvError::Disconnected) => return Ok(0),
            };
            self.offset = 0;
        }
        let n = buffer.len().min(self.buffered.len() - self.offset);
        buffer[..n].copy_from_slice(&self.buffered[self.offset..self.offset + n]);
        self.offset += n;
        Ok(n)
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(reply) = &self.pending {
            match reply.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    return result;
                }
                Err(mpsc::TryRecvError::Disconnected) => return Err(super::super::closed()),
                Err(mpsc::TryRecvError::Empty) => return Err(io::ErrorKind::WouldBlock.into()),
            }
        }
        let (reply, result) = mpsc::sync_channel(1);
        self.input
            .try_send((bytes[..bytes.len().min(8192)].to_vec(), reply))
            .map_err(|_| super::super::closed())?;
        self.pending = Some(result);
        Err(io::ErrorKind::WouldBlock.into())
    }

    pub(crate) fn interrupt(&mut self) -> io::Result<()> {
        // ConPTY processes receive terminal Ctrl-C; Windows has no Unix SIGINT.
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match self.write(b"\x03") {
                Ok(1) => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(e) => return Err(e),
                _ => return Err(io::Error::other("failed to deliver terminal interrupt")),
            }
        }
    }
}

pub(crate) struct Job(HANDLE);
// SAFETY: this exclusively owned kernel handle may be transferred between threads.
unsafe impl Send for Job {}

impl Job {
    pub(crate) fn attach(child: &dyn Child) -> io::Result<Self> {
        // SAFETY: null security/name arguments create an unnamed private job.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self(handle);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let child_handle = child
            .as_raw_handle()
            .ok_or_else(|| io::Error::other("missing PTY child handle"))?;
        // SAFETY: limits has the documented layout and both handles remain live.
        if unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
            || unsafe { AssignProcessToJobObject(handle, child_handle.cast()) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }
    pub(crate) fn terminate(&self) -> io::Result<()> {
        if unsafe { TerminateJobObject(self.0, 1) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
