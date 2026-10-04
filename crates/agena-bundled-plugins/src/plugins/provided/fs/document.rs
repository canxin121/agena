use super::*;
use regex::{Regex, RegexBuilder};
use std::time::Duration;

const MAX_SOURCE: u64 = 32 * 1024 * 1024;
const MAX_CONVERTED: usize = 2 * 1024 * 1024;
const MAX_RECORDS: usize = 128 * 1024;
static CONVERTERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Backend {
    #[default]
    Auto,
    Pdftotext,
    Markitdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(non_empty_if_present("pattern"))]
pub(super) struct DocumentInput {
    #[arg(trim, non_empty)]
    path: String,
    #[serde(default)]
    backend: Backend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 4096)]
    pattern: Option<String>,
    #[serde(default = "default_true")]
    fixed_strings: bool,
    #[serde(default)]
    ignore_case: bool,
    #[serde(default = "one")]
    #[arg(minimum = 1)]
    start_line: usize,
    #[serde(default = "hundred")]
    #[arg(minimum = 1, maximum = 500)]
    max_lines: usize,
}
const fn one() -> usize {
    1
}
const fn hundred() -> usize {
    100
}

struct Source {
    _directory: tempfile::TempDir,
    path: PathBuf,
    extension: String,
    sha256: String,
}

fn prepare(workspace: &Path, path: &str) -> SdkResult<Source> {
    let path = resolve_path(&workspace.display().to_string(), path);
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "pdf" | "docx" | "pptx" | "xlsx") {
        return Err(PluginError::invalid_params(
            "document format must be pdf, docx, pptx or xlsx",
        ));
    }
    let bytes = read_file_bounded(&path, MAX_SOURCE, "fs.document")?;
    let directory = tempfile::tempdir().map_err(fs_error)?;
    let target = directory.path().join(format!("source.{extension}"));
    std::fs::write(&target, &bytes).map_err(fs_error)?;
    Ok(Source {
        _directory: directory,
        path: target,
        extension,
        sha256: sha256_bytes(&bytes),
    })
}

fn matcher(input: &DocumentInput) -> SdkResult<Option<Regex>> {
    if input.start_line == 0 || !(1..=500).contains(&input.max_lines) {
        return Err(PluginError::invalid_params(
            "start_line must be positive and max_lines must be 1–500",
        ));
    }
    input
        .pattern
        .as_ref()
        .map(|pattern| {
            if pattern.is_empty() || pattern.len() > 4096 {
                return Err(PluginError::invalid_params(
                    "pattern must contain 1–4096 bytes",
                ));
            }
            RegexBuilder::new(&if input.fixed_strings {
                regex::escape(pattern)
            } else {
                pattern.clone()
            })
            .case_insensitive(input.ignore_case)
            .size_limit(1024 * 1024)
            .build()
            .map_err(|error| PluginError::invalid_params_error(&error))
        })
        .transpose()
}

fn command(
    workspace: &Path,
    source: &Source,
    backend: Backend,
) -> SdkResult<(tokio::process::Command, &'static str)> {
    let path = std::env::var_os("PATH");
    let poppler = || which::which_in("pdftotext", path.as_ref(), workspace).ok();
    let pdftotext = match backend {
        Backend::Pdftotext if source.extension != "pdf" => {
            return Err(PluginError::invalid_params(
                "pdftotext only supports PDF files",
            ));
        }
        Backend::Pdftotext => Some(poppler().ok_or_else(|| {
            PluginError::invalid_params(
                "pdftotext is unavailable in the host PATH; install Poppler or choose MarkItDown",
            )
        })?),
        Backend::Auto if source.extension == "pdf" => poppler(),
        _ => None,
    };
    let (mut command, backend) = if let Some(executable) = pdftotext {
        let mut command = tokio::process::Command::new(executable);
        command
            .args(["-layout", "-enc", "UTF-8"])
            .arg(&source.path)
            .arg("-");
        (command, "pdftotext")
    } else {
        let python = std::env::var_os("AGENA_DOCUMENT_PYTHON").unwrap_or_else(|| {
            if cfg!(windows) {
                "python".into()
            } else {
                "python3".into()
            }
        });
        let executable = which::which_in(python, path.as_ref(), workspace).map_err(|_| PluginError::invalid_params("document Python interpreter not found; set AGENA_DOCUMENT_PYTHON to an environment containing MarkItDown"))?;
        let mut command = tokio::process::Command::new(executable);
        command
            .args([
                "-I",
                "-X",
                "utf8",
                "-c",
                include_str!("document/convert.py"),
            ])
            .arg(&source.path)
            .arg(&source.extension);
        (command, "markitdown")
    };
    command
        .current_dir(source._directory.path())
        .stdin(std::process::Stdio::null())
        .env("PYTHONIOENCODING", "utf-8")
        .env("NO_COLOR", "1");
    Ok((command, backend))
}

