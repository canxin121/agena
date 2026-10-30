//! Static analysis of shell command shapes for permissions and tooling.

mod commands;

/// Parsed summary of a shell command used for policy and result handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandAnalysis {
    pub primary_command: Option<String>,
    pub subcommand: Option<String>,
    pub args: Vec<String>,
    pub classification: CommandClassification,
    /// Exit semantics are only attributable to a single, unexpanded command.
    pub single_command: bool,
}

/// Conservative filesystem-mutation classification for a shell command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandClassification {
    ReadOnly,
    Mutating { reason: String },
    Unknown,
}

impl CommandClassification {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Mutating { .. } => "mutating",
            Self::Unknown => "unknown",
        }
    }
}

/// Interpretation of a shell process exit code for a known command shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitInterpretation {
    Success,
    NoMatches,
    DifferencesFound,
    Error,
}

impl ExitInterpretation {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::NoMatches => "no_matches",
            Self::DifferencesFound => "differences_found",
            Self::Error => "error",
        }
    }
}

/// Interpret a process exit code using the command shape recognized by the
/// shell-tool contract.
pub fn interpret_exit_code(
    analysis: &CommandAnalysis,
    exit_code: i32,
    timed_out: bool,
) -> ExitInterpretation {
    if timed_out {
        return ExitInterpretation::Error;
    }
    if exit_code == 0 {
        return ExitInterpretation::Success;
    }

    if !analysis.single_command {
        return ExitInterpretation::Error;
    }

    match (
        analysis.primary_command.as_deref(),
        analysis.subcommand.as_deref(),
        exit_code,
    ) {
        (Some("grep" | "rg"), _, 1) | (Some("git"), Some("grep"), 1) => {
            ExitInterpretation::NoMatches
        }
        (Some("jq"), _, 1 | 4) if commands::jq_exit_status(&analysis.args) => {
            ExitInterpretation::NoMatches
        }
        (Some("diff" | "cmp"), _, 1) => ExitInterpretation::DifferencesFound,
        (Some("git"), Some("diff"), 1)
            if analysis
                .args
                .iter()
                .any(|arg| arg == "--exit-code" || arg == "--quiet") =>
        {
            ExitInterpretation::DifferencesFound
        }
        _ => ExitInterpretation::Error,
    }
}

/// Tokenize a shell command while preserving quoted spans and command
/// separators used by the conservative policy parser.
pub fn shell_tokens(command: &str) -> Vec<String> {
    commands::tokens(command)
}

/// Split shell tokens into separator-delimited command segments.
pub fn command_segments(tokens: &[String]) -> Vec<&[String]> {
    let mut segments = Vec::new();
    let mut start = 0;
    for (index, token) in tokens.iter().enumerate() {
        if is_separator(token.as_str()) {
            if start < index {
                segments.push(&tokens[start..index]);
            }
            start = index + 1;
        }
    }
    if start < tokens.len() {
        segments.push(&tokens[start..]);
    }
    segments
}

/// Extract the executable, Git subcommand, and arguments from the first
/// command segment after simple environment assignments/wrappers.
pub fn first_command(tokens: &[String]) -> (Option<String>, Option<String>, Vec<String>) {
    commands::first_command(tokens)
}

fn is_separator(token: &str) -> bool {
    matches!(token, "\0;" | "\0|" | "\0&")
}

