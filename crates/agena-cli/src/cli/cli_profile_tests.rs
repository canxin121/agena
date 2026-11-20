use super::*;
use clap::{CommandFactory, FromArgMatches};

#[test]
fn review_defaults_to_reviewer_without_changing_process_environment() {
    // Disable only this parser's environment input so the default remains
    // deterministic even when the test runner has AGENA_PROFILE configured.
    let command = AgenaCli::command().mut_subcommand("review", |command| {
        command.mut_arg("profile", |arg| arg.env(None::<&str>))
    });
    for (arguments, expected) in [
        (vec!["agena", "review"], "reviewer"),
        (vec!["agena", "review", "--profile", "main"], "main"),
        (vec!["agena", "review", "--profile="], ""),
    ] {
        let matches = command.clone().try_get_matches_from(arguments).unwrap();
        let cli = AgenaCli::from_arg_matches(&matches).unwrap();
        let Some(AgenaCommand::Review(args)) = cli.command else {
            panic!("expected review");
        };
        assert_eq!(args.profile.as_deref(), Some(expected));
    }
}

#[test]
fn one_shot_profiles_preserve_explicit_selection_and_clearing() {
    let cli =
        AgenaCli::try_parse_from(["agena", "exec", "--profile", "reviewer", "review"]).unwrap();
    let Some(AgenaCommand::Exec(args)) = cli.command else {
        panic!("expected exec");
    };
    assert_eq!(args.profile.as_deref(), Some("reviewer"));
    let cli = AgenaCli::try_parse_from(["agena", "review", "--profile="]).unwrap();
    let Some(AgenaCommand::Review(args)) = cli.command else {
        panic!("expected review");
    };
    assert_eq!(args.profile.as_deref(), Some(""));
}

#[test]
fn sessions_create_accepts_workspace_and_named_profile() {
    let cli = AgenaCli::try_parse_from([
        "agena",
        "sessions",
        "create",
        "--workspace",
        "/project",
        "--title",
        "Review",
        "--profile",
        "reviewer",
    ])
    .unwrap();
    let Some(AgenaCommand::Sessions(SessionsCommand {
        command: Some(SessionsSubcommand::Create(args)),
    })) = cli.command
    else {
        panic!("expected sessions create");
    };
    assert_eq!(args.workspace, Some(PathBuf::from("/project")));
    assert_eq!(args.title, "Review");
    assert_eq!(args.profile.as_deref(), Some("reviewer"));
}

#[test]
fn existing_session_profile_changes_require_an_explicit_option() {
    assert!(AgenaCli::try_parse_from(["agena", "sessions", "update", "42"]).is_err());
    let cli =
        AgenaCli::try_parse_from(["agena", "sessions", "update", "42", "--profile="]).unwrap();
    let Some(AgenaCommand::Sessions(SessionsCommand {
        command: Some(SessionsSubcommand::Update(args)),
    })) = cli.command
    else {
        panic!("expected sessions update");
    };
    assert_eq!(args.session_id, 42);
    assert_eq!(args.profile, "");
}
