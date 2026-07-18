use std::io::{self, Write};

use anyhow::{Result, bail};

const MAX_PROTOCOL_FRAME_BYTES: usize = 1024 * 1024;

/// Serializes application-owned terminal protocol frames. The broker never
/// reads stdin. Response-bearing probes use TerminalRuntime's typed exclusive
/// transaction; arbitrary callers cannot race a second input reader.
#[derive(Debug, Default)]
pub(super) struct TerminalProtocolBroker;

impl TerminalProtocolBroker {
    pub(super) fn write_frame(&mut self, output: &mut impl Write, frame: &[u8]) -> Result<()> {
        self.write_transaction(output, &[frame])
    }

    /// Validate every frame before emitting any of them, then flush the whole
    /// ordered transaction once. This is used for a query followed by its
    /// protocol barrier; callers can never expose a half-written transaction.
    pub(super) fn write_transaction(
        &mut self,
        output: &mut impl Write,
        frames: &[&[u8]],
    ) -> Result<()> {
        if frames.is_empty() {
            bail!("terminal protocol transaction cannot be empty");
        }
        for frame in frames {
            validate_complete_frame(frame)?;
        }
        for frame in frames {
            output.write_all(frame)?;
        }
        output.flush()?;
        Ok(())
    }
}

fn validate_complete_frame(frame: &[u8]) -> Result<()> {
    if frame.len() < 3 || frame[0] != 0x1b {
        bail!("terminal protocol frame must begin with ESC and cannot be empty");
    }
    if frame.len() > MAX_PROTOCOL_FRAME_BYTES {
        bail!("terminal protocol frame exceeds the 1 MiB safety limit");
    }
    match frame[1] {
        b']' => {
            if !frame.ends_with(&[0x07]) && !frame.ends_with(b"\x1b\\") {
                bail!("OSC frame is missing a BEL or ST terminator");
            }
            let body_end = if frame.ends_with(&[0x07]) {
                frame.len() - 1
            } else {
                frame.len() - 2
            };
            if frame[2..body_end].contains(&0x07)
                || frame[2..body_end]
                    .windows(2)
                    .any(|bytes| bytes == b"\x1b\\")
            {
                bail!("OSC buffer contains more than one protocol frame");
            }
        }
        b'P' => {
            if !frame.ends_with(b"\x1b\\") {
                bail!("DCS frame is missing an ST terminator");
            }
            if frame[2..frame.len() - 2]
                .windows(2)
                .any(|bytes| bytes == b"\x1b\\")
            {
                bail!("DCS buffer contains more than one protocol frame");
            }
        }
        b'[' => {
            let final_byte = *frame
                .last()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty CSI frame"))?;
            if !(0x40..=0x7e).contains(&final_byte) {
                bail!("CSI frame has no valid final byte");
            }
            if !frame[2..frame.len() - 1]
                .iter()
                .all(|byte| (0x20..=0x3f).contains(byte))
            {
                bail!("CSI frame contains invalid or trailing control data");
            }
        }
        _ => bail!("unsupported application terminal protocol frame"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_complete_protocol_frames() {
        assert!(validate_complete_frame(b"\x1b]52;c;YQ==\x07").is_ok());
        assert!(validate_complete_frame(b"\x1b[?25l").is_ok());
        assert!(validate_complete_frame(b"\x1b]52;c;YQ==").is_err());
        assert!(validate_complete_frame(b"plain text").is_err());
        assert!(validate_complete_frame(b"\x1b]52;c;YQ==\x07trailing\x07").is_err());
        assert!(validate_complete_frame(b"\x1bPpayload\x1b\\trailing\x1b\\").is_err());
        assert!(validate_complete_frame(b"\x1b[?25l\x1b[2J").is_err());
    }

    #[test]
    fn validates_a_transaction_before_writing_any_bytes() {
        let mut output = Vec::new();
        let result = TerminalProtocolBroker
            .write_transaction(&mut output, &[b"\x1b]11;?\x07", b"incomplete"]);
        assert!(result.is_err());
        assert!(output.is_empty());

        TerminalProtocolBroker
            .write_transaction(&mut output, &[b"\x1b]11;?\x07", b"\x1b[5n"])
            .expect("valid transaction");
        assert_eq!(output, b"\x1b]11;?\x07\x1b[5n");
    }

    #[cfg(unix)]
    #[test]
    fn pty_transaction_writes_output_without_consuming_user_input() {
        use std::{
            fs::File,
            io::{Read, Write},
            os::fd::FromRawFd,
        };

        let mut master_fd = -1;
        let mut slave_fd = -1;
        // SAFETY: openpty initializes both descriptors; null termios/winsize
        // request platform defaults, which are changed to raw immediately.
        let opened = unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert_eq!(opened, 0);
        // SAFETY: openpty returned two newly owned descriptors.
        let mut master = unsafe { File::from_raw_fd(master_fd) };
        // SAFETY: openpty returned two newly owned descriptors.
        let mut slave = unsafe { File::from_raw_fd(slave_fd) };
        let mut attributes = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: slave_fd is live and attributes points to writable memory.
        assert_eq!(
            unsafe { libc::tcgetattr(slave_fd, attributes.as_mut_ptr()) },
            0
        );
        // SAFETY: tcgetattr initialized the value.
        let mut attributes = unsafe { attributes.assume_init() };
        // SAFETY: attributes is an initialized termios value.
        unsafe { libc::cfmakeraw(&mut attributes) };
        // SAFETY: slave_fd and attributes are valid for this call.
        assert_eq!(
            unsafe { libc::tcsetattr(slave_fd, libc::TCSANOW, &attributes) },
            0
        );

        master.write_all(b"x").expect("write user input");
        let frame = b"\x1b]52;c;YQ==\x07";
        TerminalProtocolBroker
            .write_frame(&mut slave, frame)
            .expect("write protocol frame");

        let mut output = vec![0_u8; frame.len()];
        master
            .read_exact(&mut output)
            .expect("read protocol output");
        assert_eq!(output, frame);
        let mut input = [0_u8; 1];
        slave.read_exact(&mut input).expect("read preserved input");
        assert_eq!(input, *b"x");
    }
}
