//! Strict, line-oriented patch grammar. Semantic markers are never ignored.
use super::{Hunk, MAX_PATCH_FILE_BYTES, PatchOp, ToolError};

fn invalid(message: impl Into<String>) -> ToolError {
    ToolError::invalid_patch(message.into())
}
fn path(value: &str) -> Result<String, ToolError> {
    if value.is_empty() || value.chars().any(|c| matches!(c, '\0' | '\r' | '\n')) {
        return Err(invalid(
            "patch path must be nonempty and contain no control separators",
        ));
    }
    Ok(value.to_owned())
}

pub(super) fn parse_patch(text: &str) -> Result<Vec<PatchOp>, ToolError> {
    if text.len() as u64 > MAX_PATCH_FILE_BYTES {
        return Err(invalid("patch exceeds the 16 MiB byte limit"));
    }
    // Keep physical line endings for newly created file content. Structural
    // markers accept either LF or CRLF without normalizing payload bytes.
    let raw_lines: Vec<_> = text.split_terminator('\n').collect();
    let lines: Vec<_> = raw_lines
        .iter()
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    if lines.first().copied() != Some("*** Begin Patch") {
        return Err(invalid("patch must start with '*** Begin Patch'"));
    }
    if lines.last().copied() != Some("*** End Patch") {
        return Err(invalid("patch must end with '*** End Patch'"));
    }
    let mut operations = Vec::new();
    let mut index = 1;
    while index < lines.len() - 1 {
        let header = lines[index];
        index += 1;
        if header.is_empty() {
            continue;
        }
        if let Some(name) = header.strip_prefix("*** Add File: ") {
            let name = path(name)?;
            let mut content = String::new();
            let mut no_newline = false;
            while index < lines.len() - 1 && !lines[index].starts_with("*** ") {
                let line = lines[index];
                if line == "\\ No newline at end of file" && !content.is_empty() && !no_newline {
                    content.pop();
                    no_newline = true;
                } else if let Some(value) =
                    raw_lines[index].strip_prefix('+').filter(|_| !no_newline)
                {
                    content.push_str(value);
                    content.push('\n');
                } else {
                    return Err(invalid(
                        "add file expects '+' lines and an optional final no-newline marker",
                    ));
                }
                index += 1;
            }
            operations.push(PatchOp::Add {
                path: name,
                content,
            });
        } else if let Some(name) = header.strip_prefix("*** Delete File: ") {
            operations.push(PatchOp::Delete { path: path(name)? });
        } else if let Some(name) = header.strip_prefix("*** Update File: ") {
            let name = path(name)?;
            let mut move_to = None;
            let mut hunks = Vec::new();
            let mut hunk = Hunk::default();
            let mut last_kind = None;
            while index < lines.len() - 1 {
                let line = lines[index];
                if line == "*** End of File" {
                    if hunk.old.is_empty() && hunk.new.is_empty() {
                        return Err(invalid("EOF marker requires a nonempty hunk"));
                    }
                    if hunk.end_of_file {
                        return Err(invalid("duplicate EOF marker"));
                    }
                    hunk.end_of_file = true;
                    index += 1;
                    continue;
                }
                if let Some(target) = line.strip_prefix("*** Move to: ") {
                    if move_to.is_some()
                        || !hunks.is_empty()
                        || !hunk.old.is_empty()
                        || !hunk.new.is_empty()
                        || !hunk.contexts.is_empty()
                    {
                        return Err(invalid("move target must appear once, before update hunks"));
                    }
                    move_to = Some(path(target)?);
                    index += 1;
                    continue;
                }
                if line.starts_with("*** ") {
                    break;
                }
                if hunk.end_of_file {
                    return Err(invalid("EOF hunk must be the final hunk in its file"));
                }
                if line == "@@" || line.starts_with("@@ ") {
                    if !hunk.old.is_empty() || !hunk.new.is_empty() {
                        hunks.push(std::mem::take(&mut hunk));
                    }
                    if let Some(context) = line.strip_prefix("@@ ") {
                        if context.is_empty() {
                            return Err(invalid("empty context after '@@ '"));
                        }
                        hunk.contexts.push(context.to_owned());
                    }
                    last_kind = None;
                } else if line == "\\ No newline at end of file" {
                    match last_kind.take() {
                        Some('-') if !hunk.old_no_newline => {
                            hunk.old.pop();
                            hunk.old_no_newline = true;
                        }
                        Some('+') if !hunk.new_no_newline => {
                            hunk.new.pop();
                            hunk.new_no_newline = true;
                        }
                        Some(' ') if !hunk.old_no_newline && !hunk.new_no_newline => {
                            hunk.old.pop();
                            hunk.new.pop();
                            hunk.old_no_newline = true;
                            hunk.new_no_newline = true;
                        }
                        _ => {
                            return Err(invalid(
                                "no-newline marker must follow its final content line",
                            ));
                        }
                    }
                } else if let Some(value) = line.strip_prefix(' ') {
                    if hunk.old_no_newline || hunk.new_no_newline {
                        return Err(invalid("content after a no-newline marker"));
                    }
                    hunk.old.push_str(value);
                    hunk.old.push('\n');
                    hunk.new.push_str(value);
                    hunk.new.push('\n');
                    last_kind = Some(' ');
                } else if let Some(value) = line.strip_prefix('-') {
                    if hunk.old_no_newline {
                        return Err(invalid("removed content after a no-newline marker"));
                    }
                    hunk.old.push_str(value);
                    hunk.old.push('\n');
                    last_kind = Some('-');
                } else if let Some(value) = line.strip_prefix('+') {
                    if hunk.new_no_newline {
                        return Err(invalid("added content after a no-newline marker"));
                    }
                    hunk.new.push_str(value);
                    hunk.new.push('\n');
                    last_kind = Some('+');
                } else {
                    return Err(invalid(format!(
                        "invalid update hunk line in {name}: {line}"
                    )));
                }
                if hunks.len() > 4096 {
                    return Err(invalid("too many patch hunks"));
                }
                index += 1;
            }
            if !hunk.old.is_empty() || !hunk.new.is_empty() {
                hunks.push(hunk);
            } else if !hunk.contexts.is_empty() {
                return Err(invalid("context anchor has no update body"));
            }
            if hunks.is_empty() && move_to.is_none() {
                return Err(invalid(format!("update file has no hunks: {name}")));
            }
            operations.push(PatchOp::Update {
                path: name,
                move_to,
                hunks,
            });
        } else {
            return Err(invalid(format!("unknown patch section header: {header}")));
        }
    }
    Ok(operations)
}
