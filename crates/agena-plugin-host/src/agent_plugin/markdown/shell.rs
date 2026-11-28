use std::ops::Range;

pub(super) struct Injection<'a> {
    pub span: Range<usize>,
    pub command: &'a str,
}

struct Fence<'a> {
    marker: u8,
    width: usize,
    suffix: &'a str,
}

fn fence(line: &str) -> Option<Fence<'_>> {
    let line = line.trim_start_matches([' ', '\t']);
    let marker = *line.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let width = line.bytes().take_while(|byte| *byte == marker).count();
    (width >= 3).then(|| Fence {
        marker,
        width,
        suffix: line[width..].trim(),
    })
}

/// Find commands in the source document before substituting arguments or
/// inserting command output. Markdown examples in other fences stay literal.
pub(super) fn injections(source: &str) -> Vec<Injection<'_>> {
    let mut result = Vec::new();
    let mut open = None::<(Fence<'_>, usize, usize)>;
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let end = offset + line.len();
        let current_fence = fence(line);
        if let Some((opened, begin, body_begin)) = &open {
            if current_fence.is_some_and(|candidate| {
                candidate.marker == opened.marker
                    && candidate.width >= opened.width
                    && candidate.suffix.is_empty()
            }) {
                if opened.marker == b'`' && opened.suffix == "!" {
                    let command = source[*body_begin..offset].trim();
                    result.push(Injection {
                        span: *begin..offset + line.trim_end_matches(['\r', '\n']).len(),
                        command,
                    });
                }
                open = None;
            }
        } else if let Some(opened) = current_fence {
            open = Some((opened, offset, end));
        } else {
            inline(line, offset, &mut result);
        }
        offset = end;
    }
    result
}

fn inline<'a>(line: &'a str, offset: usize, result: &mut Vec<Injection<'a>>) {
    let mut ticks = line.match_indices('`').peekable();
    while let Some((start, _)) = ticks.next() {
        let mut width = 1;
        while ticks
            .next_if(|(index, _)| *index == start + width)
            .is_some()
        {
            width += 1;
        }
        let mut closing = ticks.clone();
        let mut end = None;
        while let Some((index, _)) = closing.next() {
            let mut count = 1;
            while closing
                .next_if(|(next, _)| *next == index + count)
                .is_some()
            {
                count += 1;
            }
            if count == width {
                end = Some(index);
                break;
            }
        }
        let Some(end) = end else {
            continue;
        };
        ticks = closing;
        if let Some(bang) = start.checked_sub(1)
            && line.as_bytes()[bang] == b'!'
            && line[..bang]
                .chars()
                .next_back()
                .is_none_or(char::is_whitespace)
        {
            result.push(Injection {
                span: offset + bang..offset + end + width,
                command: line[start + width..end].trim(),
            });
        }
    }
}

#[cfg(test)]
#[test]
fn finds_inline_and_block_commands_without_expanding_markdown_examples() {
    let source = "value !`printf value` and KEY=!`printf literal`\n\n```md\n!`printf example`\n```\n```!\nprintf one\nprintf two\n```\n!`printf last`\n";
    let found = injections(source);
    assert_eq!(
        found.iter().map(|entry| entry.command).collect::<Vec<_>>(),
        ["printf value", "printf one\nprintf two", "printf last"],
    );
    assert_eq!(
        &source[found[1].span.clone()],
        "```!\nprintf one\nprintf two\n```"
    );
}
