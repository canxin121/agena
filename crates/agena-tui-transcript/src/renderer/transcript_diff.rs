#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiffRowKind {
    Context,
    Added,
    Removed,
    Hunk,
    Note,
}

#[derive(Debug)]
pub(crate) struct DiffRow {
    pub kind: DiffRowKind,
    pub text: String,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
}

#[derive(Debug, Default)]
pub(crate) struct DiffFile {
    pub change: Option<char>,
    pub path: String,
    pub old_path: String,
    pub additions: usize,
    pub deletions: usize,
    pub rows: Vec<DiffRow>,
}

pub(crate) fn transcript_diff_files(diff: &str) -> Vec<DiffFile> {
    let mut files = Vec::new();
    let mut file = DiffFile::default();
    let (mut old_line, mut new_line) = (1, 1);
    let (mut old_left, mut new_left) = (0, 0);
    let mut legacy_hunk = false;
    let mut header_complete = false;
    let flush = |files: &mut Vec<DiffFile>, file: &mut DiffFile| {
        if file.change.is_none()
            && !file.old_path.is_empty()
            && !file.path.is_empty()
            && file.old_path != file.path
        {
            file.change = Some('R');
        }
        if !file.path.is_empty() || !file.old_path.is_empty() || !file.rows.is_empty() {
            files.push(std::mem::take(file));
        }
    };
    let path = |value: &str| {
        let value = value.split('\t').next().unwrap_or(value);
        value
            .strip_prefix("a/")
            .or_else(|| value.strip_prefix("b/"))
            .unwrap_or(value)
            .to_owned()
    };
    for raw in diff.lines() {
        if raw.starts_with("diff --git ") {
            flush(&mut files, &mut file);
            file.path = raw
                .rsplit_once(" b/")
                .map(|(_, value)| value.to_owned())
                .unwrap_or_default();
            old_left = 0;
            new_left = 0;
            legacy_hunk = false;
            header_complete = false;
            continue;
        }
        if raw.starts_with("@@") {
            let mut fields = raw.split_whitespace().skip(1);
            let range = |field: Option<&str>| -> Option<(usize, usize)> {
                let value = field?.get(1..)?;
                let (start, count) = value.split_once(',').unwrap_or((value, "1"));
                Some((start.parse().ok()?, count.parse().ok()?))
            };
            if let (Some(old), Some(new)) = (range(fields.next()), range(fields.next())) {
                (old_line, old_left) = old;
                (new_line, new_left) = new;
                legacy_hunk = false;
            } else {
                old_line = 1;
                new_line = 1;
                legacy_hunk = true;
            }
            file.rows.push(DiffRow {
                kind: DiffRowKind::Hunk,
                text: raw.into(),
                old_line: None,
                new_line: None,
            });
            continue;
        }
        let in_hunk = legacy_hunk || old_left > 0 || new_left > 0;
        if !in_hunk && let Some(value) = raw.strip_prefix("--- ") {
            if !file.rows.is_empty() || header_complete {
                flush(&mut files, &mut file);
            }
            header_complete = false;
            if value == "/dev/null" {
                file.change = Some('A');
            }
            file.old_path = if value == "/dev/null" {
                String::new()
            } else {
                path(value)
            };
            continue;
        }
        if !in_hunk && let Some(value) = raw.strip_prefix("+++ ") {
            header_complete = true;
            if value == "/dev/null" {
                file.change = Some('D');
            }
            file.path = if value == "/dev/null" {
                file.old_path.clone()
            } else {
                path(value)
            };
            continue;
        }
        if raw.starts_with("new file mode ") {
            file.change = Some('A');
            continue;
        }
        if raw.starts_with("deleted file mode ") {
            file.change = Some('D');
            continue;
        }
        if let Some(value) = raw.strip_prefix("rename from ") {
            file.old_path = value.into();
            continue;
        }
        if let Some(value) = raw.strip_prefix("rename to ") {
            file.path = value.into();
            continue;
        }
        if in_hunk && matches!(raw.as_bytes().first(), Some(b' ' | b'+' | b'-')) {
            let kind = match raw.as_bytes()[0] {
                b'+' => DiffRowKind::Added,
                b'-' => DiffRowKind::Removed,
                _ => DiffRowKind::Context,
            };
            file.rows.push(DiffRow {
                kind,
                text: raw[1..].into(),
                old_line: (kind != DiffRowKind::Added).then_some(old_line),
                new_line: (kind != DiffRowKind::Removed).then_some(new_line),
            });
            if kind != DiffRowKind::Added {
                old_line += 1;
                old_left = old_left.saturating_sub(1);
            }
            if kind != DiffRowKind::Removed {
                new_line += 1;
                new_left = new_left.saturating_sub(1);
            }
            if kind == DiffRowKind::Added {
                file.additions += 1;
            }
            if kind == DiffRowKind::Removed {
                file.deletions += 1;
            }
            continue;
        }
        if !raw.is_empty()
            && ![
                "index ",
                "new file mode ",
                "deleted file mode ",
                "similarity index ",
            ]
            .iter()
            .any(|prefix| raw.starts_with(prefix))
        {
            file.rows.push(DiffRow {
                kind: DiffRowKind::Note,
                text: raw.into(),
                old_line: None,
                new_line: None,
            });
        }
    }
    flush(&mut files, &mut file);
    files
}

pub(crate) fn file_change_display_path(change: &agena_domain::FileChangeRecord) -> String {
    if change.kind == agena_domain::FileChangeKind::Moved {
        change
            .from_path
            .as_ref()
            .map(|from_path| format!("{from_path} -> {}", change.path))
            .unwrap_or_else(|| change.path.clone())
    } else {
        change.path.clone()
    }
}

pub(crate) fn file_change_marker(kind: agena_domain::FileChangeKind) -> &'static str {
    match kind {
        agena_domain::FileChangeKind::Added => "A",
        agena_domain::FileChangeKind::Updated => "M",
        agena_domain::FileChangeKind::Deleted => "D",
        agena_domain::FileChangeKind::Moved => "R",
    }
}

pub(crate) fn file_change_list_item_text(
    change: &agena_domain::FileChangeRecord,
    i18n: &I18n,
) -> String {
    format!(
        "{} {} ({})",
        file_change_marker(change.kind),
        file_change_display_path(change),
        match change.kind {
            agena_domain::FileChangeKind::Added => ui_text::t(i18n, "file-change-added"),
            agena_domain::FileChangeKind::Updated => ui_text::t(i18n, "file-change-updated"),
            agena_domain::FileChangeKind::Deleted => ui_text::t(i18n, "file-change-deleted"),
            agena_domain::FileChangeKind::Moved => ui_text::t(i18n, "file-change-moved"),
        }
    )
}
use super::I18n;
use crate::ui_text;
