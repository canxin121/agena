//! Helper module to build a protocol, and swap protocols at runtime

use std::{
    env,
    io::{self, Read, Write},
    time::{Duration, Instant},
};

use crate::{
    FontSize, Resize, Result,
    errors::Errors,
    protocol::{
        Protocol, StatefulProtocol, StatefulProtocolType,
        halfblocks::Halfblocks,
        iterm2::Iterm2,
        kitty::{Kitty, StatefulKitty},
        sixel::Sixel,
    },
};
use cap_parser::{BackgroundColorQuery, Parser, QueryStdioOptions, Response};
use image::{DynamicImage, Rgba};
use rand::random;
use ratatui::layout::Size;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub mod cap_parser;

#[derive(Debug, PartialEq, Clone)]
pub enum Capability {
    /// Reports supporting kitty graphics protocol.
    Kitty,
    /// Reports supporting sixel graphics protocol.
    Sixel,
    /// Reports supporting rectangular ops.
    RectangularOps,
    /// Reports font size in pixels.
    CellSize(Option<(u16, u16)>),
    /// Reports supporting text sizing protocol.
    TextSizingProtocol,
    /// Reports a background color.
    Background(u8, u8, u8),
}

const STDIN_READ_TIMEOUT_MILLIS: u64 = 2000;

#[derive(Clone, Debug)]
pub struct Picker {
    font_size: FontSize,
    protocol_type: ProtocolType,
    background_color: Option<Rgba<u8>>,
    pub(crate) is_tmux: bool,
    capabilities: Vec<Capability>,
}

/// Serde-friendly protocol-type enum for [Picker].
#[derive(PartialEq, Clone, Debug, Copy)]
#[cfg_attr(
    feature = "serde",
    derive(Deserialize, Serialize),
    serde(rename_all = "lowercase")
)]
pub enum ProtocolType {
    Halfblocks,
    Sixel,
    Kitty,
    Iterm2,
}

impl ProtocolType {
    pub fn next(&self) -> ProtocolType {
        match self {
            ProtocolType::Halfblocks => ProtocolType::Sixel,
            ProtocolType::Sixel => ProtocolType::Kitty,
            ProtocolType::Kitty => ProtocolType::Iterm2,
            ProtocolType::Iterm2 => ProtocolType::Halfblocks,
        }
    }
}

/// Helper for building widgets
impl Picker {
    /// Query terminal stdio for graphics capabilities and font-size with some escape sequences.
    ///
    /// This writes and reads from stdio momentarily. WARNING: this method should be called after
    /// entering alternate screen but before reading terminal events.
    ///
    /// # Example
    /// ```rust
    /// use ratatui_image::picker::Picker;
    /// let mut picker = Picker::from_query_stdio();
    /// ```
    ///
    pub fn from_query_stdio() -> Result<Self> {
        Picker::from_query_stdio_with_options(QueryStdioOptions::default())
    }

    /// This should ONLY be used if [Capability::TextSizingProtocol] is needed for some external
    /// reason.
    ///
    /// Query for additional capabilities, currently supports querying for [Text Sizing Protocol].
    ///
    /// The result can be checked by searching for [Capability::TextSizingProtocol] in [Picker::capabilities].
    ///
    /// [Text Sizing Protocol] <https://sw.kovidgoyal.net/kitty/text-sizing-protocol//>
    pub fn from_query_stdio_with_options(options: QueryStdioOptions) -> Result<Self> {
        // Detect tmux, and only if positive then take some risky guess for iTerm2 support.
        let (is_tmux, tmux_proto) = detect_tmux_and_outer_protocol_from_env();

        Self::from_query_stdio_with_transport(options, is_tmux, tmux_proto)
    }

