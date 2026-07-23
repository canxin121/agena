use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "init".to_owned(),
            description: "Initialise an AGENA.md describing the codebase".to_owned(),
            aliases: vec!["bootstrap".to_owned()],
            ..SkillFrontmatter::default()
        },
        r#"You are bootstrapping a project memory file (AGENA.md) for this repository.
Walk the top-level layout, read the package manifests
(Cargo.toml, package.json, pyproject.toml, etc.), and produce a concise
markdown document covering:

1. What the project is and the technology stack.
2. How to build, test, and run it.
3. Where the important code lives (one bullet per directory).
4. Any conventions (commit style, lint rules, formatting) that future
   contributors should follow.

Save the result to AGENA.md at the repo root."#,
    )
}
