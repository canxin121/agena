//! Shared wire contracts for the Skills library and installation workflow.

pub use agena_skills::source::{
    SkillCreateRequest, SkillDeleteResult, SkillEnabledRequest, SkillInstallRequest,
    SkillInstallSource, SkillResourceWriteRequest, SkillUpdateRequest, SkillUploadFile,
    SkillValidationRequest,
};
pub use agena_skills::{
    SkillCatalog, SkillDetail, SkillDiagnostic, SkillFile, SkillInstallCandidate,
    SkillInstallPreview, SkillInstallResult, SkillMetadata, SkillResource, SkillRoot, SkillScope,
    SkillSource, SkillSummary, SkillValidation,
};
