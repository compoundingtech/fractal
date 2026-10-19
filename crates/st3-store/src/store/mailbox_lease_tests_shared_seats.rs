#![allow(unused_imports)]
use crate::store::*;
use crate::mailbox::{Authority, Fence};
use crate::model::{LaunchSpec, MemberKind, MemberSpec};
use crate::store::mailbox_lease::*;
use crate::store::mailbox_lease_shared_seats::*;

pub fn authority(sequence: u64) -> Authority {
        Authority {
            provider: "omp".into(),
            session: format!("provider-{sequence}"),
            sequence,
            pid: std::process::id(),
            process_token: st_runtime::process_start_token(std::process::id()).unwrap(),
        }
    }

pub fn request() -> Fence {
        Fence::new("agent/eval.worker", "current", "delivery")
    }

/// The result of a call that must finish while the test holds the writer. A timeout means it
    /// waited for the writer; a disconnected channel means the worker failed, and joining it
    /// reports why.
    pub fn finished_without_the_writer(
        case: &str,
        receiver: &std::sync::mpsc::Receiver<Result<(), String>>,
        worker: std::thread::JoinHandle<()>,
    ) -> Result<(), String> {
        let received = receiver.recv_timeout(std::time::Duration::from_secs(5));
        match received {
            Ok(result) => {
                worker.join().unwrap();
                result
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("{case}: waited for the writer")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                worker.join().expect("the worker thread failed");
                panic!("{case}: the worker ended without a result")
            }
        }
    }

/// Make the next promotion on this thread run `change` after its pre-check decided the call
    /// may promote, and report whether that point was reached.
    pub fn change_after_precheck(change: impl FnOnce() + 'static) -> Arc<std::sync::atomic::AtomicBool> {
        let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = Arc::clone(&reached);
        AFTER_PROMOTION_PRECHECK.with(|pause| {
            *pause.borrow_mut() = Some(Box::new(move || {
                observed.store(true, std::sync::atomic::Ordering::SeqCst);
                change();
            }));
        });
        reached
    }

pub fn precheck_was_reached(reached: &Arc<std::sync::atomic::AtomicBool>) -> bool {
        reached.load(std::sync::atomic::Ordering::SeqCst)
    }
