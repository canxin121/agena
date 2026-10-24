//! Terminal protocol detection (kitty, iterm2, ...).

use std::io::{self, Write};

use anyhow::{Result, bail};

const MAX_PROTOCOL_FRAME_BYTES: usize = 1024 * 1024;

/// Serializes application-owned terminal protocol frames. The broker never
/// reads stdin. Response-bearing probes use TerminalRuntime's typed exclusive
/// transaction; arbitrary callers cannot race a second input reader.
#[derive(Debug, Default)]
pub struct TerminalProtocolBroker;

impl TerminalProtocolBroker {
    pub fn write_frame(&mut self, output: &mut impl Write, frame: &[u8]) -> Result<()> {
        self.write_transaction(output, &[frame])
    }

    /// Validate every frame before emitting any of them, then flush the whole
    /// ordered transaction once. This is used for a query followed by its
    /// protocol barrier; callers can never expose a half-written transaction.
    pub fn write_transaction(&mut self, output: &mut impl Write, frames: &[&[u8]]) -> Result<()> {
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
    // A bare BEL (0x07) is the universal VT attention signal and a complete
    // terminal frame on its own. It is used for attention notifications where
    // the terminal family has no richer notification protocol.
    if frame == b"\x07" {
        return Ok(());
    }
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
