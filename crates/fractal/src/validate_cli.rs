use super::*;
// The same print macro as main.rs: CLI output goes through Fractal's writer.
use fractal_daemon::cli_println as println;

#[derive(Args)]
pub(super) struct ValidateArgs {
    /// Input KDL files, each parsed independently.
    #[arg(required = true, num_args = 1..)]
    files: Vec<PathBuf>,
}

#[derive(serde::Serialize)]
struct ValidatedSubject {
    kind: String,
    id: String,
}

#[derive(serde::Serialize)]
struct ValidatedMission {
    id: String,
    revision: String,
}

pub(super) fn run_validate(args: &ValidateArgs, json_output: bool) -> Result<()> {
    let mut subjects = Vec::new();
    let mut missions = Vec::new();
    for path in &args.files {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))?;
        let intent = fractal_store::graph::parse_intent(&source, "local")
            .with_context(|| format!("validate {}", path.display()))?;
        for (id, subject) in intent.subjects {
            subjects.push(ValidatedSubject {
                kind: subject.kind,
                id,
            });
        }
        for mission in intent.missions.into_values() {
            subjects.push(ValidatedSubject {
                kind: "mission".into(),
                id: mission.subject,
            });
            missions.push(ValidatedMission {
                id: mission.id,
                revision: mission.revision,
            });
        }
    }
    if json_output {
        print_value(&json!({ "subjects": subjects, "missions": missions }), true)?;
    } else {
        for subject in subjects {
            println!("{} {}", subject.kind, subject.id);
        }
        for mission in missions {
            println!("{}@{}", mission.id, mission.revision);
        }
    }
    Ok(())
}
