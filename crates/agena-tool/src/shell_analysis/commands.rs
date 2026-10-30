//! Shared command-shape rules. This is conservative static classification,
//! not a shell interpreter or a substitute for declared effects and sandboxing.

use super::CommandClassification;

pub(super) fn tokens(command: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut word = String::new();
    let (mut single, mut double, mut escaped, mut started) = (false, false, false, false);
    for ch in command.chars() {
        if escaped {
            escaped = false;
            if ch == '\n' {
                // A shell line continuation is not part of the argv word and
                // must not introduce an empty argument before whitespace.
                continue;
            }
            if double && !matches!(ch, '$' | '`' | '"' | '\\') {
                // POSIX double quotes preserve backslashes before other
                // characters. Dropping one can misidentify a real output
                // path such as "/dev\\/null" as the null device.
                word.push('\\');
            }
            word.push(ch);
            started = true;
            continue;
        }
        match ch {
            '\\' if !single => {
                escaped = true;
            }
            '\'' if !double => {
                single = !single;
                started = true;
            }
            '"' if !single => {
                double = !double;
                started = true;
            }
            ';' | '|' | '&' | '\n' if !single && !double => {
                if started {
                    result.push(std::mem::take(&mut word));
                    started = false;
                }
                // A quoted literal "|" must not become a pipeline separator.
                result.push(format!("\0{}", if ch == '\n' { ';' } else { ch }));
            }
            ch if ch.is_whitespace() && !single && !double => {
                if started {
                    result.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            _ => {
                word.push(ch);
                started = true;
            }
        }
    }
    if started {
        result.push(word);
    }
    result
}

/// Syntax whose effects cannot be derived from our literal argv parser.
pub(super) fn has_uninspected_syntax(command: &str) -> bool {
    let (mut single, mut double, mut escaped) = (false, false, false);
    for ch in command.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !single => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '$' | '`' if !single => return true,
            '(' | ')' | '{' | '}' | '#' if !single && !double => return true,
            '\0' => return true,
            _ => {}
        }
    }
    single || double || escaped
}

fn assignment(value: &str) -> bool {
    value.split_once('=').is_some_and(|(name, _)| {
        name.starts_with(|ch: char| ch == '_' || ch.is_ascii_alphabetic())
            && name
                .chars()
                .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    })
}

fn executable_name(value: &str) -> String {
    let name = value.rsplit(['/', '\\']).next().unwrap_or(value);
    name.strip_suffix(".exe").unwrap_or(name).to_owned()
}

pub(super) fn first_command(tokens: &[String]) -> (Option<String>, Option<String>, Vec<String>) {
    let Some(segment) = super::command_segments(tokens).into_iter().next() else {
        return (None, None, Vec::new());
    };
    let mut index = 0;
    while index < segment.len() {
        if assignment(&segment[index]) {
            index += 1;
            continue;
        }
        let name = executable_name(&segment[index]);
        match name.as_str() {
            "env" => {
                if index + 1 == segment.len() {
                    return (Some(name), None, Vec::new());
                }
                index += 1;
                while let Some(arg) = segment.get(index) {
                    match arg.as_str() {
                        "-i" | "--ignore-environment" | "--" => index += 1,
                        "-u" | "--unset" | "-C" | "--chdir" => index += 2,
                        _ if assignment(arg)
                            || arg.starts_with("--unset=")
                            || arg.starts_with("--chdir=") =>
                        {
                            index += 1
                        }
                        // env -S reparses another command language. Leave it unknown.
                        _ if arg.starts_with('-') => {
                            return (Some(name), None, segment[index..].to_vec());
                        }
                        _ => break,
                    }
                }
            }
            "command" | "builtin" | "exec" | "nohup" => {
                index += 1;
                if segment
                    .get(index)
                    .is_some_and(|arg| matches!(arg.as_str(), "-v" | "-V"))
                {
                    return (Some(name), None, segment[index..].to_vec());
                }
                if name == "exec" && segment.get(index).is_some_and(|arg| arg == "-a") {
                    index += 2;
                }
                if name == "command" && segment.get(index).is_some_and(|arg| arg == "-p") {
                    index += 1;
                }
                if segment.get(index).is_some_and(|arg| arg == "--") {
                    index += 1;
                }
                // nohup can create nohup.out even when the child only reads.
                if name == "nohup" {
                    return (Some(name), None, segment[index..].to_vec());
                }
            }
            _ => {
                let args = segment[index + 1..].to_vec();
                let subcommand = if name == "git" {
                    git_subcommand(&args)
                } else {
                    None
                };
                return (Some(name), subcommand, args);
            }
        }
    }
    (None, None, Vec::new())
}