    /// Query stdio using transport information supplied by the terminal owner.
    ///
    /// Unlike environment auto-detection, this correctly handles tmux sessions
    /// whose `TERM` is `screen-*`, and it does not change tmux options.
    pub fn from_query_stdio_with_options_and_tmux(
        options: QueryStdioOptions,
        is_tmux: bool,
    ) -> Result<Self> {
        let tmux_proto = is_tmux.then(outer_protocol_from_env).flatten();
        Self::from_query_stdio_with_transport(options, is_tmux, tmux_proto)
    }

    /// Query only the terminal's default background color.
    ///
    /// Color detection is intentionally independent from graphics protocol
    /// negotiation: applications still need a stable palette when native
    /// images are disabled. The terminal owner selects the query appropriate
    /// for the endpoint and supplies the complete transport policy.
    pub fn query_background_color_stdio(
        query: BackgroundColorQuery,
        is_tmux: bool,
        timeout: Duration,
    ) -> Result<(u8, u8, u8)> {
        let mut raw_mode = RawModeGuard::new(enable_raw_mode()?);
        let query_result = query_background_color(is_tmux, query, Instant::now() + timeout);
        let restore_result = raw_mode.restore();
        match (query_result, restore_result) {
            (Ok(result), Ok(())) => Ok(result),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    fn from_query_stdio_with_transport(
        options: QueryStdioOptions,
        is_tmux: bool,
        tmux_proto: Option<ProtocolType>,
    ) -> Result<Self> {
        static DEFAULT_PICKER: Picker = Picker {
            // This is completely arbitrary. For halfblocks, it doesn't have to be precise
            // since we're not rendering pixels. It should be roughly 1:2 ratio, and some
            // reasonable size.
            font_size: FontSize::new(10, 20),
            background_color: None,
            protocol_type: ProtocolType::Halfblocks,
            is_tmux: false,
            capabilities: Vec::new(),
        };

        let mut options_with_blacklist = options;
        let is_wezterm = env::var("WEZTERM_EXECUTABLE").is_ok_and(|s| !s.is_empty());
        let is_konsole = env::var("KONSOLE_VERSION").is_ok_and(|s| !s.is_empty());
        if is_wezterm || is_konsole {
            // WezTerm could use Sixel, but iTerm2 (detected later is better).
            // Konsole's Sixel implementation is buggy: https://github.com/ratatui/ratatui-image?tab=readme-ov-file#compatibility-matrix
            // Neither implement the placeholder part of kitty correctly.
            options_with_blacklist.blacklist_protocols =
                vec![ProtocolType::Kitty, ProtocolType::Sixel];
        }

        // Write and read to stdin to query protocol capabilities and font-size.
        match query_with_timeout(is_tmux, options_with_blacklist) {
            Ok((capability_proto, font_size, caps)) => {
                let iterm2_proto = iterm2_from_env();
                Ok(picker_from_query_parts(
                    capability_proto,
                    font_size,
                    caps,
                    is_tmux,
                    tmux_proto,
                    iterm2_proto,
                ))
            }
            Err(Errors::NoCap | Errors::NoStdinResponse | Errors::NoFontSize) => {
                let mut p = DEFAULT_PICKER.clone();
                p.is_tmux = is_tmux;
                Ok(p)
            }
            Err(err) => Err(err),
        }
    }

    /// Create a picker that is guaranteed to only work with Halfblocks.
    ///
    /// # Example
    /// ```rust
    /// use ratatui_image::picker::Picker;
    ///
    /// let mut picker = Picker::halfblocks();
    /// ```
    pub fn halfblocks() -> Self {
        // Detect tmux, ignore iTerm2 as we don't have font-size.
        let (is_tmux, _tmux_proto) = detect_tmux_and_outer_protocol_from_env();

        Self {
            font_size: FontSize::new(10, 20),
            background_color: None,
            protocol_type: ProtocolType::Halfblocks,
            is_tmux,
            capabilities: Vec::new(),
        }
    }

    /// Construct a picker from already negotiated terminal properties.
    ///
    /// This constructor performs no environment probing, terminal I/O, or
    /// multiplexer configuration changes. It is intended for applications
    /// that centrally own terminal negotiation and transport policy.
    pub fn from_parts(
        font_size: FontSize,
        protocol_type: ProtocolType,
        is_tmux: bool,
        capabilities: Vec<Capability>,
    ) -> Self {
        Self {
            font_size,
            background_color: None,
            protocol_type,
            is_tmux,
            capabilities,
        }
    }

    /// Create a picker from a given terminal [FontSize].
    #[deprecated(
        since = "9.0.0",
        note = "use `from_query_stdio` or `halfblocks` instead"
    )]
    pub fn from_fontsize(font_size: FontSize) -> Self {
        // Detect tmux, and if positive then take some risky guess for iTerm2 support.
        let (is_tmux, tmux_proto) = detect_tmux_and_outer_protocol_from_env();

        // Disregard protocol-from-capabilities if some env var says that we could try iTerm2.
        let iterm2_proto = iterm2_from_env();

        let protocol_type = tmux_proto
            .or(iterm2_proto)
            .unwrap_or(ProtocolType::Halfblocks);

        Self {
            font_size,
            background_color: None,
            protocol_type,
            is_tmux,
            capabilities: Vec::new(),
        }
    }

