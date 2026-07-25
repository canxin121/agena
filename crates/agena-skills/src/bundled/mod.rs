//! Built-in skills shipped as typed Rust capabilities.

mod batch;
mod debug;
mod doctor;
mod imagegen;
mod init;
mod plugin_creator;
mod review;
mod run;
mod run_skill_generator;
mod security_review;
mod simplify;
mod skill_creator;
mod skill_installer;
mod verify;

use crate::skill::Skill;

pub fn all() -> Vec<Skill> {
    vec![
        batch::skill(),
        debug::skill(),
        doctor::skill(),
        imagegen::skill(),
        init::skill(),
        plugin_creator::skill(),
        review::skill(),
        run::skill(),
        run_skill_generator::skill(),
        security_review::skill(),
        simplify::skill(),
        skill_creator::skill(),
        skill_installer::skill(),
        verify::skill(),
    ]
}

#[cfg(test)]
mod tests {
    use super::all;

    #[test]
    fn builtins_expose_stable_names_and_aliases() {
        let skills = all();
        assert_eq!(
            skills
                .iter()
                .map(|skill| skill.frontmatter.name.as_str())
                .collect::<Vec<_>>(),
            [
                "batch",
                "debug",
                "doctor",
                "imagegen",
                "init",
                "plugin_creator",
                "review",
                "run",
                "run_skill_generator",
                "security_review",
                "simplify",
                "skill_creator",
                "skill_installer",
                "verify",
            ]
        );
        assert!(skills.iter().any(|skill| skill.matches("bootstrap")));
        assert!(skills.iter().any(|skill| skill.matches("security-review")));
        assert!(skills.iter().any(|skill| skill.matches("create-skill")));
        assert!(skills.iter().any(|skill| skill.matches("check")));
    }
}
