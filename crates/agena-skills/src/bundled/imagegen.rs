use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "imagegen".to_owned(),
            description: "Generate or edit images through an available provider-native image tool"
                .to_owned(),
            aliases: vec!["image-generate".to_owned(), "image-edit".to_owned()],
            allowed_tools: vec![
                "agena.fs.read".to_owned(),
                "agena.fs.view_image".to_owned(),
                "agena.fs.stat".to_owned(),
            ],
            ..SkillFrontmatter::default()
        },
        r#"Create or edit the requested image only when the active provider advertises an executable image-generation capability.

Clarify the visual objective from the user's request without inventing brand assets or copyrighted references. For edits, inspect the supplied image first and preserve requested composition, transparency, dimensions, and identity constraints. Use the provider-native image tool, then verify the resulting managed artifact with `fs.view_image`. Report the durable artifact path, format, dimensions when available, and the material prompt/edit decisions.

If the active provider has no executable image-generation tool, say so explicitly and suggest switching to a configured model route that supports it. Never claim an image was generated from a text-only response or return a transient URL as the only artifact."#,
    )
}
