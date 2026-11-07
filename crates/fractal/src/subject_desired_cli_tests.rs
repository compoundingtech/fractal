use super::*;

fn parse(arguments: &[&str]) -> (bool, SubjectDesiredArgs) {
    let matches = command_graph().try_get_matches_from(arguments).unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    let Command::Subject { command: SubjectCommand::Desired(args) } = cli.command else {
        panic!("expected subject desired");
    };
    (cli.json, args)
}

#[test]
fn desired_accepts_exact_subjects_kind_and_global_json() {
    let (json, args) = parse(&[
        "fractal", "subject", "desired", "--json", "agent/example/worker",
        "mission/example", "--kind", "agent",
    ]);
    assert!(json);
    assert_eq!(args.subjects, ["agent/example/worker", "mission/example"]);
    assert_eq!(args.kind.as_deref(), Some("agent"));
    assert!(!args.all);
}

#[test]
fn desired_accepts_explicit_and_implicit_all() {
    for arguments in [
        vec!["fractal", "subject", "desired", "--all", "--kind", "mission"],
        vec!["fractal", "--json", "subject", "desired", "--kind", "mission"],
    ] {
        let (_, args) = parse(&arguments);
        assert!(args.subjects.is_empty());
        assert_eq!(args.kind.as_deref(), Some("mission"));
    }
    let (_, args) = parse(&["fractal", "subject", "desired"]);
    assert!(args.subjects.is_empty());
    assert!(!args.all);
    assert!(args.kind.is_none());
}

#[test]
fn desired_refuses_all_with_named_subjects() {
    for arguments in [
        vec!["fractal", "subject", "desired", "--all", "agent/example/worker"],
        vec!["fractal", "subject", "desired", "agent/example/worker", "--all"],
    ] {
        let error = command_graph().try_get_matches_from(arguments).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}

#[test]
fn desired_kind_requires_a_value() {
    assert!(command_graph().try_get_matches_from([
        "fractal", "subject", "desired", "--kind",
    ]).is_err());
}
