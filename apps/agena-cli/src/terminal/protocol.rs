use std::{
    env,
    io::{self, IsTerminal, Write},
};

use agena_tui_components::TerminalRgb;

use super::TerminalContext;

pub(super) fn detect_terminal_background(context: &TerminalContext) -> Option<TerminalRgb> {
    background_from_environment().or_else(|| {
        context
            .capabilities
            .default_color_query
            .is_supported()
            .then(query_terminal_background)
            .flatten()
    })
}

fn background_from_environment() -> Option<TerminalRgb> {
    if let Ok(value) = env::var("COLORFGBG")
        && let Some(color) = parse_colorfgbg(&value)
    {
        return Some(color);
    }

    for key in ["TERM_BACKGROUND", "VSCODE_THEME_KIND"] {
        let Ok(value) = env::var(key) else {
            continue;
        };
        match value.trim().to_ascii_lowercase().as_str() {
            "dark" | "highcontrastdark" => return Some(TerminalRgb::new(24, 24, 27)),
            "light" | "highcontrastlight" => return Some(TerminalRgb::new(250, 250, 250)),
            _ => {}
        }
    }
    None
}

fn parse_colorfgbg(value: &str) -> Option<TerminalRgb> {
    let index = value
        .split([';', ':'])
        .next_back()?
        .trim()
        .parse::<u8>()
        .ok()?;
    let (red, green, blue) = match index {
        0 => (0, 0, 0),
        1 => (170, 0, 0),
        2 => (0, 170, 0),
        3 => (170, 85, 0),
        4 => (0, 0, 170),
        5 => (170, 0, 170),
        6 => (0, 170, 170),
        7 => (170, 170, 170),
        8 => (85, 85, 85),
        9 => (255, 85, 85),
        10 => (85, 255, 85),
        11 => (255, 255, 85),
        12 => (85, 85, 255),
        13 => (255, 85, 255),
        14 => (85, 255, 255),
        15 => (255, 255, 255),
        16..=231 => {
            let offset = index - 16;
            let component = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            (
                component(offset / 36),
                component((offset % 36) / 6),
                component(offset % 6),
            )
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            (gray, gray, gray)
        }
    };
    Some(TerminalRgb::new(red, green, blue))
}

#[cfg(unix)]
fn query_terminal_background() -> Option<TerminalRgb> {
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return None;
    }

    let mut stdout = io::stdout();
    stdout.write_all(b"\x1b]11;?\x1b\\").ok()?;
    stdout.flush().ok()?;

    let fd = io::stdin().as_raw_fd();
    let first_byte_deadline = Instant::now() + Duration::from_millis(150);
    let mut completion_deadline = None;
    let mut response = Vec::with_capacity(64);
    while response.len() < 256 {
        let deadline = completion_deadline.unwrap_or(first_byte_deadline);
        if Instant::now() >= deadline {
            break;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout = i32::try_from(remaining.as_millis().max(1)).unwrap_or(150);
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one initialized pollfd remains valid for this call.
        if unsafe { libc::poll(&mut pollfd, 1, timeout) } <= 0 {
            break;
        }
        let mut chunk = [0_u8; 64];
        // SAFETY: chunk is writable and fd is a live terminal descriptor.
        let count = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if count <= 0 {
            break;
        }
        response.extend_from_slice(&chunk[..count as usize]);
        completion_deadline.get_or_insert_with(|| Instant::now() + Duration::from_millis(500));
        if osc11_response_end(&response).is_some() {
            return parse_osc11_response(&response);
        }
    }

    // An incomplete response still belongs to this startup transaction. Wait a
    // short bounded grace period for a terminator, then discard the abandoned
    // protocol tail before handing stdin to the application EventStream.
    let grace = Instant::now() + Duration::from_millis(100);
    while Instant::now() < grace {
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one initialized pollfd remains valid for this call.
        if unsafe { libc::poll(&mut pollfd, 1, 10) } <= 0 {
            continue;
        }
        let mut chunk = [0_u8; 64];
        // SAFETY: chunk is writable and fd is a live terminal descriptor.
        let count = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if count <= 0 {
            break;
        }
        response.extend_from_slice(&chunk[..count as usize]);
        if osc11_response_end(&response).is_some() {
            return parse_osc11_response(&response);
        }
    }
    None
}

#[cfg(not(unix))]
fn query_terminal_background() -> Option<TerminalRgb> {
    None
}

fn parse_osc11_response(response: &[u8]) -> Option<TerminalRgb> {
    let end = osc11_response_end(response)?;
    let response = String::from_utf8_lossy(&response[..end]);
    let payload = response.split("]11;").nth(1)?;
    let payload = payload
        .strip_prefix("rgb:")
        .or_else(|| payload.strip_prefix("rgba:"))?;
    let mut components = payload
        .split(['/', '\x07', '\x1b'])
        .take(3)
        .map(parse_osc_component);
    Some(TerminalRgb::new(
        components.next()??,
        components.next()??,
        components.next()??,
    ))
}

fn osc11_response_end(response: &[u8]) -> Option<usize> {
    let start = response.windows(4).position(|bytes| bytes == b"]11;")?;
    let payload = &response[start + 4..];
    if let Some(end) = payload.iter().position(|byte| *byte == 0x07) {
        return Some(start + 4 + end + 1);
    }
    payload
        .windows(2)
        .position(|bytes| bytes == b"\x1b\\")
        .map(|end| start + 4 + end + 2)
}

fn parse_osc_component(value: &str) -> Option<u8> {
    let value = value.trim();
    if value.is_empty() || value.len() > 4 {
        return None;
    }
    let raw = u32::from_str_radix(value, 16).ok()?;
    let maximum = (1_u32 << (value.len() * 4)) - 1;
    Some(((raw * 255 + maximum / 2) / maximum) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colorfgbg_and_complete_osc11() {
        assert_eq!(parse_colorfgbg("15;0"), Some(TerminalRgb::new(0, 0, 0)));
        assert_eq!(
            parse_colorfgbg("0;231"),
            Some(TerminalRgb::new(255, 255, 255))
        );
        assert_eq!(
            parse_osc11_response(b"\x1b]11;rgb:ffff/8000/0000\x1b\\"),
            Some(TerminalRgb::new(255, 128, 0))
        );
        assert_eq!(parse_osc11_response(b"\x1b]11;rgb:fae0/fae0/fae0"), None);
    }

    #[test]
    fn osc11_framing_is_fragmentation_safe() {
        let response = b"\x1b]11;rgb:fae0/fae0/fae0\x1b\\";
        for split in 0..response.len() {
            assert_eq!(osc11_response_end(&response[..split]), None);
        }
        assert_eq!(osc11_response_end(response), Some(response.len()));
    }
}