    /// Returns the current protocol type.
    pub fn protocol_type(&self) -> ProtocolType {
        self.protocol_type
    }

    /// Force a protocol type.
    pub fn set_protocol_type(&mut self, protocol_type: ProtocolType) {
        self.protocol_type = protocol_type;
    }

    /// Returns the [FontSize] detected by [Picker::from_query_stdio].
    pub fn font_size(&self) -> FontSize {
        self.font_size
    }

    /// Change the default background color (transparent black).
    pub fn set_background_color<T: Into<Rgba<u8>>>(&mut self, background_color: Option<T>) {
        self.background_color = background_color.map(Into::into);
    }

    /// Returns the capabilities detected by [Picker::from_query_stdio].
    pub fn capabilities(&self) -> &Vec<Capability> {
        &self.capabilities
    }

    /// Returns a new protocol.
    ///
    /// The image must match the given area at the terminal's current font size.
    pub(crate) fn new_protocol_raw(&self, image: DynamicImage, size: Size) -> Result<Protocol> {
        match self.protocol_type {
            ProtocolType::Halfblocks => Ok(Protocol::Halfblocks(Halfblocks::new(image, size)?)),
            ProtocolType::Sixel => Ok(Protocol::Sixel(Sixel::new(image, size, self.is_tmux)?)),
            ProtocolType::Kitty => Ok(Protocol::Kitty(Kitty::new(
                image,
                size,
                rand::random(),
                self.is_tmux,
            )?)),
            ProtocolType::Iterm2 => Ok(Protocol::ITerm2(Iterm2::new(image, size, self.is_tmux)?)),
        }
    }

    /// Returns a new protocol for [`crate::Image`] widgets that fits into the given size.
    pub fn new_protocol(
        &self,
        image: DynamicImage,
        size: Size,
        resize: Resize,
    ) -> Result<Protocol> {
        let desired =
            Resize::round_pixel_size_to_cells(image.width(), image.height(), self.font_size);
        let (image, area) =
            match resize.needs_resize(&image, Some(desired), self.font_size, None, size, false) {
                Some(area) => {
                    let image = resize.resize(&image, self.font_size, area, self.background_color);
                    (image, area)
                }
                None => (image, desired),
            };

        self.new_protocol_raw(image, area)
    }

    /// Returns a new *stateful* protocol for [`crate::StatefulImage`] widgets.
    pub fn new_resize_protocol(&self, image: DynamicImage) -> StatefulProtocol {
        let protocol_type = match self.protocol_type {
            ProtocolType::Halfblocks => StatefulProtocolType::Halfblocks(Halfblocks::default()),
            ProtocolType::Sixel => StatefulProtocolType::Sixel(Sixel {
                is_tmux: self.is_tmux,
                ..Sixel::default()
            }),
            ProtocolType::Kitty => {
                StatefulProtocolType::Kitty(StatefulKitty::new(random(), self.is_tmux))
            }
            ProtocolType::Iterm2 => StatefulProtocolType::ITerm2(Iterm2 {
                is_tmux: self.is_tmux,
                ..Iterm2::default()
            }),
        };
        StatefulProtocol::new(image, self.font_size, self.background_color, protocol_type)
    }
}