fn git_subcommand(args: &[String]) -> Option<String> {
    git_subcommand_index(args).map(|index| args[index].clone())
}

fn git_subcommand_index(args: &[String]) -> Option<usize> {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--config-env" => {
                index += 2
            }
            _ if arg.starts_with('-') => index += 1,
            _ => return Some(index),
        }
    }
    None
}

fn option(args: &[String], long: &str, short: Option<char>) -> bool {
    args.iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| {
            arg == long
                || arg
                    .strip_prefix(long)
                    .is_some_and(|tail| tail.starts_with('='))
                || short.is_some_and(|short| {
                    arg.starts_with('-') && !arg.starts_with("--") && arg[1..].contains(short)
                })
        })
}

fn any_exact(args: &[String], names: &[&str]) -> bool {
    args.iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| names.contains(&arg.as_str()))
}

/// Ignore option values: a string such as `--arg name -example` is data,
/// not a cluster containing jq's `-e` flag. Unknown syntax stays an error.
pub(super) fn jq_exit_status(args: &[String]) -> bool {
    let mut index = 0;
    let mut enabled = false;
    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--" | "--args" | "--jsonargs" => break,
            "--exit-status" => enabled = true,
            "--arg" | "--argjson" | "--slurpfile" | "--rawfile" | "--argfile" => {
                index += 2;
            }
            "--from-file" | "--library-path" | "--indent" => index += 1,
            _ if arg.starts_with('-') && !arg.starts_with("--") => {
                let mut flags = arg[1..].chars().peekable();
                while let Some(flag) = flags.next() {
                    match flag {
                        'e' => enabled = true,
                        'f' | 'L' => {
                            if flags.peek().is_none() {
                                index += 1;
                            }
                            break;
                        }
                        'n' | 'R' | 's' | 'c' | 'r' | 'j' | 'a' | 'S' | 'C' | 'M' | 'b' => {}
                        _ => return false,
                    }
                }
            }
            _ => {}
        }
        index += 1;
    }
    enabled
}

fn mutation(primary: &str) -> Option<CommandClassification> {
    Some(CommandClassification::Mutating {
        reason: format!("invokes '{primary}' with a file-writing operation"),
    })
}

