use fractal_mac_custody::{Invocation, Status, execute, rejected_invocation};
use std::io::Write;

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let receipt = match Invocation::parse(args.iter().cloned()) {
        Ok(invocation) => execute(&invocation),
        Err(reason) => rejected_invocation(&args, reason),
    };
    let failed = matches!(receipt.status, Status::Failed { .. } | Status::Unsupported { .. } | Status::PlanRejected { .. } | Status::RetirementUnknown { .. });
    let busy = matches!(receipt.status, Status::SlotBusy { .. });
    let mut stdout = std::io::stdout().lock();
    if serde_json::to_writer(&mut stdout, &receipt).is_err() || stdout.write_all(b"\n").is_err() || stdout.flush().is_err() {
        std::process::exit(1);
    }
    if busy { std::process::exit(75); }
    if failed { std::process::exit(1); }
}