fn picker_from_query_parts(
    capability_proto: Option<ProtocolType>,
    font_size: Option<FontSize>,
    capabilities: Vec<Capability>,
    is_tmux: bool,
    tmux_proto: Option<ProtocolType>,
    environment_proto: Option<ProtocolType>,
) -> Picker {
    // IO-based detection is authoritative; env-based hints are fallbacks
    // (env vars like KITTY_WINDOW_ID can be stale in tmux sessions).
    let protocol_type = capability_proto
        .or(tmux_proto)
        .or(environment_proto)
        .unwrap_or(ProtocolType::Halfblocks);
    Picker {
        // A missing cell-size reply must not erase independent capabilities
        // that did arrive, especially OSC 11. Agena may still have a trusted
        // endpoint protocol hint and can use the conservative geometry while
        // retaining the correct light/dark appearance.
        font_size: font_size.unwrap_or(FontSize::new(10, 20)),
        background_color: None,
        protocol_type,
        is_tmux,
        capabilities,
    }
}

fn detect_tmux_and_outer_protocol_from_env() -> (bool, Option<ProtocolType>) {
    // Check if we're inside tmux.
    if !env::var("TERM").is_ok_and(|term| term.starts_with("tmux"))
        && !env::var("TERM_PROGRAM").is_ok_and(|term_program| term_program == "tmux")
    {
        return (false, None);
    }

    (true, outer_protocol_from_env())
}

fn outer_protocol_from_env() -> Option<ProtocolType> {
    // Crude guess based on the *existence* of some magic program specific env vars.
    // Note: kitty is detected via io query (which works through tmux passthrough),
    // not env vars, since KITTY_WINDOW_ID is often stale in tmux sessions.
    const OUTER_TERM_HINTS: [(&str, ProtocolType); 2] = [
        ("ITERM_SESSION_ID", ProtocolType::Iterm2),
        ("WEZTERM_EXECUTABLE", ProtocolType::Iterm2),
    ];
    for (hint, proto) in OUTER_TERM_HINTS {
        if env::var(hint).is_ok_and(|s| !s.is_empty()) {
            return Some(proto);
        }
    }
    None
}

fn iterm2_from_env() -> Option<ProtocolType> {
    if env::var("TERM_PROGRAM").is_ok_and(|term_program| {
        term_program.contains("iTerm")
            || term_program.contains("WezTerm")
            || term_program.contains("mintty")
            || term_program.contains("vscode")
            || term_program.contains("Tabby")
            || term_program.contains("Hyper")
            || term_program.contains("rio")
            || term_program.contains("Bobcat")
            || term_program.contains("WarpTerminal")
    }) {
        return Some(ProtocolType::Iterm2);
    }
    if env::var("LC_TERMINAL").is_ok_and(|lc_term| lc_term.contains("iTerm")) {
        return Some(ProtocolType::Iterm2);
    }
    None
}

#[cfg(not(windows))]
fn enable_raw_mode() -> Result<impl FnOnce() -> Result<()>> {
    use rustix::termios::{self, LocalModes, OptionalActions};

    let stdin = io::stdin();
    let mut termios = termios::tcgetattr(&stdin)?;
    let termios_original = termios.clone();

    // Disable canonical mode to read without waiting for Enter, disable echoing.
    termios.local_modes &= !LocalModes::ICANON;
    termios.local_modes &= !LocalModes::ECHO;
    termios::tcsetattr(&stdin, OptionalActions::Drain, &termios)?;

    Ok(move || {
        Ok(termios::tcsetattr(
            io::stdin(),
            OptionalActions::Now,
            &termios_original,
        )?)
    })
}

