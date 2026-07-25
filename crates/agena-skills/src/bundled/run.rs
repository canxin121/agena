use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "run".to_owned(),
            description: "Identify and start the current project with a reusable shell process"
                .to_owned(),
            aliases: vec!["start".to_owned()],
            allowed_tools: vec![
                "agena.fs.read".to_owned(),
                "agena.fs.glob".to_owned(),
                "agena.fs.grep".to_owned(),
                "agena.shell.run".to_owned(),
                "agena.shell.list".to_owned(),
                "agena.shell.logs".to_owned(),
                "agena.shell.stop".to_owned(),
                "agena.web.fetch".to_owned(),
            ],
            ..SkillFrontmatter::default()
        },
        r#"Determine the repository's supported development entry point from its documentation and manifests, then start it using `shell.run` in background/monitor mode when it is long-lived.

Avoid guessing commands when the project documents one. Reuse an already healthy matching process. Capture the process id, working directory, command, detected endpoint, readiness evidence, and relevant logs. If startup fails, diagnose the first actionable root cause without hiding the original output. Do not install dependencies or change configuration unless the user requested it."#,
    )
}
