//! Discovery and rendering for Agena's layered project instructions.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

const INSTRUCTION_FILE: &str = "AGENA.md";
const MAX_FILE_BYTES: usize = 50_000;

#[derive(Debug, Clone)]
struct ProjectInstructionLayer {
    path: PathBuf,
    content: String,
    truncated: bool,
}

pub(super) fn render_for_workspace(workspace_root: &Path) -> Option<String> {
    let mut layers = Vec::new();
    if let Some(global) = discover_global() {
        layers.push(global);
    }
    layers.extend(discover_workspace(workspace_root));
    render_section(&layers)
}

fn discover_global() -> Option<ProjectInstructionLayer> {
    home_dir().and_then(|home| read_instruction(home.join(".agena").as_path()))
}

fn discover_workspace(workspace_root: &Path) -> Vec<ProjectInstructionLayer> {
    let mut layers = Vec::new();
    let mut current = Some(workspace_root.to_path_buf());
    while let Some(directory) = current {
        if let Some(layer) = read_instruction(&directory) {
            layers.push(layer);
        }
        current = directory.parent().map(Path::to_path_buf);
    }
    layers.reverse();
    layers
}

fn read_instruction(directory: &Path) -> Option<ProjectInstructionLayer> {
    let path = directory.join(INSTRUCTION_FILE);
    if !path.is_file() {
        return None;
    }
    match read_truncated(&path) {
        Ok((content, truncated)) => Some(ProjectInstructionLayer {
            path,
            content,
            truncated,
        }),
        Err(error) => {
            tracing::warn!(
                target: "agena::project_instructions",
                "failed to read {}: {error}",
                path.display()
            );
            None
        }
    }
}

fn read_truncated(path: &Path) -> io::Result<(String, bool)> {
    let raw = fs::read_to_string(path)?;
    if raw.len() <= MAX_FILE_BYTES {
        return Ok((raw, false));
    }
    let mut cut_at = MAX_FILE_BYTES;
    while !raw.is_char_boundary(cut_at) {
        cut_at -= 1;
    }
    cut_at = raw[..cut_at].rfind('\n').unwrap_or(cut_at);
    Ok((raw[..cut_at].to_owned(), true))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn render_section(layers: &[ProjectInstructionLayer]) -> Option<String> {
    if layers.is_empty() {
        return None;
    }
    let mut output = String::from("# Project instructions\n\n");
    output.push_str(
        "These instruction files are part of Agena's core execution context. Later, more local files take precedence when instructions conflict.\n\n",
    );
    for layer in layers {
        output.push_str(&format!("## {}\n\n", layer.path.display()));
        output.push_str(layer.content.trim());
        if layer.truncated {
            output.push_str(&format!(
                "\n\n> This file exceeded {MAX_FILE_BYTES} bytes and was truncated."
            ));
        }
        output.push_str("\n\n");
    }
    Some(output.trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("agena-{label}-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn workspace_layers_render_outermost_to_innermost() {
        let root = temporary_root("project-instructions-order");
        let workspace = root.join("project").join("crate");
        fs::create_dir_all(&workspace).expect("create project instruction fixture");
        fs::write(root.join(INSTRUCTION_FILE), "outer rule").expect("write outer instructions");
        fs::write(workspace.join(INSTRUCTION_FILE), "inner rule")
            .expect("write inner instructions");

        let layers = discover_workspace(&workspace);
        let fixture_layers = layers
            .iter()
            .filter(|layer| layer.path.starts_with(&root))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(fixture_layers.len(), 2);
        assert_eq!(fixture_layers[0].path, root.join(INSTRUCTION_FILE));
        assert_eq!(fixture_layers[1].path, workspace.join(INSTRUCTION_FILE));

        let rendered = render_section(&fixture_layers).expect("render instruction layers");
        let outer = rendered.find("outer rule").expect("outer layer rendered");
        let inner = rendered.find("inner rule").expect("inner layer rendered");
        assert!(outer < inner);
        fs::remove_dir_all(root).expect("remove project instruction fixture");
    }

    #[test]
    fn truncation_never_splits_utf8() {
        let root = temporary_root("project-instructions-utf8");
        fs::create_dir_all(&root).expect("create project instruction fixture");
        let path = root.join(INSTRUCTION_FILE);
        let content = format!("{}界trailing", "a".repeat(MAX_FILE_BYTES - 1));
        fs::write(&path, content).expect("write oversized instructions");

        let (content, truncated) = read_truncated(&path).expect("read truncated instructions");
        assert!(truncated);
        assert_eq!(content.len(), MAX_FILE_BYTES - 1);
        assert!(content.is_char_boundary(content.len()));
        fs::remove_dir_all(root).expect("remove project instruction fixture");
    }
}