#[cfg(windows)]
fn enable_raw_mode() -> Result<impl FnOnce() -> Result<()>> {
    use windows::{
        Win32::{
            Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE},
            Storage::FileSystem::{
                self, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
            },
            System::Console::{
                self, CONSOLE_MODE, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
            },
        },
        core::PCWSTR,
    };

    let utf16: Vec<u16> = "CONIN$\0".encode_utf16().collect();
    let utf16_ptr: *const u16 = utf16.as_ptr();

    let in_handle = unsafe {
        FileSystem::CreateFileW(
            PCWSTR(utf16_ptr),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            HANDLE::default(),
        )
    }?;

    let mut original_in_mode = CONSOLE_MODE::default();
    unsafe { Console::GetConsoleMode(in_handle, &mut original_in_mode) }?;

    let requested_in_modes = !ENABLE_ECHO_INPUT & !ENABLE_LINE_INPUT & !ENABLE_PROCESSED_INPUT;
    let in_mode = original_in_mode & requested_in_modes;
    unsafe { Console::SetConsoleMode(in_handle, in_mode) }?;

    Ok(move || {
        unsafe { Console::SetConsoleMode(in_handle, original_in_mode) }?;
        Ok(())
    })
}

#[cfg(not(windows))]
fn font_size_fallback() -> Option<FontSize> {
    use rustix::termios::{self, Winsize};

    let winsize = termios::tcgetwinsize(io::stdout()).ok()?;
    let Winsize {
        ws_xpixel: x,
        ws_ypixel: y,
        ws_col: cols,
        ws_row: rows,
    } = winsize;
    if x == 0 || y == 0 || cols == 0 || rows == 0 {
        return None;
    }

    Some(FontSize::new(x / cols, y / rows))
}

#[cfg(windows)]
fn font_size_fallback() -> Option<FontSize> {
    None
}

/// Query the terminal, by writing and reading to stdin and stdout.
/// The terminal must be in "raw mode" and should probably be reset to "cooked mode" when this
/// operation has completed.
///
/// The returned [ProtocolType] and [FontSize] may be included in the list of [Capability]s,
/// but the burden of picking out the right one or a font-size fallback is already resolved here.
fn query_stdio_capabilities(
    is_tmux: bool,
    options: QueryStdioOptions,
    deadline: Instant,
) -> Result<(Option<ProtocolType>, Option<FontSize>, Vec<Capability>)> {
    // Send several control sequences at once:
    // `_Gi=...`: Kitty graphics support.
    // `[c`: Capabilities including sixels.
    // `[16t`: Cell-size (perhaps we should also do `[14t`).
    // `[1337n`: iTerm2 (some terminals implement the protocol but sadly not this custom CSI)
    // `[5n`: Device Status Report, implemented by all terminals, ensure that there is some
    // response and we don't hang reading forever.
    let query = Parser::query(is_tmux, options);
    io::stdout().write_all(query.as_bytes())?;
    io::stdout().flush()?;

    let mut parser = Parser::new();
    let mut responses = vec![];
    loop {
        let mut charbuf: [u8; 50] = [0; 50];
        let read = read_stdin_with_deadline(&mut charbuf, deadline)?;
        if read == 0 {
            return Err(Errors::NoStdinResponse);
        }

        if parse_query_response_chunk(&mut parser, &charbuf[..read], &mut responses) {
            break;
        }
    }

    interpret_parser_responses(responses)
}

fn parse_query_response_chunk(
    parser: &mut Parser,
    bytes: &[u8],
    responses: &mut Vec<Response>,
) -> bool {
    let mut status_seen = false;
    for byte in bytes {
        for response in parser.push(char::from(*byte)) {
            if response == Response::Status {
                status_seen = true;
            } else {
                responses.push(response);
            }
        }
    }
    status_seen
}

