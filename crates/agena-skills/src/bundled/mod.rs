//! Built-in skills shipped as typed Rust capabilities.

mod init;
mod review;
mod security_review;

use crate::skill::Skill;

pub fn all() -> Vec<Skill> {
    vec![init::skill(), review::skill(), security_review::skill()]
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
            ["init", "review", "security_review"]
        );
        assert!(skills[0].matches("bootstrap"));
        assert!(skills[2].matches("security-review"));
    }
}
