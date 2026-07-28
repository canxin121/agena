use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "imagegen".to_owned(),
            description: "Generate or edit images with ordinary ChatGPT and Gemini execution tools"
                .to_owned(),
            aliases: vec!["image-generate".to_owned(), "image-edit".to_owned()],
            allowed_tools: vec![
                "agena.chatgpt.image_generation".to_owned(),
                "agena.chatgpt.image_edit".to_owned(),
                "agena.gemini.image_generation".to_owned(),
                "agena.gemini.image_edit".to_owned(),
                "agena.fs.read".to_owned(),
                "agena.fs.view_image".to_owned(),
                "agena.fs.stat".to_owned(),
            ],
            ..SkillFrontmatter::default()
        },
        r#"Create or edit the requested image through the ordinary `openai.image_generation` or `openai.image_edit` execution tools.

Treat these provider-backed tools exactly like every other Agena tool: discover or inspect them through the Tool API, obey the active agent/Skill allowlist and permission policy, and invoke them through `tools_call`. Their OpenAI transport is an implementation detail and is never exposed as a second provider-native tool system.

For edits, inspect supplied images first and preserve requested composition, transparency, dimensions, and identity constraints. Verify the resulting managed artifact with `fs.view_image`. Report the durable artifact path, format, dimensions when available, and the material prompt/edit decisions. Never claim an image was generated from a text-only response or return a transient URL as the only artifact."#,
    )
}