fn query_background_color(
    is_tmux: bool,
    query: BackgroundColorQuery,
    deadline: Instant,
) -> Result<(u8, u8, u8)> {
    let request = Parser::background_query(is_tmux, query);
    io::stdout().write_all(request.as_bytes())?;
    io::stdout().flush()?;

    let mut parser = Parser::new();
    let mut background = None;
    loop {
        let mut buffer = [0_u8; 128];
        let read = read_stdin_with_deadline(&mut buffer, deadline)?;
        if read == 0 {
            return Err(Errors::NoStdinResponse);
        }
        for byte in &buffer[..read] {
            for response in parser.push(char::from(*byte)) {
                match advance_background_query(&mut background, query, response) {
                    BackgroundQueryProgress::Pending => {}
                    BackgroundQueryProgress::Complete(Some(background)) => return Ok(background),
                    BackgroundQueryProgress::Complete(None) => return Err(Errors::NoCap),
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackgroundQueryProgress {
    Pending,
    Complete(Option<(u8, u8, u8)>),
}

fn advance_background_query(
    background: &mut Option<(u8, u8, u8)>,
    query: BackgroundColorQuery,
    response: Response,
) -> BackgroundQueryProgress {
    match response {
        Response::Background(response_query, red, green, blue) if response_query == query => {
            *background = Some((red, green, blue));
            BackgroundQueryProgress::Pending
        }
        // `background_query` appends DSR after the OSC query on the same
        // ordered terminal stream. DSR is the protocol boundary: waiting a
        // fixed grace interval after it is neither necessary nor stronger.
        Response::Status => BackgroundQueryProgress::Complete(*background),
        _ => BackgroundQueryProgress::Pending,
    }
}

fn interpret_parser_responses(
    responses: Vec<Response>,
) -> Result<(Option<ProtocolType>, Option<FontSize>, Vec<Capability>)> {
    if responses.is_empty() {
        return Err(Errors::NoCap);
    }

    let mut capabilities = Vec::new();

    let mut proto = None;
    let mut font_size = None;

    let mut cursor_position_reports = vec![];
    for response in &responses {
        if let Some(capability) = match response {
            Response::Kitty => {
                proto = Some(ProtocolType::Kitty);
                Some(Capability::Kitty)
            }
            Response::Sixel => {
                if proto.is_none() {
                    // Only if kitty is not supported.
                    proto = Some(ProtocolType::Sixel);
                }
                Some(Capability::Sixel)
            }
            Response::RectangularOps => Some(Capability::RectangularOps),
            Response::CellSize(cell_size) => {
                if let Some((w, h)) = cell_size {
                    font_size = Some((*w, *h).into());
                }
                Some(Capability::CellSize(*cell_size))
            }
            Response::CursorPositionReport(x, y) => {
                cursor_position_reports.push((x, y));
                None
            }
            Response::Background(_, r, g, b) => Some(Capability::Background(*r, *g, *b)),
            Response::Status => None,
        } {
            capabilities.push(capability);
        }
    }

    // In case some terminal didn't support the cell-size query.
    font_size = font_size.or_else(font_size_fallback);

    if let [(x1, _y1), (x2, _y2), (x3, _y3)] = cursor_position_reports[..] {
        // Test if the cursor advanced exactly two columns (instead of one) on both the width and
        // scaling queries of the protocol.
        // The documentation is a bit ambiguous, as it only says the cursor positions "need to be
        // different from each other".
        // However from my testing on Kitty and other terminals that do not support the feature,
        // the cursor always advances at least one column since it is printing a space, so the CPRs
        // will always be different from each other (unless we would move the cursor to a known
        // position or something like that - and this also begs the question of needing to do this
        // anyway, for the edge case of the cursor being at the very end of a line).
        // My interpretation is that the cursor should advance 2 columns, instead of one, with both
        // queries, and only then can we interpret it as supported.
        // The Foot terminal notably reports a 2 column movement but fortunately only for the `w=2`
        // query.
        //
        // The row part can be ignored.
        if *x2 == x1 + 2 && *x3 == x2 + 2 {
            capabilities.push(Capability::TextSizingProtocol);
        }
    }

    Ok((proto, font_size, capabilities))
}

fn query_with_timeout(
    is_tmux: bool,
    options: QueryStdioOptions,
) -> Result<(Option<ProtocolType>, Option<FontSize>, Vec<Capability>)> {
    let timeout = options.timeout;
    let mut raw_mode = RawModeGuard::new(enable_raw_mode()?);
    let query_result = query_stdio_capabilities(is_tmux, options, Instant::now() + timeout);
    let restore_result = raw_mode.restore();
    match (query_result, restore_result) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

struct RawModeGuard<F: FnOnce() -> Result<()>> {
    restore: Option<F>,
}

impl<F: FnOnce() -> Result<()>> RawModeGuard<F> {
    fn new(restore: F) -> Self {
        Self {
            restore: Some(restore),
        }
    }

    fn restore(&mut self) -> Result<()> {
        self.restore.take().map_or(Ok(()), |restore| restore())
    }
}

impl<F: FnOnce() -> Result<()>> Drop for RawModeGuard<F> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[cfg(not(windows))]
fn read_stdin_with_deadline(buffer: &mut [u8], deadline: Instant) -> Result<usize> {
    read_with_deadline(&mut io::stdin(), buffer, deadline)
}

#[cfg(not(windows))]
fn read_with_deadline(
    input: &mut (impl Read + std::os::fd::AsFd),
    buffer: &mut [u8],
    deadline: Instant,
) -> Result<usize> {
    use rustix::event::{PollFd, PollFlags, poll};

    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(Errors::NoStdinResponse);
    }
    let timeout_millis = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
    {
        let mut descriptors = [PollFd::new(&*input, PollFlags::IN)];
        if poll(&mut descriptors, timeout_millis)? == 0 {
            return Err(Errors::NoStdinResponse);
        }
        if descriptors[0]
            .revents()
            .intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL)
        {
            return Err(Errors::NoStdinResponse);
        }
    }
    Ok(input.read(buffer)?)
}

#[cfg(windows)]
fn read_stdin_with_deadline(buffer: &mut [u8], deadline: Instant) -> Result<usize> {
    use windows::Win32::{
        Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::{
            Console::{GetStdHandle, STD_INPUT_HANDLE},
            Threading::WaitForSingleObject,
        },
    };

    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(Errors::NoStdinResponse);
    }
    let timeout_millis = remaining.as_millis().min(u128::from(u32::MAX)) as u32;
    let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) }?;
    let status = unsafe { WaitForSingleObject(input, timeout_millis) };
    if status == WAIT_TIMEOUT {
        return Err(Errors::NoStdinResponse);
    }
    if status != WAIT_OBJECT_0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(io::stdin().read(buffer)?)
}

#[cfg(test)]
mod tests {
    use std::assert_eq;

    use crate::picker::{Capability, Picker, ProtocolType};

    use super::{
        BackgroundQueryProgress, advance_background_query,
        cap_parser::{BackgroundColorQuery, Parser, Response},
        interpret_parser_responses, parse_query_response_chunk, picker_from_query_parts,
    };

    #[test]
    fn background_query_commits_only_at_the_ordered_dsr_barrier() {
        let mut background = None;
        assert_eq!(
            advance_background_query(
                &mut background,
                BackgroundColorQuery::Iterm2Osc4,
                Response::Background(BackgroundColorQuery::Iterm2Osc4, 250, 224, 224),
            ),
            BackgroundQueryProgress::Pending
        );
        assert_eq!(background, Some((250, 224, 224)));
        assert_eq!(
            advance_background_query(
                &mut background,
                BackgroundColorQuery::Iterm2Osc4,
                Response::Background(BackgroundColorQuery::Osc11, 0, 0, 0),
            ),
            BackgroundQueryProgress::Pending
        );
        assert_eq!(
            advance_background_query(
                &mut background,
                BackgroundColorQuery::Iterm2Osc4,
                Response::Status,
            ),
            BackgroundQueryProgress::Complete(Some((250, 224, 224)))
        );

        let mut reordered_background = None;
        assert_eq!(
            advance_background_query(
                &mut reordered_background,
                BackgroundColorQuery::Osc11,
                Response::Status,
            ),
            BackgroundQueryProgress::Complete(None)
        );
    }

    #[test]
    fn status_response_does_not_discard_later_capabilities_in_the_same_read() {
        let mut parser = Parser::new();
        let mut responses = Vec::new();
        let status_seen = parse_query_response_chunk(
            &mut parser,
            concat!(
                "\x1b[0n",
                "\x1b]11;rgb:ffff/ffff/ffff\x07",
                "\x1b[6;20;10t"
            )
            .as_bytes(),
            &mut responses,
        );

        assert!(status_seen);
        let (_, font_size, capabilities) = interpret_parser_responses(responses).unwrap();
        let font_size = font_size.expect("cell-size response should be retained");
        assert_eq!((font_size.width, font_size.height), (10, 20));
        assert!(capabilities.contains(&Capability::Background(255, 255, 255)));
    }

    #[test]
    fn missing_cell_size_does_not_erase_an_independent_background_response() {
        let picker = picker_from_query_parts(
            None,
            None,
            vec![Capability::Background(248, 249, 250)],
            false,
            None,
            Some(ProtocolType::Iterm2),
        );

        assert_eq!(picker.protocol_type(), ProtocolType::Iterm2);
        assert_eq!(
            (picker.font_size().width, picker.font_size().height),
            (10, 20)
        );
        assert!(
            picker
                .capabilities()
                .contains(&Capability::Background(248, 249, 250))
        );
    }

    #[test]
    fn test_cycle_protocol() {
        let mut proto = ProtocolType::Halfblocks;
        proto = proto.next();
        assert_eq!(proto, ProtocolType::Sixel);
        proto = proto.next();
        assert_eq!(proto, ProtocolType::Kitty);
        proto = proto.next();
        assert_eq!(proto, ProtocolType::Iterm2);
        proto = proto.next();
        assert_eq!(proto, ProtocolType::Halfblocks);
    }

    #[test]
    fn test_from_query_stdio_no_hang() {
        let _ = Picker::from_query_stdio();
    }

    #[test]
    fn test_interpret_parser_responses_text_sizing_protocol() {
        let (_, _, caps) = interpret_parser_responses(vec![
            // Example response from Kitty.
            Response::CursorPositionReport(1, 1),
            Response::CursorPositionReport(3, 1),
            Response::CursorPositionReport(5, 1),
        ])
        .unwrap();
        assert!(caps.contains(&Capability::TextSizingProtocol));
    }

    #[test]
    fn test_interpret_parser_responses_text_sizing_protocol_incomplete() {
        let (_, _, caps) = interpret_parser_responses(vec![
            // Example response from Foot, notably moves 2 columns only on `w=2` query, but not
            // `s=2`.
            Response::CursorPositionReport(1, 22),
            Response::CursorPositionReport(3, 22),
            Response::CursorPositionReport(4, 22),
        ])
        .unwrap();
        assert!(!caps.contains(&Capability::TextSizingProtocol));
    }

    #[cfg(not(windows))]
    #[test]
    fn deadline_read_times_out_without_spawning_a_reader() {
        use std::{
            io::Write,
            os::unix::net::UnixStream,
            time::{Duration, Instant},
        };

        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        let mut buffer = [0_u8; 1];
        let error = super::read_with_deadline(
            &mut reader,
            &mut buffer,
            Instant::now() + Duration::from_millis(5),
        )
        .unwrap_err();
        assert!(matches!(error, crate::errors::Errors::NoStdinResponse));

        writer.write_all(b"x").unwrap();
        let read = super::read_with_deadline(
            &mut reader,
            &mut buffer,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(read, 1);
        assert_eq!(buffer, *b"x");
    }
}
