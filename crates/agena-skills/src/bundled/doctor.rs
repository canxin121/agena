use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "doctor".to_owned(),
            description:
                "Diagnose Agena runtime, provider, plugin, Skill, MCP and project tooling health"
                    .to_owned(),
            ..SkillFrontmatter::default()
        },
        r#"Run a bounded health audit and distinguish confirmed failures from optional or unconfigured features.

Check, in order: effective configuration and validation diagnostics; provider credentials without printing secrets; workspace and database paths; plugin enablement, trust and manifest diagnostics; Skill discovery diagnostics and dependency availability; MCP server status/auth/capabilities; LSP server availability; and project build/test command availability.

Do not mutate configuration automatically. Present a table with component, status (`healthy`, `degraded`, `unconfigured`, `failed`), evidence, impact, and the smallest remediation. Redact credentials and tokens. End with the exact checks run and any area that could not be verified."#,
    )
}
