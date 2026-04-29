//! Built-in skills compiled into the binary so a fresh install has
//! something usable out of the box.

const INIT_SKILL: &str = include_str!("init.md");
const REVIEW_SKILL: &str = include_str!("review.md");
const SECURITY_REVIEW_SKILL: &str = include_str!("security_review.md");

use crate::error::SkillResult;
use crate::skill::Skill;

pub fn all() -> SkillResult<Vec<Skill>> {
    let mut skills = Vec::new();
    for raw in [INIT_SKILL, REVIEW_SKILL, SECURITY_REVIEW_SKILL] {
        skills.push(Skill::from_raw(raw)?);
    }
    Ok(skills)
}
