use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "plugin_creator".to_owned(),
            description: "Scaffold and verify an Agena plugin using the repository SDK".to_owned(),
            aliases: vec!["create-plugin".to_owned()],
            ..SkillFrontmatter::default()
        },
        r#"Create or update an Agena plugin that follows the current SDK and repository conventions.

1. Inspect the plugin SDK, macro examples, built-in plugins, and the target workspace before designing the manifest.
2. Choose the smallest appropriate transport and capability set. Define typed inputs, output schema, tags, permission paths/network targets, concurrency, streaming, and UI display behavior explicitly.
3. Keep execution logic behind the shared Host APIs so permissions, cancellation, transcript output, and audit metadata remain consistent.
4. Add focused manifest, schema, permission, and invocation tests. Reject unknown fields and exercise failure paths.
5. Run formatting, tests, and clippy for the affected packages. Report the plugin key, tools, hooks, capabilities, configuration, and verification commands."#,
    )
}
