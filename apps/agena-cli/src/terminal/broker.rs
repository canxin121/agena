use std::io::{self, Write};

use anyhow::{Result, bail};

/// Serializes application-owned terminal protocol frames. The broker never
/// reads stdin: response-bearing protocols stay disabled until the event
/// parser can route their bytes without stealing user input.
#[derive(Debug, Default)]
pub(super) struct TerminalProtocolBroker {
    generation: u64,
}

impl TerminalProtocolBroker {
    pub(super) fn write_frame(&mut self, output: &mut impl Write, frame: &[u8]) -> Result<()> {
        validate_complete_frame(frame)?;
        output.write_all(frame)?;
        output.flush()?;
        Ok(())
    }

    pub(super) fn next_generation(&mut self) {
        self.generation = self.generation.saturating_add(1);
    }
}

fn validate_complete_frame(frame: &[u8]) -> Result<()> {
    if frame.len() < 3 || frame[0] != 0x1b {
        bail!("terminal protocol frame must begin with ESC and cannot be empty");
    }
    match frame[1] {
        b']' => {
            if !frame.ends_with(&[0x07]) && !frame.ends_with(b"\x1b\\") {
                bail!("OSC frame is missing a BEL or ST terminator");
            }
        }
        b'P' => {
            if !frame.ends_with(b"\x1b\\") {
                bail!("DCS frame is missing an ST terminator");
            }
        }
        b'[' => {
            let final_byte = *frame
                .last()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty CSI frame"))?;
            if !(0x40..=0x7e).contains(&final_byte) {
                bail!("CSI frame has no valid final byte");
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
        TerminalProtocolBroker::default()
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
