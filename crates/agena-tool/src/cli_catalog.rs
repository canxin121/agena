//! Curated task-oriented CLI guidance, shared by capability discovery and docs.
//! Presence never authorizes execution or overrides a project's toolchain.

#[derive(Debug, Clone, Copy)]
pub struct CliToolSpec {
    pub name: &'static str,
    pub executables: &'static [&'static str],
    pub purpose: &'static str,
    pub guidance: &'static str,
    pub tier: &'static str,
    pub version_args: &'static [&'static str],
}

macro_rules! cli {
    ($name:literal, $aliases:expr, $purpose:literal, $tier:literal, $guidance:literal) => {
        CliToolSpec {
            name: $name,
            executables: $aliases,
            purpose: $purpose,
            tier: $tier,
            guidance: $guidance,
            version_args: &["--version"],
        }
    };
}

/// Preferred means a useful default for this task, not a global command alias.
pub const CLI_CATALOG: &[CliToolSpec] = &[
    cli!(
        "rg",
        &["rg"],
        "text_search",
        "preferred",
        "Prefer rg for shell text search; -F for literals, -n --color=never for text, --json for parsing; exit 1 means no matches. Use --no-config for reproducible behavior."
    ),
    cli!(
        "fd",
        &["fd", "fdfind"],
        "path_search",
        "preferred",
        "Find paths with fd --type f, --extension or --glob; use rg --files if unavailable. Hidden/ignored files require explicit options. -x/-X execute commands."
    ),
    cli!(
        "jq",
        &["jq"],
        "json",
        "preferred",
        "Parse JSON structurally; -c for compact JSON, -r for strings. Select required fields. With -e, false/null and empty results have nonzero exit status."
    ),
    cli!(
        "ast-grep",
        &["ast-grep"],
        "structural_code",
        "preferred",
        "Use code.search_ast or ast-grep run for AST patterns; use LSP for symbol relationships. --json is machine-readable. Review rewrites before updating; do not use ambiguous sg aliases."
    ),
    cli!(
        "git",
        &["git"],
        "version_control",
        "preferred",
        "Use git ls-files/git grep for tracked-file scope; git --no-pager diff --no-ext-diff --color=never for machine-readable patches."
    ),
    cli!(
        "yq",
        &["yq"],
        "yaml",
        "optional",
        "Use Mike Farah yq v4 for structural YAML edits; verify --version because another yq has incompatible syntax. -i writes, and comments/whitespace may change."
    ),
    cli!(
        "uv",
        &["uv"],
        "python_environment",
        "optional",
        "Prefer uv for new Python environments/tools where appropriate; preserve existing project lockfiles and toolchains. run/tool commands can install dependencies and contact the network."
    ),
    cli!(
        "xan",
        &["xan"],
        "csv",
        "optional",
        "Parse quoted CSV with xan select/filter/stats instead of splitting lines on commas. Select the correct delimiter and output only needed columns."
    ),
    cli!(
        "qsv",
        &["qsv"],
        "csv",
        "optional",
        "Alternative CSV processing suite; features depend on the build. Use one CSV processor rather than duplicating workflows; some commands write files or access networks."
    ),
    cli!(
        "duckdb",
        &["duckdb"],
        "tabular_sql",
        "optional",
        "Query CSV/Parquet and joins with SQL. Set explicit types where needed; database files/extensions can cause writes or network access."
    ),
    cli!(
        "sd",
        &["sd"],
        "text_replacement",
        "optional",
        "Use --preview before replacing files and -F for literal strings. File arguments normally cause in-place edits; preserve Agena revision checks."
    ),
    cli!(
        "gh",
        &["gh"],
        "github",
        "optional",
        "Use GitHub-specific commands and --json field selection instead of scraping pages; credentials and mutation authorization still apply."
    ),
    cli!(
        "watchexec",
        &["watchexec"],
        "file_watch",
        "optional",
        "Use file events for rebuild/restart tasks instead of polling; launch through managed background execution and keep its handle."
    ),
    cli!(
        "hyperfine",
        &["hyperfine"],
        "benchmark",
        "optional",
        "Compare equivalent workloads with warmups and repeated runs, export JSON, and report cache/startup assumptions; it executes the supplied commands."
    ),
    cli!(
        "just",
        &["just"],
        "project_recipes",
        "optional",
        "Use an existing justfile's named recipes. Follow existing Make/Cargo/Bun workflows; just is a command runner, not an incremental build system."
    ),
    cli!(
        "tokei",
        &["tokei"],
        "code_statistics",
        "optional",
        "Count language/code/comment/blank lines with machine-readable output; specify whether dependencies and generated files belong in scope."
    ),
    cli!(
        "scc",
        &["scc"],
        "code_statistics",
        "optional",
        "Alternative code statistics with JSON output. Complexity counts do not measure code quality; --output writes a file."
    ),
    cli!(
        "dust",
        &["dust"],
        "directory_sizes",
        "optional",
        "Find large directories/files; -j returns JSON. Disk traversal still costs I/O."
    ),
    cli!(
        "duf",
        &["duf"],
        "disk_capacity",
        "optional",
        "Use --json and filters for filesystem capacity; system df remains suitable for simple checks."
    ),
    cli!(
        "procs",
        &["procs"],
        "process_inspection",
        "optional",
        "Process search and tree views; platform coverage varies. Keep lifecycle ownership in Agena; use explicit ps columns when machine parsing is needed."
    ),
    cli!(
        "xh",
        &["xh"],
        "http_debugging",
        "optional",
        "Convenient HTTP request syntax; retain curl for specialized transfers. Disable formatting for machine output and declare network/file effects."
    ),
    cli!(
        "httpie",
        &["http"],
        "http_debugging",
        "optional",
        "Human-friendly HTTP requests, not necessarily lower network latency. Disable styling and paging for model output."
    ),
    cli!(
        "rga",
        &["rga"],
        "document_search",
        "optional",
        "Search extracted PDF/Office/archive text. Requires format adapters and may populate caches; ordinary source code search should use rg."
    ),
    cli!(
        "markitdown",
        &["markitdown"],
        "document_conversion",
        "optional",
        "Convert supported local documents to Markdown; inspect extraction quality and optional plugin/model dependencies."
    ),
    cli!(
        "docling",
        &["docling"],
        "document_layout_ocr",
        "optional",
        "Use for document layout, tables and OCR when simple text extraction is insufficient; models may need downloading and output files are created."
    ),
    CliToolSpec {
        name: "pdftotext",
        executables: &["pdftotext"],
        purpose: "pdf_text",
        tier: "optional",
        guidance: "Extract text PDFs locally; use stdout output (-) to avoid an implicit text file. Scanned PDFs require OCR.",
        version_args: &["-v"],
    },
    cli!(
        "agent-browser",
        &["agent-browser"],
        "browser_automation",
        "optional",
        "Native browser CLI; use explicit isolated sessions, fresh snapshot refs and JSON. Browser actions have network and state effects."
    ),
    cli!(
        "playwright-cli",
        &["playwright-cli"],
        "browser_automation",
        "optional",
        "Use explicit browser sessions and bounded snapshots; keep browser ownership, downloads, and automation effects within the runtime."
    ),
    cli!(
        "semgrep",
        &["semgrep"],
        "code_rules",
        "optional",
        "Use for dedicated code/security rules, not simple string searches. Select local rules and verify telemetry/network behavior and licensed features."
    ),
    cli!(
        "rtk",
        &["rtk"],
        "output_compression",
        "experimental",
        "Optional output filtering; retain raw logs, exit status and failure details. Never transparently rewrite commands or equate byte reduction with bill reduction."
    ),
    cli!(
        "bat",
        &["bat", "batcat"],
        "code_preview",
        "display",
        "Use --paging=never --color=never and a line range. Builtin fs.read/read_many or cat remain suitable for plain text; highlighting is for human display."
    ),
    cli!(
        "eza",
        &["eza"],
        "directory_display",
        "display",
        "Tree/metadata views for humans; prefer fd/rg for path discovery and avoid icons/color in model output."
    ),
    cli!(
        "delta",
        &["delta"],
        "diff_display",
        "display",
        "Human-oriented diff display. Use plain git diff for model input and patch application; disable the pager."
    ),
    cli!(
        "difft",
        &["difft"],
        "syntax_diff",
        "display",
        "Syntax-aware comparison for reviews; output is not an applicable patch and parsing has a cost."
    ),
    cli!(
        "fzf",
        &["fzf"],
        "interactive_selection",
        "interactive",
        "Use only for deliberate interactive selection or explicit non-interactive --filter; bindings can execute commands."
    ),
    cli!(
        "zoxide",
        &["zoxide"],
        "directory_history",
        "interactive",
        "Useful for human directory history; autonomous tasks should use explicit cwd rather than depending on personal navigation history."
    ),
];

pub fn find(name: &str) -> Option<&'static CliToolSpec> {
    CLI_CATALOG
        .iter()
        .find(|tool| tool.name == name || tool.executables.contains(&name))
}

#[cfg(test)]
mod tests {
    #[test]
    fn aliases_do_not_hide_tool_identity() {
        assert_eq!(super::find("fdfind").unwrap().name, "fd");
        assert_eq!(super::find("batcat").unwrap().name, "bat");
        assert!(super::find("sg").is_none());
        let mut names = std::collections::HashSet::new();
        for tool in super::CLI_CATALOG {
            assert!(names.insert(tool.name), "duplicate {}", tool.name);
        }
    }
}
