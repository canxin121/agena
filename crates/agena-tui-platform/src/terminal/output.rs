//! Bounded batching for terminal frame fragments. This remains synchronous
//! and owned by the terminal runtime; no background writer can reorder OSC/CSI.

use std::io::{self, BufWriter, Write};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const FRAME_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Clone, Default)]
pub(super) struct OutputControl(Arc<AtomicBool>);

impl OutputControl {
    pub(super) fn discard_pending(&self) {
        self.0.store(true, Ordering::Release);
    }
}

pub(super) struct TerminalOutput<W: Write> {
    writer: Option<BufWriter<W>>,
    control: OutputControl,
}

impl<W: Write> TerminalOutput<W> {
    pub(super) fn new(writer: W) -> Self {
        Self {
            writer: Some(BufWriter::with_capacity(FRAME_BUFFER_BYTES, writer)),
            control: Default::default(),
        }
    }

    pub(super) fn control(&self) -> OutputControl {
        self.control.clone()
    }

    fn discard_if_requested(&mut self) {
        if self.control.0.swap(false, Ordering::AcqRel) {
            // The panic hook may already have restored the shell. Also protect
            // a final cursor-show command emitted by Terminal's own Drop.
            let (writer, _) = self.writer.take().expect("writer is present").into_parts();
            self.writer = Some(BufWriter::with_capacity(FRAME_BUFFER_BYTES, writer));
        }
    }
}

impl<W: Write> Write for TerminalOutput<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.discard_if_requested();
        self.writer
            .as_mut()
            .expect("writer is present")
            .write(bytes)
    }

    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.discard_if_requested();
        self.writer
            .as_mut()
            .expect("writer is present")
            .write_all(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.discard_if_requested();
        self.writer.as_mut().expect("writer is present").flush()
    }
}

impl<W: Write> Drop for TerminalOutput<W> {
    fn drop(&mut self) {
        // Normal frames, protocol transactions and suspend all flush explicitly.
        // BufWriter's implicit drop-flush would violate emergency restoration.
        if let Some(writer) = self.writer.take() {
            let _ = writer.into_parts();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        backend::{Backend, CrosstermBackend},
        buffer::Cell,
        style::Color,
    };
    use std::{cell::RefCell, rc::Rc};

    #[derive(Default)]
    struct State {
        bytes: Vec<u8>,
        writes: usize,
        flushes: usize,
        fail: bool,
    }

    #[derive(Clone, Default)]
    struct Recorder(Rc<RefCell<State>>);
    impl Write for Recorder {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let mut state = self.0.borrow_mut();
            if state.fail {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "synthetic terminal failure",
                ));
            }
            state.writes += 1;
            state.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.borrow_mut().flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn frame_bytes_are_identical_but_small_backend_writes_are_batched() {
        let cells = (0..2000)
            .map(|index| {
                let mut cell = Cell::default();
                cell.set_symbol("x")
                    .set_fg(Color::Rgb((index % 255) as u8, 80, 160));
                cell
            })
            .collect::<Vec<_>>();
        let changes = || {
            cells
                .iter()
                .enumerate()
                .map(|(index, cell)| ((index % 80) as u16, (index / 80) as u16, cell))
        };
        let plain = Recorder::default();
        let batched = Recorder::default();
        let mut plain_backend = CrosstermBackend::new(plain.clone());
        let mut batched_backend = CrosstermBackend::new(TerminalOutput::new(batched.clone()));
        Backend::draw(&mut plain_backend, changes()).unwrap();
        Backend::flush(&mut plain_backend).unwrap();
        Backend::draw(&mut batched_backend, changes()).unwrap();
        Backend::flush(&mut batched_backend).unwrap();
        let original = plain.0.borrow();
        let optimized = batched.0.borrow();
        assert_eq!(original.bytes, optimized.bytes);
        assert!(optimized.writes <= original.bytes.len().div_ceil(FRAME_BUFFER_BYTES) + 1);
        assert!(optimized.writes * 100 < original.writes);
        eprintln!(
            "terminal frame: {} bytes; writer calls {} -> {}; bytes identical",
            original.bytes.len(),
            original.writes,
            optimized.writes
        );
    }

    #[test]
    fn protocol_transactions_flush_buffered_frame_bytes_in_order() {
        let recorder = Recorder::default();
        let mut output = TerminalOutput::new(recorder.clone());
        output.write_all(b"frame bytes").unwrap();
        let mut broker = agena_tui::terminal_protocol::TerminalProtocolBroker;
        broker
            .write_transaction(&mut output, &[b"\x1b]2;title\x07", b"\x1b[6n"])
            .unwrap();
        assert_eq!(
            recorder.0.borrow().bytes,
            b"frame bytes\x1b]2;title\x07\x1b[6n"
        );
        assert_eq!(recorder.0.borrow().flushes, 1);
    }

    #[test]
    fn emergency_restore_and_drop_never_emit_an_unfinished_frame() {
        let recorder = Recorder::default();
        let mut output = TerminalOutput::new(recorder.clone());
        output.write_all(b"must not follow panic hook").unwrap();
        output.control().discard_pending();
        output.flush().unwrap();
        assert!(recorder.0.borrow().bytes.is_empty());
        output
            .write_all(b"also not implicitly flushed on drop")
            .unwrap();
        drop(output);
        assert!(recorder.0.borrow().bytes.is_empty());
    }

    #[test]
    fn final_cursor_restore_cannot_flush_a_discarded_frame() {
        let expected = Recorder::default();
        let actual = Recorder::default();
        let mut plain = CrosstermBackend::new(expected.clone());
        Backend::show_cursor(&mut plain).unwrap();
        let mut output = TerminalOutput::new(actual.clone());
        let control = output.control();
        output.write_all(b"stale frame before panic").unwrap();
        let mut backend = CrosstermBackend::new(output);
        control.discard_pending();
        Backend::show_cursor(&mut backend).unwrap();
        drop(backend);
        assert_eq!(actual.0.borrow().bytes, expected.0.borrow().bytes);
    }

    #[test]
    fn flush_errors_propagate_and_normal_suspend_flushes_once() {
        let recorder = Recorder::default();
        let mut output = TerminalOutput::new(recorder.clone());
        output.write_all(b"pending").unwrap();
        recorder.0.borrow_mut().fail = true;
        assert_eq!(
            output.flush().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        recorder.0.borrow_mut().fail = false;
        output.flush().unwrap();
        assert_eq!(recorder.0.borrow().bytes, b"pending");
        drop(output);
        assert_eq!(recorder.0.borrow().bytes, b"pending");
    }
}
