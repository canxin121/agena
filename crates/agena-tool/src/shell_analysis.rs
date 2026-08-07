/// Parsed summary of a shell command used for policy and result handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandAnalysis {
    pub primary_command: Option<String>,
    pub subcommand: Option<String>,
    pub args: Vec<String>,
    pub classification: CommandClassification,
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

    match (
        analysis.primary_command.as_deref(),
        analysis.subcommand.as_deref(),
        exit_code,
    ) {
        (Some("grep" | "rg"), _, 1) | (Some("git"), Some("grep"), 1) => {
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
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut single_quote = false;
    let mut double_quote = false;
    let mut escape = false;

    for ch in command.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }

        match ch {
            '\\' if !single_quote => escape = true,
            '\'' if !double_quote => single_quote = !single_quote,
            '"' if !single_quote => double_quote = !double_quote,
            c if c.is_whitespace() && !single_quote && !double_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            ';' | '|' | '&' if !single_quote && !double_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(ch.to_string());
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
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
    let Some(segment) = command_segments(tokens).into_iter().next() else {
        return (None, None, Vec::new());
    };
    let mut index = 0;
    while index < segment.len() && is_assignment(segment[index].as_str()) {
        index += 1;
    }
    while index < segment.len() && is_command_wrapper(segment[index].as_str()) {
        index += 1;
    }
    let Some(primary) = segment.get(index).cloned() else {
        return (None, None, Vec::new());
    };
    let args = segment.iter().skip(index + 1).cloned().collect::<Vec<_>>();
    let subcommand = if primary == "git" {
        args.iter().find(|arg| !arg.starts_with('-')).cloned()
    } else {
        None
    };
    (Some(primary), subcommand, args)
}

fn is_separator(token: &str) -> bool {
    matches!(token, ";" | "|" | "&")
}

fn is_assignment(token: &str) -> bool {
    let Some((name, _value)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        && name
            .chars()
            .next()
            .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
}

fn is_command_wrapper(token: &str) -> bool {
    matches!(token, "env" | "command" | "builtin" | "nohup")
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
                if !target.is_empty() && target != "/dev/null" {
                    return Some(target);
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
    redirect_target(chars) == "/dev/null"
}

fn skip_redirect_whitespace(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
        chars.next();
    }
}

/// Collect the redirect destination after an already-consumed `>` (or `>>`).
fn redirect_target(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut target = String::new();
    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '<' | '>') {
            break;
        }
        target.push(ch);
        chars.next();
    }
    target
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
        let (_, _, args) = first_command(segment);
        if let Some(reason) = curl_filesystem_reason(args.as_slice()) {
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

fn curl_filesystem_reason(args: &[String]) -> Option<String> {
    if args.iter().any(|arg| arg.starts_with("file://")) {
        return Some("invokes curl with a local file URL".to_string());
    }
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "-o" | "--output" if args.get(index + 1).is_none_or(|value| value != "-") => {
                return Some(match args.get(index + 1) {
                    Some(target) if target != "-" => {
                        format!("invokes curl output option that writes a local file '{target}'")
                    }
                    _ => "invokes curl output option that writes a local file".to_string(),
                });
            }
            "-O" | "--remote-name" => {
                return Some(
                    "invokes curl remote-name output which writes to the current directory"
                        .to_string(),
                );
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
            "-c" | "--cookie-jar" => {
                return Some(match args.get(index + 1) {
                    Some(target) => format!(
                        "invokes curl cookie-jar option that writes a local file '{target}'"
                    ),
                    None => "invokes curl cookie-jar option that writes a local file".to_string(),
                });
            }
            "--cacert" | "--cert" | "--key" => {
                return Some(match args.get(index + 1) {
                    Some(target) => {
                        format!("invokes curl option '{arg}' that reads a local file '{target}'")
                    }
                    None => format!("invokes curl option '{arg}' that reads a local file"),
                });
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
                if (arg.starts_with("-o") && arg.len() > 2 && &arg[2..] != "-")
                    || (arg.starts_with("--output=") && arg != "--output=-")
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
                if (arg.starts_with("-c") && arg.len() > 2) || arg.starts_with("--cookie-jar=") {
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
                {
                    let target = arg.split_once('=').map(|(_, value)| value).unwrap_or("");
                    return Some(format!(
                        "invokes curl option '{arg}' that reads a local file '{target}'"
                    ));
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
        index += 1;
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
    let mut saw_unknown = false;
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
mod tests {
    use super::*;

    #[test]
    fn required_reason_only_for_provable_file_ops() {
        // Mutating commands still require declared effects.
        assert!(filesystem_effects_required_reason("rm -rf /tmp/x").is_some());
        assert!(filesystem_effects_required_reason("cat a.txt > /tmp/out").is_some());
        assert!(filesystem_effects_required_reason("cat < /etc/passwd").is_some());
        assert!(
            filesystem_effects_required_reason("curl -o /tmp/out https://example.com").is_some()
        );
        assert!(
            filesystem_effects_required_reason("curl --upload-file x https://example.com")
                .is_some()
        );

        // Interpreters and build tools may legitimately have no file effects
        // beyond the executables they invoke; an explicit empty list is fine.
        assert!(filesystem_effects_required_reason("node --version").is_none());
        assert!(filesystem_effects_required_reason("python3 --version").is_none());
        assert!(filesystem_effects_required_reason("uv --version").is_none());
        assert!(filesystem_effects_required_reason("cargo build").is_none());
        assert!(filesystem_effects_required_reason("ls -la").is_none());
        assert!(filesystem_effects_required_reason("git status").is_none());
    }

    #[test]
    fn dev_null_redirects_do_not_require_declared_effects() {
        assert!(contains_write_redirection("cat a.txt > /tmp/out"));
        assert!(!contains_write_redirection("cmd 2>/dev/null"));
        assert!(!contains_write_redirection("cmd > /dev/null"));
        assert!(!contains_write_redirection("cmd &>/dev/null"));
        assert!(!contains_write_redirection("cmd >>/dev/null 2>&1"));
        assert!(contains_input_redirection("cat < /etc/passwd"));
        assert!(!contains_input_redirection("cat < /dev/null"));

        assert!(
            filesystem_effects_required_reason(
                "which lldb && lldb --version 2>/dev/null | head -2"
            )
            .is_none()
        );
        assert!(
            filesystem_effects_required_reason(
                "pkill -f lldb 2>/dev/null; pkill -f debugserver 2>/dev/null; echo done"
            )
            .is_none()
        );
    }

    #[test]
    fn required_reason_names_the_detected_path() {
        let reason = filesystem_effects_required_reason("cat a.txt > /tmp/out")
            .expect("redirect to real file requires declaration");
        assert!(
            reason.contains("/tmp/out"),
            "reason should name the redirect target: {reason}"
        );

        let reason = filesystem_effects_required_reason("curl -o /tmp/out https://example.com")
            .expect("curl output requires declaration");
        assert!(
            reason.contains("/tmp/out"),
            "reason should name the curl output target: {reason}"
        );

        let reason = filesystem_effects_required_reason(
            "curl --cookie-jar /tmp/cookies.txt https://example.com",
        )
        .expect("curl cookie jar requires declaration");
        assert!(
            reason.contains("/tmp/cookies.txt"),
            "reason should name the cookie jar: {reason}"
        );
    }
}
