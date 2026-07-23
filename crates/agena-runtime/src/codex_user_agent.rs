//! Stable Codex request identity used by Runtime-owned provider transports.

pub const RUNTIME_CODEX_ORIGINATOR: &str = "codex_cli_rs";

pub fn runtime_codex_user_agent(version: &str) -> String {
    let os_info = os_info::get();
    let terminal = runtime_terminal_user_agent();
    sanitize_runtime_http_user_agent(format!(
        "{RUNTIME_CODEX_ORIGINATOR}/{version} ({} {}; {}) {terminal}",
        os_info.os_type(),
        os_info.version(),
        os_info.architecture().unwrap_or("unknown"),
    ))
}

fn runtime_terminal_user_agent() -> String {
    let term_program = non_empty_env("TERM_PROGRAM");
    let term_program_version = non_empty_env("TERM_PROGRAM_VERSION");
    if let Some(program) = term_program {
        return term_program_version
            .map(|version| format!("{program}/{version}"))
            .unwrap_or(program);
    }
    non_empty_env("TERM").unwrap_or_else(|| "unknown".to_owned())
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

fn sanitize_runtime_http_user_agent(value: String) -> String {
    value
        .chars()
        .map(|character| {
            if matches!(character, ' '..='~') {
                character
            } else {
                '_'
            }
        })
        .collect()
}