pub(super) async fn invoke(workspace: &Path, input: DocumentInput) -> SdkResult<ToolInvokeOutput> {
    let regex = matcher(&input)?;
    let _permit = CONVERTERS
        .acquire()
        .await
        .map_err(|error| PluginError::internal_error(&error))?;
    let root = workspace.to_owned();
    let path = input.path.clone();
    let selected = input.backend;
    let (source, command, backend) = run_fs_blocking(move || {
        let source = prepare(&root, &path)?;
        let (command, backend) = command(&root, &source, selected)?;
        Ok((source, command, backend))
    })
    .await?;
    let result = agena_process::output(command, Duration::from_secs(30), MAX_CONVERTED)
        .await
        .map_err(fs_error)?;
    let warnings = String::from_utf8_lossy(&result.stderr)
        .chars()
        .take(4096)
        .collect::<String>();
    if !result.status.success() {
        return Err(PluginError::invalid_params(format!(
            "{backend} conversion failed (exit {:?}): {warnings}",
            result.status.code()
        )));
    }
    let text = String::from_utf8(result.stdout)
        .map_err(|_| PluginError::invalid_params("document converter returned non-UTF-8 output"))?;
    let output = project(&text, regex.as_ref(), &input);
    let mut body = output
        .lines
        .iter()
        .map(|row| format!("{}: {}", row.line, row.text))
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        body = "No text was extracted. A scanned PDF may require OCR.".into();
    } else if output.lines.is_empty() {
        body = "No extracted lines matched in the requested range.".into();
    }
    if output.truncated {
        body.push_str("\n...document display truncated; narrow pattern or advance start_line.");
    }
    if !warnings.is_empty() {
        body.push_str("\nConverter warnings: ");
        body.push_str(&warnings);
    }
    Ok(ToolInvokeOutput::from_parts(
        format!("Read document · {}", input.path),
        format!("{} extracted lines · {backend}", output.total_lines),
        body,
        Some(
            serde_json::json!({"path":input.path,"backend":backend,"format":source.extension,"source_sha256":source.sha256,"total_lines":output.total_lines,"matching_lines":output.matching_lines,"lines":output.lines,"truncated":output.truncated,"conversion_complete":true,"warnings":warnings,"empty":text.trim().is_empty()}),
        ),
        Default::default(),
        Vec::new(),
    ))
}

#[derive(Serialize)]
struct Line {
    line: usize,
    text: String,
    text_truncated: bool,
}
struct Projection {
    lines: Vec<Line>,
    total_lines: usize,
    matching_lines: usize,
    truncated: bool,
}

fn project(text: &str, regex: Option<&Regex>, input: &DocumentInput) -> Projection {
    let mut out = Projection {
        lines: Vec::new(),
        total_lines: 0,
        matching_lines: 0,
        truncated: false,
    };
    let mut bytes = 0;
    let mut full = false;
    for (index, text) in text.lines().enumerate() {
        out.total_lines += 1;
        if index + 1 < input.start_line || regex.is_some_and(|regex| !regex.is_match(text)) {
            continue;
        }
        out.matching_lines += 1;
        if full || out.lines.len() >= input.max_lines {
            out.truncated = true;
            continue;
        }
        let clipped = text.len() > 4096;
        let text = text[..text.floor_char_boundary(4096.min(text.len()))].to_owned();
        let line = Line {
            line: index + 1,
            text,
            text_truncated: clipped,
        };
        let size = serde_json::to_vec(&line)
            .expect("document line serializes")
            .len()
            + 1;
        if bytes + size > MAX_RECORDS {
            out.truncated = true;
            full = true;
            continue;
        }
        out.truncated |= clipped;
        bytes += size;
        out.lines.push(line);
    }
    out
}

#[cfg(test)]
mod tests;
