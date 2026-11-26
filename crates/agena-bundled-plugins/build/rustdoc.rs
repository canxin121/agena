//! Build Rustdoc modules from the anchored sections in the bundled reference.

use std::{collections::BTreeSet, fmt::Write as _, fs, path::Path};

pub(super) fn write_pages(manifest: &Path, out: &Path) {
    let reference = manifest.join("generated/tools-reference.md");
    println!("cargo:rerun-if-changed={}", reference.display());
    println!("cargo:rerun-if-changed=build/rustdoc.rs");
    let markdown = fs::read_to_string(reference).expect("read bundled tools reference");
    let (index, plugins) = sections(&markdown, "plugin");
    let index = link_pages(index, &plugins, "plugin");
    let mut source = format!("#[doc = {index:?}]\npub mod tools_docs {{\n");

    for plugin in plugins {
        let (overview, tools) = sections(plugin.body, "tool");
        let overview = link_pages(overview, &tools, "tool");
        writeln!(
            source,
            "#[doc = {overview:?}]\n#[doc(alias = {:?})]\npub mod {} {{",
            plugin.id, plugin.module
        )
        .unwrap();
        for tool in tools {
            let short = tool.id.strip_prefix("agena.").unwrap_or(tool.id);
            writeln!(
                source,
                "#[doc = {:?}]\n#[doc(alias = {:?})]",
                tool.body.trim(),
                tool.id
            )
            .unwrap();
            if short != tool.id {
                writeln!(source, "#[doc(alias = {short:?})]").unwrap();
            }
            writeln!(source, "pub mod {} {{}}", tool.module).unwrap();
        }
        source.push_str("}\n");
    }
    source.push_str("}\n");
    fs::write(out.join("tools_docs.rs"), source).expect("write bundled Rustdoc pages");
}

struct Section<'a> {
    id: &'a str,
    module: String,
    body: &'a str,
}

/// The renderer emits each anchor on its own line, outside of prose and code
/// blocks. Both the Markdown links and the Rustdoc split use those same IDs.
fn sections<'a>(text: &'a str, kind: &str) -> (&'a str, Vec<Section<'a>>) {
    let prefix = format!("<a id=\"{kind}-");
    let mut offset = 0;
    let mut boundaries = Vec::new();
    for line in text.split_inclusive('\n') {
        if let Some(id) = line
            .trim_end_matches(['\r', '\n'])
            .strip_prefix(&prefix)
            .and_then(|anchor| anchor.strip_suffix("\"></a>"))
        {
            boundaries.push((id, offset, offset + line.len()));
        }
        offset += line.len();
    }

    let intro_end = boundaries
        .first()
        .map_or(text.len(), |&(_, start, _)| start);
    let mut modules = BTreeSet::new();
    let mut sections = Vec::with_capacity(boundaries.len());
    for (index, &(id, _, start)) in boundaries.iter().enumerate() {
        let end = boundaries
            .get(index + 1)
            .map_or(text.len(), |&(_, end, _)| end);
        let short = if kind == "tool" {
            id.splitn(3, '.').nth(2).unwrap_or(id)
        } else {
            id
        };
        let slug: String = short
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
            .collect();
        let name = format!("{kind}_{slug}");
        let module = if modules.insert(name.clone()) {
            name
        } else {
            let mut encoded = String::from("id_");
            for byte in id.bytes() {
                write!(encoded, "{byte:02x}").unwrap();
            }
            encoded
        };
        sections.push(Section {
            id,
            module,
            body: &text[start..end],
        });
    }
    (&text[..intro_end], sections)
}

fn link_pages(intro: &str, sections: &[Section<'_>], kind: &str) -> String {
    let mut text = intro.trim().to_owned();
    for section in sections {
        text = text.replace(
            &format!("](#{kind}-{})", section.id),
            &format!("]({}/index.html)", section.module),
        );
    }
    text
}