/// Detect a shell output-redirection operator outside quotes.
///
/// Redirects whose target is `/dev/null` (for example `2>/dev/null` or
/// `&>/dev/null`) are treated as discards, not file writes: the standard
/// pattern of silencing output must not require a declared filesystem effect.
pub fn contains_write_redirection(command: &str) -> bool {
    let mut single_quote = false;
    let mut double_quote = false;
    let mut escape = false;
    let mut chars = command.chars().peekable();

    while let Some(ch) = chars.next() {
        if escape {
            escape = false;
            continue;
        }
        match ch {
            '\\' if !single_quote => escape = true,
            '\'' if !double_quote => single_quote = !single_quote,
            '"' if !single_quote => double_quote = !double_quote,
            '>' if !single_quote && !double_quote && chars.peek() != Some(&'&') => {
                if redirect_target_is_dev_null(&mut chars) {
                    continue;
                }
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Return the first output-redirection target that is not `/dev/null`, or
/// `None` when every redirection discards to `/dev/null`.
pub fn first_write_redirection_target(command: &str) -> Option<String> {
    let mut single_quote = false;
    let mut double_quote = false;
    let mut escape = false;
    let mut chars = command.chars().peekable();

    while let Some(ch) = chars.next() {
        if escape {
            escape = false;
            continue;
        }
        match ch {
            '\\' if !single_quote => escape = true,
            '\'' if !double_quote => single_quote = !single_quote,
            '"' if !single_quote => double_quote = !double_quote,
            '>' if !single_quote && !double_quote && chars.peek() != Some(&'&') => {
                skip_redirect_whitespace(&mut chars);
                if chars.peek() == Some(&'>') {
                    chars.next();
                    skip_redirect_whitespace(&mut chars);
                }
                let target = redirect_target(&mut chars);
                if !target.raw.is_empty() && target.literal.as_deref() != Some("/dev/null") {
                    return Some(target.raw);
                }
            }
            _ => {}
        }
    }
    None
}

/// Consume a redirection target (`2>`, `>`, `>>`, `&>`, `2>>` handled by the
/// caller's `>` matching) and return whether the target is `/dev/null`.
fn redirect_target_is_dev_null(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> bool {
    skip_redirect_whitespace(chars);
    if chars.peek() == Some(&'>') {
        // `>>` append redirect; keep scanning past the second `>`.
        chars.next();
        skip_redirect_whitespace(chars);
    }
    redirect_target(chars).literal.as_deref() == Some("/dev/null")
}

fn skip_redirect_whitespace(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
        chars.next();
    }
}

struct RedirectTarget {
    raw: String,
    literal: Option<String>,
}

/// Collect one shell word after an already-consumed redirect operator.
///
/// Parentheses terminate an unquoted word, including the closing `)` in
/// `$(command 2>/dev/null)`. Quoted or escaped parentheses remain filename
/// characters. Only a complete literal word may qualify as the null sink;
/// expansions and unterminated quotes remain conservative file effects.
fn redirect_target(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> RedirectTarget {
    let mut raw = String::new();
    let mut literal = String::new();
    let mut quote = None;
    let mut complete_literal = true;
    while let Some(&ch) = chars.peek() {
        if quote.is_none()
            && (ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '<' | '>' | '(' | ')'))
        {
            break;
        }
        chars.next();
        raw.push(ch);
        match (quote, ch) {
            (None, '\'' | '"') => quote = Some(ch),
            (Some(active), closing) if active == closing => quote = None,
            (Some('\''), _) => literal.push(ch),
            (_, '\\') => {
                let Some(escaped) = chars.next() else {
                    complete_literal = false;
                    break;
                };
                raw.push(escaped);
                if escaped != '\n' {
                    // In double quotes, backslash only quotes these shell
                    // characters; other backslashes remain literal.
                    if quote == Some('"') && !matches!(escaped, '$' | '`' | '"' | '\\') {
                        literal.push('\\');
                    }
                    literal.push(escaped);
                }
            }
            (_, '$' | '`') => {
                complete_literal = false;
                literal.push(ch);
            }
            _ => literal.push(ch),
        }
    }
    RedirectTarget {
        raw,
        literal: (complete_literal && quote.is_none()).then_some(literal),
    }
}

/// Detect a shell input-redirection operator outside quotes. Reading from
/// `/dev/null` is treated as no file read: `< /dev/null` must not require a
/// declared filesystem effect.
pub fn contains_input_redirection(command: &str) -> bool {
    let mut single_quote = false;
    let mut double_quote = false;
    let mut escape = false;
    let mut chars = command.chars().peekable();

    while let Some(ch) = chars.next() {
        if escape {
            escape = false;
            continue;
        }
        match ch {
            '\\' if !single_quote => escape = true,
            '\'' if !double_quote => single_quote = !single_quote,
            '"' if !single_quote => double_quote = !double_quote,
            '<' if !single_quote && !double_quote => {
                if chars.peek() == Some(&'<') {
                    chars.next();
                    continue;
                }
                if redirect_target_is_dev_null(&mut chars) {
                    continue;
                }
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Return a conservative reason when a shell command appears to contact a
/// network target and therefore needs a declared network effect.
pub fn network_command_reason(command: &str) -> Option<String> {
    let tokens = shell_tokens(command);
    for segment in command_segments(tokens.as_slice()) {
        if let Some(reason) = network_segment_reason(segment) {
            return Some(reason);
        }
    }
    None
}

/// Classify a shell command conservatively as read-only, mutating, or unknown.
pub fn analyze_command(command: &str) -> CommandAnalysis {
    let tokens = shell_tokens(command);
    let (primary_command, subcommand, args) = first_command(tokens.as_slice());
    let classification = classify_command(command, tokens.as_slice());
    CommandAnalysis {
        primary_command,
        subcommand,
        args,
        classification,
        single_command: command_segments(&tokens).len() == 1
            && !tokens.iter().any(|token| is_separator(token))
            && !commands::has_uninspected_syntax(command),
    }
}

/// Return the mutation reason when a command can be classified as mutating.
pub fn mutating_command_reason(command: &str) -> Option<String> {
    match analyze_command(command).classification {
        CommandClassification::Mutating { reason } => Some(reason),
        CommandClassification::ReadOnly | CommandClassification::Unknown => None,
    }
}

/// Return a conservative reason when a shell command appears to read or write
/// the local filesystem and therefore needs declared filesystem effects.
pub fn filesystem_command_reason(command: &str) -> Option<String> {
    if let Some(reason) = mutating_command_reason(command) {
        return Some(reason);
    }
    if contains_input_redirection(command) {
        return Some("uses shell input redirection".to_string());
    }
    let tokens = shell_tokens(command);
    for segment in command_segments(tokens.as_slice()) {
        if let Some(reason) = filesystem_segment_reason(segment) {
            return Some(reason);
        }
    }
    None
}

/// Return a conservative reason when a shell command with no declared
/// filesystem effects is still unsafe to run: it provably mutates the
/// filesystem, reads through shell input redirection, or reads/writes explicit
/// files via curl. Generic "may read or write local files" hints for
/// interpreters and build tools (node, python, uv, cargo, ...) are
/// intentionally NOT included: those commands can legitimately have no file
/// effects beyond the executables they invoke, and requiring an enumeration
/// forces models to over-declare interpreter/runtime paths.
pub fn filesystem_effects_required_reason(command: &str) -> Option<String> {
    if let Some(reason) = mutating_command_reason(command) {
        return Some(reason);
    }
    if contains_input_redirection(command) {
        return Some("uses shell input redirection".to_string());
    }
    let tokens = shell_tokens(command);
    for segment in command_segments(tokens.as_slice()) {
        let (primary, _, args) = first_command(segment);
        let is_curl = primary.as_deref().is_some_and(|program| {
            matches!(
                program.rsplit(['/', '\\']).next(),
                Some("curl" | "curl.exe")
            )
        });
        if is_curl && let Some(reason) = curl_filesystem_reason(args.as_slice()) {
            return Some(reason);
        }
    }
    None
}

fn filesystem_segment_reason(tokens: &[String]) -> Option<String> {
    let (Some(primary), subcommand, args) = first_command(tokens) else {
        return Some("contains a command that could not be classified".to_string());
    };
    let primary_norm = primary.to_ascii_lowercase();
    let subcommand_norm = subcommand.as_deref().map(str::to_ascii_lowercase);
    if primary_norm == "curl" {
        return curl_filesystem_reason(args.as_slice());
    }
    if matches!(
        primary_norm.as_str(),
        "invoke-webrequest" | "invoke-restmethod"
    ) {
        return powershell_web_cmdlet_filesystem_reason(primary.as_str(), args.as_slice());
    }
    if primary_norm == "start-bitstransfer" {
        return Some("invokes Start-BitsTransfer which may read or write local files".to_string());
    }
    if matches!(
        primary_norm.as_str(),
        "pwd"
            | "echo"
            | "printf"
            | "env"
            | "printenv"
            | "date"
            | "uname"
            | "whoami"
            | "id"
            | "hostname"
            | "sleep"
            | "true"
            | "false"
            | "yes"
            | "seq"
            | "ps"
            | "kill"
            | "get-location"
            | "ping"
            | "traceroute"
            | "dig"
            | "nslookup"
            | "host"
            | "telnet"
            | "nc"
            | "ncat"
            | "socat"
            | "test-netconnection"
    ) {
        return None;
    }
    if matches!(
        primary_norm.as_str(),
        "cat"
            | "ls"
            | "find"
            | "grep"
            | "rg"
            | "diff"
            | "cmp"
            | "head"
            | "tail"
            | "sed"
            | "awk"
            | "sort"
            | "uniq"
            | "wc"
            | "stat"
            | "file"
            | "basename"
            | "dirname"
            | "realpath"
            | "which"
            | "type"
            | "du"
            | "tree"
            | "wget"
    ) {
        return Some(format!("invokes filesystem command '{primary}'"));
    }
    if primary_norm == "git" {
        return Some(match subcommand_norm.as_deref() {
            Some(subcommand) => format!("invokes git subcommand '{subcommand}'"),
            None => "invokes git".to_string(),
        });
    }
    if matches!(
        primary_norm.as_str(),
        "python"
            | "python3"
            | "node"
            | "perl"
            | "ruby"
            | "php"
            | "lua"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "pwsh"
            | "powershell"
            | "cmd"
            | "make"
            | "just"
            | "cargo"
            | "go"
            | "java"
            | "javac"
            | "rustc"
            | "npm"
            | "pnpm"
            | "yarn"
            | "bun"
            | "scp"
            | "sftp"
            | "rsync"
            | "ssh"
    ) {
        return Some(format!(
            "invokes '{primary}' which may read or write local files"
        ));
    }
    Some(format!(
        "invokes command '{primary}' that is not proven to avoid filesystem access"
    ))
}

fn curl_short_option_takes_argument(flag: char) -> bool {
    matches!(
        flag,
        'A' | 'b'
            | 'c'
            | 'C'
            | 'd'
            | 'D'
            | 'e'
            | 'E'
            | 'F'
            | 'h'
            | 'H'
            | 'K'
            | 'm'
            | 'o'
            | 'P'
            | 'Q'
            | 'r'
            | 't'
            | 'T'
            | 'u'
            | 'U'
            | 'w'
            | 'x'
            | 'X'
            | 'y'
            | 'Y'
            | 'z'
    )
}

fn curl_option_takes_next_argument(option: &str) -> bool {
    (option.len() == 2
        && option.starts_with('-')
        && option
            .chars()
            .nth(1)
            .is_some_and(curl_short_option_takes_argument))
        || matches!(
            option,
            "--output"
                | "--upload-file"
                | "--config"
                | "--cookie-jar"
                | "--cookie"
                | "--cacert"
                | "--cert"
                | "--key"
                | "--netrc-file"
                | "--etag-compare"
                | "--data"
                | "--data-ascii"
                | "--data-binary"
                | "--data-raw"
                | "--json"
                | "--form"
                | "--header"
                | "--proxy-header"
                | "--write-out"
                | "--user"
                | "--proxy-user"
                | "--user-agent"
                | "--request"
                | "--url"
                | "--max-time"
                | "--connect-timeout"
                | "--proxy"
                | "--referer"
        )
}

fn curl_filesystem_reason(args: &[String]) -> Option<String> {
    if args.iter().any(|arg| arg.starts_with("file://")) {
        return Some("invokes curl with a local file URL".to_string());
    }
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "--" {
            break;
        }
        let mut consumes_next_argument = curl_option_takes_next_argument(arg);
        // Curl accepts short-option clusters such as `-sSo /dev/null`.
        // Stop at the first option taking a value: the remainder belongs to
        // that value, so `-Atrace` cannot become a trace-output option.
        if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 2 {
            for (offset, flag) in arg[1..].char_indices() {
                if matches!(flag, 'O' | 'R' | 'n') {
                    let option = vec![format!("-{flag}")];
                    if let Some(reason) = curl_filesystem_reason(&option) {
                        return Some(reason);
                    }
                }
                if curl_short_option_takes_argument(flag) {
                    let tail = &arg[offset + 1 + flag.len_utf8()..];
                    consumes_next_argument = tail.is_empty();
                    let value = if tail.is_empty() {
                        args.get(index + 1).map(String::as_str)
                    } else {
                        Some(tail)
                    };
                    if matches!(
                        flag,
                        'b' | 'c' | 'd' | 'D' | 'E' | 'F' | 'H' | 'K' | 'o' | 'T' | 'w'
                    ) {
                        let mut option = vec![format!("-{flag}")];
                        option.extend(value.map(str::to_owned));
                        if let Some(reason) = curl_filesystem_reason(&option) {
                            return Some(reason);
                        }
                    }
                    break;
                }
            }
        }
        let (option, attached) = arg
            .split_once('=')
            .map_or((arg, None), |(name, value)| (name, Some(value)));
        if matches!(
            option,
            "-D" | "--dump-header"
                | "--trace"
                | "--trace-ascii"
                | "--stderr"
                | "--etag-save"
                | "--libcurl"
                | "--hsts"
                | "--alt-svc"
        ) {
            let target = attached.or_else(|| args.get(index + 1).map(String::as_str));
            let supports_stdout = matches!(
                option,
                "-D" | "--dump-header" | "--trace" | "--trace-ascii" | "--stderr" | "--libcurl"
            );
            if target != Some("/dev/null") && !(supports_stdout && target == Some("-")) {
                return Some(format!(
                    "invokes curl option '{option}' that writes a local file"
                ));
            }
            // Discarding the body or another output is only one no-op effect.
            // Continue checking every other output, upload and command.
            index += if attached.is_some() { 1 } else { 2 };
            continue;
        }
        match arg {
            "-o" | "--output"
                if args
                    .get(index + 1)
                    .is_none_or(|value| !matches!(value.as_str(), "-" | "/dev/null")) =>
            {
                return Some(match args.get(index + 1) {
                    Some(target) if target != "-" => {
                        format!("invokes curl output option that writes a local file '{target}'")
                    }
                    _ => "invokes curl output option that writes a local file".to_string(),
                });
            }
            "-O" | "--remote-name" | "--remote-name-all" => {
                return Some(
                    "invokes curl remote-name output which writes to the current directory"
                        .to_string(),
                );
            }
            "-R" | "--remote-time" | "--remove-on-error" | "--no-clobber" | "--xattr" => {
                return Some(format!(
                    "invokes curl option '{arg}' that can change the output file or its metadata"
                ));
            }
            "--output-dir" => {
                return Some("invokes curl output-directory option for local files".to_string());
            }
            "-n" | "--netrc" | "--netrc-optional" => {
                return Some("invokes curl netrc option that reads a local file".to_string());
            }
            "-T" | "--upload-file" => {
                return Some(match args.get(index + 1) {
                    Some(target) => {
                        format!("invokes curl upload option that reads a local file '{target}'")
                    }
                    None => "invokes curl upload option that reads a local file".to_string(),
                });
            }
            "-K" | "--config" => {
                return Some(match args.get(index + 1) {
                    Some(target) => {
                        format!("invokes curl config option that reads a local file '{target}'")
                    }
                    None => "invokes curl config option that reads a local file".to_string(),
                });
            }
            "-c" | "--cookie-jar"
                if args.get(index + 1).is_none_or(|value| value != "/dev/null") =>
            {
                return Some(match args.get(index + 1) {
                    Some(target) => format!(
                        "invokes curl cookie-jar option that writes a local file '{target}'"
                    ),
                    None => "invokes curl cookie-jar option that writes a local file".to_string(),
                });
            }
            "--cacert" | "-E" | "--cert" | "--key" | "--netrc-file" | "--etag-compare" => {
                return Some(match args.get(index + 1) {
                    Some(target) => {
                        format!("invokes curl option '{arg}' that reads a local file '{target}'")
                    }
                    None => format!("invokes curl option '{arg}' that reads a local file"),
                });
            }
            "-H" | "--header" | "--proxy-header"
                if args
                    .get(index + 1)
                    .is_some_and(|value| value.starts_with('@')) =>
            {
                return Some("invokes curl header option that reads a local file".to_string());
            }
            "-w" | "--write-out"
                if args
                    .get(index + 1)
                    .is_some_and(|value| value.starts_with('@') || value.contains("%output{")) =>
            {
                return Some(
                    "invokes curl write-out format that accesses a local file".to_string(),
                );
            }
            "-b" | "--cookie"
                if args
                    .get(index + 1)
                    .is_some_and(|value| curl_cookie_option_uses_file(value)) =>
            {
                return Some("invokes curl cookie option that reads from a local file".to_string());
            }
            "-d" | "--data" | "--data-ascii" | "--data-binary" | "--data-raw" | "--json"
                if args
                    .get(index + 1)
                    .is_some_and(|value| curl_data_option_uses_file(value)) =>
            {
                return Some(format!(
                    "invokes curl option '{arg}' that reads request data from a local file"
                ));
            }
            "-F" | "--form"
                if args
                    .get(index + 1)
                    .is_some_and(|value| curl_form_option_uses_file(value)) =>
            {
                return Some("invokes curl form upload that reads a local file".to_string());
            }
            _ => {
                if arg.starts_with("--output-dir=") || arg.starts_with("--expand-output") {
                    return Some(
                        "invokes curl output option with a dynamic local destination".to_string(),
                    );
                }
                if (arg.starts_with("-o")
                    && arg.len() > 2
                    && !matches!(&arg[2..], "-" | "/dev/null"))
                    || (arg.starts_with("--output=")
                        && !matches!(arg, "--output=-" | "--output=/dev/null"))
                {
                    let target = arg
                        .split_once('=')
                        .map(|(_, value)| value)
                        .unwrap_or(&arg[2..]);
                    return Some(format!(
                        "invokes curl output option that writes a local file '{target}'"
                    ));
                }
                if (arg.starts_with("-T") && arg.len() > 2) || arg.starts_with("--upload-file=") {
                    let target = arg
                        .split_once('=')
                        .map(|(_, value)| value)
                        .unwrap_or(&arg[2..]);
                    return Some(format!(
                        "invokes curl upload option that reads a local file '{target}'"
                    ));
                }
                if (arg.starts_with("-K") && arg.len() > 2) || arg.starts_with("--config=") {
                    let target = arg
                        .split_once('=')
                        .map(|(_, value)| value)
                        .unwrap_or(&arg[2..]);
                    return Some(format!(
                        "invokes curl config option that reads a local file '{target}'"
                    ));
                }
                if (arg.starts_with("-c") && arg.len() > 2 && &arg[2..] != "/dev/null")
                    || (arg.starts_with("--cookie-jar=") && arg != "--cookie-jar=/dev/null")
                {
                    let target = arg
                        .split_once('=')
                        .map(|(_, value)| value)
                        .unwrap_or(&arg[2..]);
                    return Some(format!(
                        "invokes curl cookie-jar option that writes a local file '{target}'"
                    ));
                }
                if arg.starts_with("--cacert=")
                    || arg.starts_with("--cert=")
                    || arg.starts_with("--key=")
                    || arg.starts_with("--netrc-file=")
                    || arg.starts_with("--etag-compare=")
                {
                    let target = arg.split_once('=').map(|(_, value)| value).unwrap_or("");
                    return Some(format!(
                        "invokes curl option '{arg}' that reads a local file '{target}'"
                    ));
                }
                if ["--header=", "--proxy-header="].iter().any(|prefix| {
                    arg.strip_prefix(prefix)
                        .is_some_and(|value| value.starts_with('@'))
                }) {
                    return Some("invokes curl header option that reads a local file".to_string());
                }
                if arg
                    .strip_prefix("--write-out=")
                    .is_some_and(|value| value.starts_with('@') || value.contains("%output{"))
                {
                    return Some(
                        "invokes curl write-out format that accesses a local file".to_string(),
                    );
                }
                if (arg.starts_with("-b")
                    && arg.len() > 2
                    && curl_cookie_option_uses_file(&arg[2..]))
                    || (arg.starts_with("--cookie=")
                        && curl_cookie_option_uses_file(
                            arg.split_once('=').map(|(_, value)| value).unwrap_or(""),
                        ))
                {
                    return Some(
                        "invokes curl cookie option that reads from a local file".to_string(),
                    );
                }
                let value = arg.split_once('=').map(|(_, value)| value).unwrap_or("");
                if (arg.starts_with("-d") && arg.len() > 2 && curl_data_option_uses_file(&arg[2..]))
                    || ((arg.starts_with("--data=")
                        || arg.starts_with("--data-ascii=")
                        || arg.starts_with("--data-binary=")
                        || arg.starts_with("--data-raw=")
                        || arg.starts_with("--json="))
                        && curl_data_option_uses_file(value))
                {
                    return Some(
                        "invokes curl data option that reads request data from a local file"
                            .to_string(),
                    );
                }
                if (arg.starts_with("-F") && arg.len() > 2 && curl_form_option_uses_file(&arg[2..]))
                    || (arg.starts_with("--form=") && curl_form_option_uses_file(value))
                {
                    return Some("invokes curl form upload that reads a local file".to_string());
                }
            }
        }
        index += if consumes_next_argument { 2 } else { 1 };
    }
    None
}

fn classify_command(command: &str, tokens: &[String]) -> CommandClassification {
    if contains_write_redirection(command) {
        let target = first_write_redirection_target(command);
        let reason = match target {
            Some(path) => format!("uses shell output redirection writing '{path}'"),
            None => "uses shell output redirection".to_string(),
        };
        return CommandClassification::Mutating { reason };
    }
    let segments = command_segments(tokens);
    if segments.is_empty() {
        return CommandClassification::Unknown;
    }
    let mut saw_unknown = commands::has_uninspected_syntax(command);
    for segment in segments {
        match classify_segment(segment) {
            CommandClassification::Mutating { reason } => {
                return CommandClassification::Mutating { reason };
            }
            CommandClassification::Unknown => saw_unknown = true,
            CommandClassification::ReadOnly => {}
        }
    }
    if saw_unknown {
        CommandClassification::Unknown
    } else {
        CommandClassification::ReadOnly
    }
}

fn classify_segment(tokens: &[String]) -> CommandClassification {
    let (Some(primary), subcommand, args) = first_command(tokens) else {
        return CommandClassification::Unknown;
    };
    if let Some(classification) = commands::classify(&primary, subcommand.as_deref(), &args) {
        return classification;
    }
    if is_obvious_write_command(primary.as_str(), subcommand.as_deref(), args.as_slice()) {
        return CommandClassification::Mutating {
            reason: format!("invokes mutating command '{primary}'"),
        };
    }
    if is_known_read_only_command(primary.as_str(), subcommand.as_deref(), args.as_slice()) {
        return CommandClassification::ReadOnly;
    }
    CommandClassification::Unknown
}

fn is_known_read_only_command(primary: &str, subcommand: Option<&str>, args: &[String]) -> bool {
    if matches!(
        primary,
        "cat"
            | "pwd"
            | "ls"
            | "find"
            | "grep"
            | "rg"
            | "diff"
            | "cmp"
            | "head"
            | "tail"
            | "sed"
            | "awk"
            | "sort"
            | "uniq"
            | "wc"
            | "stat"
            | "file"
            | "basename"
            | "dirname"
            | "realpath"
            | "echo"
            | "printf"
            | "env"
            | "which"
            | "type"
            | "du"
            | "ps"
    ) {
        return !matches!(primary, "sed") || !args.iter().any(|arg| is_in_place_flag(arg));
    }
    primary == "git"
        && matches!(
            subcommand,
            Some("status" | "diff" | "grep" | "show" | "log" | "rev-parse")
        )
}

fn is_obvious_write_command(primary: &str, subcommand: Option<&str>, args: &[String]) -> bool {
    if matches!(
        primary,
        "touch"
            | "mkdir"
            | "rmdir"
            | "rm"
            | "mv"
            | "cp"
            | "install"
            | "chmod"
            | "chown"
            | "ln"
            | "tee"
            | "dd"
            | "truncate"
    ) {
        return true;
    }
    if matches!(primary, "sed" | "perl") && args.iter().any(|arg| is_in_place_flag(arg)) {
        return true;
    }
    primary == "git"
        && matches!(
            subcommand,
            Some(
                "apply"
                    | "am"
                    | "add"
                    | "checkout"
                    | "switch"
                    | "restore"
                    | "commit"
                    | "merge"
                    | "rebase"
                    | "cherry-pick"
                    | "revert"
                    | "clean"
                    | "stash"
            )
        )
}

fn is_in_place_flag(arg: &str) -> bool {
    arg == "-i" || (arg.starts_with("-i") && arg.len() > 2)
}

/// Whether a curl cookie option names a local file rather than an inline cookie.
pub fn curl_cookie_option_uses_file(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && !trimmed.contains('=')
}

/// Whether a curl data option reads its request body from a local file.
pub fn curl_data_option_uses_file(value: &str) -> bool {
    value.trim_start().starts_with('@')
}

/// Whether a curl form option uploads a local file.
pub fn curl_form_option_uses_file(value: &str) -> bool {
    value.contains('@') || value.contains('<')
}

/// Return the local filesystem reason exposed by PowerShell web cmdlets.
pub fn powershell_web_cmdlet_filesystem_reason(primary: &str, args: &[String]) -> Option<String> {
    for arg in args {
        let lower = arg.to_ascii_lowercase();
        if lower == "-outfile" || lower.starts_with("-outfile:") {
            return Some(format!(
                "invokes {primary} with -OutFile which writes a local file"
            ));
        }
        if lower == "-infile" || lower.starts_with("-infile:") {
            return Some(format!(
                "invokes {primary} with -InFile which reads a local file"
            ));
        }
    }
    None
}

fn network_segment_reason(tokens: &[String]) -> Option<String> {
    let (Some(primary), subcommand, args) = first_command(tokens) else {
        return None;
    };
    let primary_norm = primary.to_ascii_lowercase();
    let subcommand_norm = subcommand.as_deref().map(str::to_ascii_lowercase);

    if matches!(
        primary_norm.as_str(),
        "curl"
            | "wget"
            | "ssh"
            | "scp"
            | "sftp"
            | "rsync"
            | "telnet"
            | "nc"
            | "ncat"
            | "socat"
            | "ping"
            | "traceroute"
            | "dig"
            | "nslookup"
            | "host"
            | "ftp"
            | "tftp"
            | "invoke-webrequest"
            | "invoke-restmethod"
            | "start-bitstransfer"
            | "test-netconnection"
    ) {
        return Some(format!("invokes network command '{primary}'"));
    }

    if primary_norm == "git"
        && matches!(
            subcommand_norm.as_deref(),
            Some("fetch" | "pull" | "push" | "ls-remote")
        )
    {
        return Some(format!(
            "invokes git subcommand '{}' that may contact a remote",
            subcommand.unwrap_or_default()
        ));
    }

    if primary_norm == "git"
        && subcommand_norm.as_deref() == Some("clone")
        && args
            .iter()
            .filter(|arg| !arg.starts_with('-'))
            .any(|arg| looks_like_remote_target(arg))
    {
        return Some("invokes git clone with a remote target".to_string());
    }
    None
}

fn looks_like_remote_target(arg: &str) -> bool {
    let trimmed = arg.trim();
    !trimmed.is_empty()
        && (trimmed.contains("://")
            || trimmed.starts_with("ssh://")
            || trimmed.starts_with("git@")
            || trimmed.starts_with("http://")
            || trimmed.starts_with("https://"))
}

#[cfg(test)]
mod null_sink_tests {
    use super::*;

    #[test]
    fn redirection_recognizes_literal_null_at_shell_boundaries() {
        for command in [
            "status=$(printf ok 2>/dev/null)",
            "(printf ok >/dev/null)",
            "printf ok >/dev/null; printf done",
            "printf ok 2>>/dev/null",
            "printf ok &>/dev/null",
            "printf ok >'/dev/null'",
            r#"printf ok >"/dev/null""#,
            r#"printf ok >/dev/"null""#,
            r"printf ok >/dev/\null",
            "printf ok >/dev/\\\nnull",
        ] {
            assert!(!contains_write_redirection(command), "{command}");
            assert_eq!(first_write_redirection_target(command), None, "{command}");
        }
        for command in [
            "cat </dev/null",
            "(cat <'/dev/null')",
            r#"cat <"/dev/null""#,
        ] {
            assert!(!contains_input_redirection(command), "{command}");
        }
    }

    #[test]
    fn null_like_paths_and_expansions_remain_file_effects() {
        for target in [
            "'/dev/null)'",
            r#""/dev/null)""#,
            r"/dev/null\)",
            "/dev/nullx",
            "/dev/null/file",
            "/dev/zero",
            "./dev/null",
            "${SINK}",
            r#"/dev/null"$suffix""#,
            r#""/dev\/null""#,
            "'/dev/null",
            "/dev/null\\",
        ] {
            let output = format!("printf ok >{target}");
            assert!(contains_write_redirection(&output), "{output}");
            assert!(
                first_write_redirection_target(&output).is_some(),
                "{output}"
            );
            assert!(
                filesystem_effects_required_reason(&output).is_some(),
                "{output}"
            );
            let input = format!("cat <{target}");
            assert!(contains_input_redirection(&input), "{input}");
        }
    }

    #[test]
    fn null_redirection_does_not_hide_later_writes() {
        for command in [
            "printf ok 2>/dev/null >result.txt",
            "status=$(printf ok 2>/dev/null); printf done >result.txt",
            "(printf ok >/dev/null) >result.txt",
        ] {
            assert!(contains_write_redirection(command), "{command}");
            assert_eq!(
                first_write_redirection_target(command).as_deref(),
                Some("result.txt"),
                "{command}"
            );
            assert!(
                filesystem_effects_required_reason(command).is_some(),
                "{command}"
            );
        }
        for command in [
            "printf ok >/dev/null; touch result.txt",
            "curl -o /dev/null https://example.invalid; touch result.txt",
            "curl -o /dev/null https://example.invalid | tee result.txt",
            "curl -o /dev/null https://example.invalid && curl -o result.txt https://example.invalid",
        ] {
            assert!(
                filesystem_effects_required_reason(command).is_some(),
                "{command}"
            );
        }
    }

    #[test]
    fn null_redirection_does_not_upgrade_uninspected_syntax() {
        let analysis = analyze_command("status=$(printf ok 2>/dev/null)");
        assert_eq!(analysis.classification, CommandClassification::Unknown);
        assert!(!analysis.single_command);
        assert_eq!(
            interpret_exit_code(&analysis, 1, false),
            ExitInterpretation::Error
        );
    }

    #[test]
    fn curl_body_can_discard_to_exact_null_or_stdout() {
        for options in [
            "-o /dev/null",
            "-o/dev/null",
            "--output /dev/null",
            "--output=/dev/null",
            "-o '/dev/null'",
            r#"-o "/dev/null""#,
            r#"-o /dev/"null""#,
            "-sSo /dev/null",
            "-sSfo/dev/null",
            "-o -",
            "-o-",
            "--output=-",
            "-o /dev/null -w '%{http_code}'",
            "-o /dev/null -D /dev/null --trace=/dev/null -c /dev/null",
        ] {
            let command = format!("curl {options} https://example.invalid");
            assert_eq!(
                filesystem_effects_required_reason(&command),
                None,
                "{command}"
            );
            assert!(network_command_reason(&command).is_some(), "{command}");
        }
    }

    #[test]
    fn curl_null_like_output_paths_still_require_declarations() {
        for target in [
            "/dev/nullx",
            "/dev/null/file",
            "/dev/zero",
            "./dev/null",
            "'/dev/null)'",
            r#""/dev\/null""#,
            "$SINK",
            r#"/dev/null"$suffix""#,
            "result.txt",
        ] {
            for output in ["-o ", "-o", "--output ", "--output="] {
                let command = format!("curl {output}{target} https://example.invalid");
                assert!(
                    filesystem_effects_required_reason(&command).is_some(),
                    "{command}"
                );
            }
        }
    }

    #[test]
    fn curl_discard_does_not_hide_other_file_outputs() {
        for options in [
            "-o result.txt",
            "-sSo result.txt",
            "-D headers.txt",
            "-sDheaders.txt",
            "--dump-header=headers.txt",
            "--trace trace.log",
            "--trace-ascii=trace.log",
            "--stderr errors.log",
            "--etag-save etag.txt",
            "--etag-save -",
            "--libcurl generated.c",
            "--hsts hsts.txt",
            "--alt-svc=altsvc.txt",
            "-c cookies.txt",
            "-scookies.txt",
            "--cookie-jar=cookies.txt",
            "-O",
            "-sO",
            "--remote-name-all",
            "--output-dir files",
            "--output-dir=files",
            "--expand-output '{{destination}}'",
            "--no-clobber",
            "--remove-on-error",
            "--remote-time",
            "-sR",
            "--xattr",
            "-w '%output{result.txt}%{http_code}'",
            "--write-out=%output{result.txt}",
        ] {
            let command = format!("curl -o /dev/null {options} https://example.invalid");
            assert!(
                filesystem_effects_required_reason(&command).is_some(),
                "{command}"
            );
        }
    }

    #[test]
    fn curl_discard_does_not_hide_explicit_file_reads() {
        for options in [
            "-T input.txt",
            "--upload-file=input.txt",
            "-K curl.conf",
            "--config=curl.conf",
            "-d @body.txt",
            "--data-binary=@body.txt",
            "-F image=@photo.png",
            "-b cookies.txt",
            "-H @headers.txt",
            "-sH@headers.txt",
            "--header=@headers.txt",
            "--proxy-header @headers.txt",
            "-w @format.txt",
            "--write-out=@format.txt",
            "--netrc",
            "--netrc-file=netrc.txt",
            "--etag-compare etag.txt",
            "--cert cert.pem",
            "-sEcert.pem",
            "file:///tmp/input.txt",
        ] {
            let command = format!("curl -o /dev/null {options} https://example.invalid");
            assert!(
                filesystem_effects_required_reason(&command).is_some(),
                "{command}"
            );
        }
    }

    #[test]
    fn curl_option_values_are_not_reinterpreted_as_options() {
        for options in [
            "-A '-oresult.txt'",
            "--user-agent '-oresult.txt'",
            "-sA '-oresult.txt'",
            "-H '--trace'",
            "--header '--trace'",
            "-d '--remote-name-all'",
            "--data-raw '--remote-name-all'",
            "-w '--output=result.txt'",
        ] {
            let command = format!("curl {options} -o /dev/null https://example.invalid");
            assert_eq!(
                filesystem_effects_required_reason(&command),
                None,
                "{command}"
            );
        }
    }

    #[test]
    fn shell_tokens_preserve_posix_double_quote_escapes() {
        assert_eq!(
            shell_tokens(r#"curl -o "/dev\/null""#),
            ["curl", "-o", r"/dev\/null"]
        );
        assert_eq!(
            shell_tokens(r#"printf '%s' "a\q\$b\`c\"d\\e""#),
            ["printf", "%s", "a\\q$b`c\"d\\e"]
        );
        assert_eq!(shell_tokens(r"printf '%s' 'a\b'"), ["printf", "%s", r"a\b"]);
        assert_eq!(shell_tokens("echo \\\n hi"), ["echo", "hi"]);
        assert_eq!(shell_tokens("echo hel\\\nlo"), ["echo", "hello"]);
        assert_eq!(shell_tokens("echo \"\\\n\" hi"), ["echo", "", "hi"]);
        assert_eq!(shell_tokens("echo '|'"), ["echo", "|"]);
    }
}
