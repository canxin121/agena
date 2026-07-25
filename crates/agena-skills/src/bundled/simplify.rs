use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "simplify".to_owned(),
            description: "Reduce unnecessary complexity without changing behavior".to_owned(),
            allowed_tools: vec![
                "agena.fs.read".to_owned(),
                "agena.fs.glob".to_owned(),
                "agena.fs.grep".to_owned(),
                "agena.fs.apply_patch".to_owned(),
                "agena.code.search_ast".to_owned(),
                "agena.lsp.references".to_owned(),
                "agena.lsp.diagnostics".to_owned(),
                "agena.shell.run".to_owned(),
            ],
            ..SkillFrontmatter::default()
        },
        r#"Simplify the selected change or subsystem while preserving its observable behavior and public contracts.

Look for duplicated logic, unnecessary indirection, redundant compatibility layers, over-generalized abstractions, dead branches, and avoidable state. Confirm callers and tests before removing anything. Prefer a smaller coherent design over mechanical shortening, and keep changes inside the requested scope. After editing, run focused behavioral tests plus formatting and lint/type checks. Summarize what became simpler and the evidence that behavior stayed intact."#,
    )
}
