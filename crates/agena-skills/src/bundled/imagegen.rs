use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "imagegen".to_owned(),
            description: "Generate or edit images through the active route's direct image API"
                .to_owned(),
            aliases: vec!["image-generate".to_owned(), "image-edit".to_owned()],
            allowed_tools: vec![
                "agena.image.generate".to_owned(),
                "agena.image.edit".to_owned(),
                "agena.fs.read".to_owned(),
                "agena.fs.view_image".to_owned(),
                "agena.fs.stat".to_owned(),
            ],
            ..SkillFrontmatter::default()
        },
        r#"Create or edit the requested image only when the active provider/model route exposes Agena's executable direct image capability.

Clarify the visual objective from the user's request without inventing brand assets or copyrighted references. For edits, inspect the supplied image first and preserve requested composition, transparency, dimensions, and identity constraints. Use `image.generate` or `image.edit`; each call must finish through the provider adapter and return a process-managed attachment in that same tool result. Then verify the resulting artifact with `fs.view_image`. Report the durable artifact path, format, dimensions when available, and the material prompt/edit decisions.

If the active provider has no executable image-generation tool, say so explicitly and suggest switching to a configured model route that supports it. Never claim an image was generated from a text-only response or return a transient URL as the only artifact."#,
    )
}