pub(super) fn classify(
    primary: &str,
    subcommand: Option<&str>,
    args: &[String],
) -> Option<CommandClassification> {
    use CommandClassification::{ReadOnly, Unknown};
    let readonly = Some(ReadOnly);
    let unknown = Some(Unknown);
    match primary {
        "rg" => {
            if option(args, "--pre", None) || option(args, "--hostname-bin", None) {
                unknown
            } else {
                readonly
            }
        }
        "fd" | "fdfind" => {
            if option(args, "--exec", Some('x')) || option(args, "--exec-batch", Some('X')) {
                unknown
            } else {
                readonly
            }
        }
        "find" => {
            // find's predicates remain active after a leading "--".
            if args.iter().any(|arg| {
                ["-delete", "-fprint", "-fprint0", "-fprintf", "-fls"].contains(&arg.as_str())
            }) {
                mutation(primary)
            } else if args
                .iter()
                .any(|arg| ["-exec", "-execdir", "-ok", "-okdir"].contains(&arg.as_str()))
            {
                unknown
            } else {
                readonly
            }
        }
        "env" => {
            if args.is_empty() {
                readonly
            } else {
                unknown
            }
        }
        "jq" => readonly,
        "yq" => {
            if option(args, "--inplace", Some('i')) || option(args, "--split-exp", Some('s')) {
                mutation(primary)
            } else if args
                .iter()
                .any(|arg| arg.contains("load(") || arg.contains("load_"))
            {
                unknown
            } else {
                readonly
            }
        }
        "ast-grep" => {
            if option(args, "--update-all", Some('U'))
                || option(args, "--interactive", Some('i'))
                || args.first().is_some_and(|arg| arg == "new")
            {
                mutation(primary)
            } else if args
                .first()
                .is_some_and(|arg| matches!(arg.as_str(), "lsp" | "completions"))
            {
                unknown
            } else if args.first().is_none_or(|arg| {
                arg.starts_with('-') || matches!(arg.as_str(), "run" | "scan" | "test")
            }) {
                readonly
            } else {
                unknown
            }
        }
        "sd" => {
            if option(args, "--preview", Some('p')) {
                readonly
            } else {
                mutation(primary)
            }
        }
        "bat" | "batcat" | "delta" => {
            if option(args, "--pager", None) {
                unknown
            } else {
                readonly
            }
        }
        "eza" | "dust" | "duf" | "procs" | "tokei" | "difft" => readonly,
        "scc" => {
            if option(args, "--output", Some('o')) {
                mutation(primary)
            } else {
                readonly
            }
        }
        "sort" => {
            if option(args, "--output", Some('o')) {
                mutation(primary)
            } else if option(args, "--compress-program", None) {
                unknown
            } else {
                readonly
            }
        }
        "sed" => {
            if option(args, "--in-place", Some('i')) {
                return mutation(primary);
            }
            // Support bounded line previews; arbitrary sed programs can execute
            // commands or write files even without -i (e/w and s///e/w).
            let scripts = args
                .iter()
                .filter(|arg| !arg.starts_with('-'))
                .collect::<Vec<_>>();
            if args.iter().filter(|arg| arg.starts_with('-')).all(|arg| {
                matches!(
                    arg.as_str(),
                    "-" | "--" | "-n" | "--quiet" | "--silent" | "-E" | "-r" | "--regexp-extended"
                )
            }) && scripts.first().is_some_and(|script| {
                script.ends_with('p')
                    && script[..script.len() - 1]
                        .chars()
                        .all(|ch| ch.is_ascii_digit() || matches!(ch, ',' | '$'))
            }) {
                readonly
            } else {
                unknown
            }
        }
        "awk" | "gawk" | "mawk" | "nawk" | "perl" | "fzf" | "rga" | "rtk" | "nohup" => unknown,
        "uniq" => {
            if args.iter().filter(|arg| !arg.starts_with('-')).count() > 1 {
                mutation(primary)
            } else {
                readonly
            }
        }
        "git" => {
            if option(args, "--output", None) {
                return mutation(primary);
            }
            if any_exact(
                args,
                &[
                    "-c",
                    "--config-env",
                    "--ext-diff",
                    "--textconv",
                    "--open-files-in-pager",
                ],
            ) || option(args, "--config-env", None)
                || args
                    .iter()
                    .any(|arg| arg.starts_with("-c") && arg.len() > 2)
                || (subcommand == Some("grep") && option(args, "--open-files-in-pager", Some('O')))
            {
                return unknown;
            }
            let operands = git_subcommand_index(args)
                .map(|index| &args[index + 1..])
                .unwrap_or_default();
            match subcommand {
                Some(
                    "status" | "diff" | "grep" | "show" | "log" | "rev-parse" | "ls-files"
                    | "ls-tree" | "cat-file" | "describe",
                ) => readonly,
                // branch/remote without explicit listing flags can mutate.
                Some("branch")
                    if any_exact(
                        operands,
                        &[
                            "-d",
                            "-D",
                            "--delete",
                            "-m",
                            "-M",
                            "--move",
                            "-c",
                            "-C",
                            "--copy",
                            "--edit-description",
                            "--set-upstream-to",
                            "--unset-upstream",
                        ],
                    ) =>
                {
                    mutation(primary)
                }
                Some("branch")
                    if any_exact(operands, &["--list", "--show-current"])
                        || operands.is_empty() =>
                {
                    readonly
                }
                Some("remote")
                    if operands
                        .iter()
                        .all(|arg| matches!(arg.as_str(), "-v" | "--verbose")) =>
                {
                    readonly
                }
                _ => None,
            }
        }
        "command"
            if args
                .first()
                .is_some_and(|arg| matches!(arg.as_str(), "-v" | "-V")) =>
        {
            readonly
        }
        "uv" | "duckdb" | "xan" | "qsv" | "hyperfine" | "watchexec" | "just" | "zoxide" | "gh"
        | "xh" | "http" | "https" | "agent-browser" | "playwright-cli" | "markitdown"
        | "docling" | "pdftotext" => {
            if args.len() == 1 && matches!(args[0].as_str(), "--version" | "--help" | "-h" | "-V") {
                readonly
            } else {
                unknown
            }
        }
        "tree" => {
            if option(args, "--outfile", Some('o')) {
                mutation(primary)
            } else {
                readonly
            }
        }
        "date" => {
            if args.iter().all(|arg| {
                arg.starts_with('+')
                    || matches!(
                        arg.as_str(),
                        "-u" | "--utc" | "--universal" | "-R" | "--rfc-email" | "-I" | "--iso-8601"
                    )
            }) {
                readonly
            } else {
                unknown
            }
        }
        "readlink" | "df" | "printenv" | "uname" | "true" | "false" => readonly,
        _ => None,
    }
}
