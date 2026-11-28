use agena_skills::{DeclaredSkill, SkillInvocation, SkillResourceBase};
use serde::Deserialize;

use super::{BTreeSet, Path, PathBuf, Url};

pub(super) const SERVICE: &str = include_str!("skills.js");
pub(super) const EXPORTS: &str = r#"
export const BUNDLED_SKILL_RANK = 600;
export const isSkillName = name => typeof name === 'string' && /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(name);
export const isModelInvocable = skill => skill.invocation.modelInvocable;
export const isUserInvocable = skill => skill.invocation.userInvocable;
"#;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    pub invocation: SkillInvocation,
    pub resource_base: Option<SkillResourceBase>,
    pub rank: f64,
}

#[derive(Deserialize)]
pub(in crate::agent_plugin) struct Snapshot {
    pub skills: Vec<Skill>,
    pub diagnostics: Vec<String>,
}

impl Skill {
    pub(super) fn validate(
        &mut self,
        root: &Path,
        watch: &mut BTreeSet<PathBuf>,
    ) -> Result<(), String> {
        agena_skills::validate_name(&self.name).map_err(|error| error.to_string())?;
        if self.content.len() > agena_skills::MAX_DOCUMENT_BYTES || self.description.len() > 4096 {
            return Err("Cordis skill exceeds its byte limit".into());
        }
        match self.resource_base.as_mut() {
            Some(SkillResourceBase::Directory { path }) => {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| "Cordis skill directory is outside its package")?;
                let directory = super::super::package_path(root, relative)?;
                if !directory.is_dir() {
                    return Err("Cordis skill resource base is not a directory".into());
                }
                watch.insert(directory.clone());
                *path = directory;
            }
            Some(SkillResourceBase::Url { url }) => {
                let target = Url::parse(url).map_err(|_| "invalid Cordis skill resource URL")?;
                if !matches!(target.scheme(), "http" | "https")
                    || target.host().is_none()
                    || !target.username().is_empty()
                    || target.password().is_some()
                    || url.len() > 2048
                {
                    return Err("invalid Cordis skill resource URL".into());
                }
                *url = target.to_string();
            }
            Some(SkillResourceBase::Opaque { description }) if description.len() > 2048 => {
                return Err("Cordis skill resource description is too long".into());
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn into_declared(self, plugin_id: &str, version: &str) -> DeclaredSkill {
        DeclaredSkill {
            plugin_id: plugin_id.to_owned(),
            version: version.to_owned(),
            name: self.name,
            description: self.description,
            instructions: self.content,
            aliases: Vec::new(),
            invocation: Some(self.invocation),
            resource_base: self.resource_base,
        }
    }
}
