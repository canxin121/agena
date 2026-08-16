//! Server-owned AWS profile presentation for provider choices.

pub(crate) fn list_aws_profile_names(application: &crate::TuiBackend) -> Vec<String> {
    application.aws_profiles()
}
