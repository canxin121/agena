#[cfg(feature = "use-dev-tty")]
pub(crate) mod tty;

#[cfg(not(feature = "use-dev-tty"))]
pub(crate) mod mio;

#[cfg(feature = "use-dev-tty")]
pub(crate) use self::tty::UnixInternalEventSource;

#[cfg(not(feature = "use-dev-tty"))]
pub(crate) use self::mio::UnixInternalEventSource;

#[cfg(test)]
mod responsiveness_tests {
    use super::*;
    use crate::event::{source::EventSource, Event, InternalEvent, KeyCode};
    use crate::terminal::sys::file_descriptor::FileDesc;
    use std::{
        io::{self, Write},
        os::unix::net::UnixStream,
        time::Duration,
    };

    fn source() -> (UnixInternalEventSource, UnixStream) {
        let (reader, writer) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        #[cfg(feature = "libc")]
        let fd = {
            use std::os::fd::IntoRawFd;
            FileDesc::new(reader.into_raw_fd(), true)
        };
        #[cfg(not(feature = "libc"))]
        let fd = FileDesc::Owned(reader.into());
        (
            UnixInternalEventSource::from_file_descriptor(fd).unwrap(),
            writer,
        )
    }

    #[test]
    fn buffered_keys_do_not_wait_for_another_readiness_notification() {
        let (mut source, mut writer) = source();
        assert!(source.try_read(Some(Duration::ZERO)).unwrap().is_none());
        writer.write_all(b"xy").unwrap();
        assert!(
            matches!(source.try_read(Some(Duration::from_millis(100))).unwrap(),
            Some(InternalEvent::Event(Event::Key(key))) if key.code == KeyCode::Char('x'))
        );
        assert!(matches!(source.try_read(Some(Duration::ZERO)).unwrap(),
            Some(InternalEvent::Event(Event::Key(key))) if key.code == KeyCode::Char('y')));
    }

    #[test]
    fn closed_input_reports_eof_instead_of_spinning() {
        let (mut source, writer) = source();
        drop(writer);
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = tx.send(source.try_read(Some(Duration::from_millis(100))));
        });
        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("terminal poll must not spin after EOF");
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        worker.join().unwrap();
    }
}
