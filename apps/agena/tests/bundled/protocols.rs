//! External protocol peers are local fixtures; Agena still uses its ordinary
//! bundled plugins, permission checks, process ownership, and HTTP transports.

use agena_testkit::TestResult;

mod lsp;
mod mcp;
mod providers;
mod web;

pub(super) const CASES: &[(&str, fn() -> TestResult)] = &[
    (
        "agena.chatgpt",
        providers::chatgpt_uses_the_session_provider,
    ),
    ("agena.claude", providers::claude_uses_the_session_provider),
    ("agena.gemini", providers::gemini_uses_the_session_provider),
    ("agena.web", web::configured_search_uses_real_http),
    (
        "agena.lsp",
        lsp::language_server_navigation_and_document_versions,
    ),
    ("agena.mcp", mcp::http_peer_handshake_discovery_and_calls),
];
