//! The ledger against each thread's own CPU clock: spans crossing either mark count only their
//! part between the marks, spans outside count nothing, open and unreadable spans, restarts and
//! lost spans say so, and a charge never exceeds the CPU its threads spent between the marks.
use super::*;
use std::sync::mpsc;
use std::time::Duration;

/// Waits in these controls are bounded, so a failed assertion never leaves a worker or the test
/// blocked.
const WAIT: Duration = Duration::from_secs(10);

const KIND: &str = "task reconcile-pass";

/// The ledger is process-wide: one test at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn thread_ns() -> u64 {
    super::super::thread_cpu_ns()
}

/// Spend about `ms` of this thread's CPU.
fn burn(ms: u64) {
    let until = thread_ns() + ms * 1_000_000;
    let mut x = 0_u64;
    while thread_ns() < until {
        x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
    }
}

fn charged(work: impl FnOnce()) {
    super::super::with_cpu(Some(KIND), None, work);
}

#[test]
fn a_span_counts_only_its_cpu_between_the_marks() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    start(&[KIND]);
    // Crossing the first mark: only the part after it.
    let mut first = None;
    let mut after_first = 0;
    let mut closed = 0;
    charged(|| {
        burn(30);
        after_first = thread_ns();
        first = mark();
        burn(20);
        closed = thread_ns();
    });
    let first = first.unwrap();
    // Wholly between the marks.
    charged(|| burn(10));
    // Crossing the second mark: only the part before it.
    let mut second = None;
    let mut before_second = 0;
    charged(|| {
        burn(10);
        before_second = thread_ns();
        second = mark();
        burn(30);
    });
    let second = second.unwrap();
    // Wholly after the second mark: nothing.
    charged(|| burn(10));
    let charge = charge(KIND, first, second);
    assert_eq!(charge.status, Status::Complete);
    assert_eq!((charge.spans, charge.open_at_start, charge.open_at_end), (3, 1, 1));
    let ms = charge.cpu_ns as f64 / 1e6;
    // 20 + 10 + 10 ms, give or take the marks' own reads; never the 30 ms outside either.
    assert!((38.0..=48.0).contains(&ms), "{ms} ms");
    assert!(closed - after_first >= 20_000_000 && before_second > 0);
    stop();
}

#[test]
fn an_open_span_on_another_thread_counts_up_to_the_mark() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    start(&[KIND]);
    let (marked, wait_marked) = mpsc::channel::<()>();
    let (burned, wait_burned) = mpsc::channel::<()>();
    // Marked before the worker exists, so its span opens after this mark.
    let from = mark().unwrap();
    let worker = std::thread::spawn(move || {
        charged(|| {
            burn(20);
            let _ = burned.send(());
            // Released by the mark, or by `marked` dropping if the test fails first.
            let _ = wait_marked.recv_timeout(WAIT);
            burn(40);
        });
    });
    wait_burned.recv_timeout(WAIT).expect("the worker spent its first 20 ms");
    let to = mark().unwrap();
    drop(marked);
    worker.join().unwrap();
    let open = charge(KIND, from, to);
    assert_eq!(open.status, Status::Complete);
    assert_eq!((open.spans, open.open_at_start, open.open_at_end), (1, 0, 1));
    let ms = open.cpu_ns as f64 / 1e6;
    assert!((19.0..=30.0).contains(&ms), "only the 20 ms before the second mark, not {ms} ms");
    stop();
}

#[test]
fn charges_say_when_they_cannot_be_exact() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // An unreadable clock for a span open at a mark.
    start(&[KIND]);
    let from = mark().unwrap();
    let mut status = None;
    charged(|| {
        {
            let mut ledger = LEDGER.lock().unwrap();
            let state = ledger.as_mut().unwrap();
            let open = state.open[0];
            state.spans[open].clock = -1;
        }
        let to = mark().unwrap();
        status = Some(charge(KIND, from, to).status);
    });
    assert_eq!(status, Some(Status::Incomplete));
    // Its end is unreadable too: the span is lost when it closes.
    assert_eq!(LEDGER.lock().unwrap().as_ref().unwrap().lost, 1);
    // A restart between the marks.
    start(&[KIND]);
    let from = mark().unwrap();
    start(&[KIND]);
    let to = mark().unwrap();
    assert_eq!(charge(KIND, from, to).status, Status::Unknown);
    // A kind it does not watch, marks out of order, and no ledger at all.
    let later = mark().unwrap();
    assert_eq!(charge("task other", to, later).status, Status::Unknown);
    assert_eq!(charge(KIND, later, to).status, Status::Unknown);
    stop();
    assert!(mark().is_none());
    assert_eq!(charge(KIND, to, later).status, Status::Unknown);
    // Spans past the limit are lost, and the charge says so.
    start(&[KIND]);
    LEDGER.lock().unwrap().as_mut().unwrap().limit = 1;
    let from = mark().unwrap();
    charged(|| burn(1));
    charged(|| burn(1));
    let to = mark().unwrap();
    let lossy = charge(KIND, from, to);
    assert_eq!((lossy.status, lossy.spans), (Status::Lossy, 1));
    assert_eq!(LEDGER.lock().unwrap().as_ref().unwrap().lost, 1);
    // An unwatched kind never reaches the ledger's lock or its spans.
    super::super::with_cpu(Some("task other"), None, || burn(1));
    assert_eq!(LEDGER.lock().unwrap().as_ref().unwrap().spans.len(), 1);
    stop();
    assert!(WATCHED.load(Ordering::Acquire).is_null());
}

#[test]
fn a_charge_never_exceeds_its_threads_cpu_and_unwatched_kinds_stay_out() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    start(&[KIND]);
    let before = thread_ns();
    let from = mark().unwrap();
    charged(|| burn(15));
    super::super::with_cpu(Some("task other"), None, || burn(15));
    // Nested work is the outer span's: one span, no double count.
    charged(|| super::super::with_cpu(Some(KIND), None, || burn(15)));
    let to = mark().unwrap();
    let spent = thread_ns() - before;
    let charge = charge(KIND, from, to);
    assert_eq!(charge.status, Status::Complete);
    assert_eq!(charge.spans, 2);
    assert!(charge.cpu_ns <= spent, "{} > {spent}", charge.cpu_ns);
    let ms = charge.cpu_ns as f64 / 1e6;
    assert!((29.0..=36.0).contains(&ms), "{ms} ms");
    stop();
}

#[test]
fn handles_and_marks_from_an_earlier_ledger_change_nothing() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    start(&[KIND]);
    let old_span = open(KIND).unwrap();
    let old_mark = mark().unwrap();
    stop();
    // Closing into no ledger, then into a new empty one, is safe and does nothing.
    close(old_span);
    start(&[KIND]);
    close(old_span);
    assert!(LEDGER.lock().unwrap().as_ref().unwrap().spans.is_empty());
    // The new ledger's first span has the old handle's index; the old handle cannot close it.
    let new_span = open(KIND).unwrap();
    assert_eq!(new_span.1, old_span.1);
    assert_ne!(new_span.0, old_span.0, "a generation is never reused across stop and start");
    close(old_span);
    assert!(LEDGER.lock().unwrap().as_ref().unwrap().spans[0].closed_after.is_none());
    let new_mark = mark().unwrap();
    assert_eq!(charge(KIND, old_mark, new_mark).status, Status::Unknown);
    close(new_span);
    close(new_span);
    assert!(LEDGER.lock().unwrap().as_ref().unwrap().open.is_empty());
    stop();
}

#[test]
fn charges_between_consecutive_marks_add_up_under_concurrent_spans() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    start(&[KIND]);
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // However the test ends, the workers stop.
    struct Release(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Release {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let release = Release(done.clone());
    // Every worker is inside its loop before the first mark.
    let (ready, wait_ready) = mpsc::channel::<()>();
    let workers = (0..3)
        .map(|_| {
            let (done, ready) = (done.clone(), ready.clone());
            std::thread::spawn(move || {
                let _ = ready.send(());
                while !done.load(Ordering::Relaxed) {
                    charged(|| burn(1));
                }
            })
        })
        .collect::<Vec<_>>();
    for _ in 0..3 {
        wait_ready.recv_timeout(WAIT).expect("every worker started");
    }
    // Marks land at every point of the workers' 1 ms spans: between their opens, reads and
    // closes, and across them.
    let marks = (0..60)
        .map(|_| {
            std::thread::sleep(Duration::from_micros(300));
            mark().unwrap()
        })
        .collect::<Vec<_>>();
    drop(release);
    for worker in workers {
        worker.join().unwrap();
    }
    // Each span's reads are in the order the ledger recorded them: its start, then each mark's
    // sample, then its end. An open or close never straddles a mark.
    {
        let ledger = LEDGER.lock().unwrap();
        for span in &ledger.as_ref().unwrap().spans {
            let mut last = span.start_ns;
            for (&index, sample) in &span.samples {
                let sample = sample.expect("a live worker's clock reads");
                assert!(index >= span.opened_after && sample >= last);
                last = sample;
            }
            if let Some(closed) = span.closed_after {
                assert!(span.samples.keys().all(|&index| index < closed));
                assert!(span.end_ns >= last);
            }
        }
    }
    let whole = charge(KIND, marks[0], marks[marks.len() - 1]);
    assert_eq!(whole.status, Status::Complete);
    assert!(whole.spans > 0 && whole.cpu_ns > 0, "the workers' spans ran between the marks");
    let (mut sum, mut crossed) = (0, 0);
    for pair in marks.windows(2) {
        let part = charge(KIND, pair[0], pair[1]);
        assert_eq!(part.status, Status::Complete);
        sum += part.cpu_ns;
        crossed += part.open_at_start + part.open_at_end;
    }
    assert!(crossed > 0, "some span was open at a mark, so the split was exercised");
    assert_eq!(sum, whole.cpu_ns, "every span's CPU is counted once across the marks");
    stop();
}

/// This fills the process-wide list table for good. Every other test here starts with `[KIND]`,
/// which this test keeps first, so they still start under one process for all tests as well as
/// one process per test.
#[test]
fn watched_lists_are_kept_once_and_bounded() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for _ in 0..100 {
        start(&[KIND]);
    }
    let kept = || LISTS.lock().unwrap().len();
    let after_one = kept();
    start(&[KIND]);
    assert_eq!(kept(), after_one, "the same kinds reuse their list");
    // Fill the remaining room with distinct lists; one more leaves the ledger off.
    while kept() < LIST_LIMIT {
        let kind: &'static str = Box::leak(format!("task other-{}", kept()).into_boxed_str());
        start(Box::leak(vec![kind].into_boxed_slice()));
        assert!(mark().is_some());
    }
    start(Box::leak(vec!["task one-too-many"].into_boxed_slice()));
    assert_eq!(kept(), LIST_LIMIT);
    assert!(mark().is_none(), "no ledger started");
    assert!(WATCHED.load(Ordering::Acquire).is_null());
    // A kept list still starts.
    start(&[KIND]);
    assert!(mark().is_some());
    stop();
}

#[test]
fn generations_stop_at_exhaustion_and_never_wrap() {
    // The allocator `start` uses, on its own counter: the process-wide one is never reset.
    let counter = AtomicU64::new(u64::MAX - 1);
    assert_eq!(next_generation(&counter), Some(u64::MAX));
    assert_eq!(next_generation(&counter), None);
    assert_eq!(next_generation(&counter), None);
    assert_eq!(counter.load(Ordering::Relaxed), u64::MAX, "exhausted, not wrapped to 0");
    let fresh = AtomicU64::new(0);
    assert_eq!((next_generation(&fresh), next_generation(&fresh)), (Some(1), Some(2)));
}

#[test]
fn spans_wholly_between_outer_and_inner_marks_count_only_in_the_outer_bound() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    start(&[KIND]);
    // Outer and inner marks on each side, as the load takes them around its CPU reads, with a
    // pass that opens and closes wholly in each gap and one inside both.
    let outer_before = mark().unwrap();
    charged(|| burn(10));
    let inner_before = mark().unwrap();
    charged(|| burn(20));
    let inner_after = mark().unwrap();
    charged(|| burn(10));
    let outer_after = mark().unwrap();
    let outer = charge(KIND, outer_before, outer_after);
    let inner = charge(KIND, inner_before, inner_after);
    assert_eq!((outer.status, inner.status), (Status::Complete, Status::Complete));
    assert_eq!((outer.spans, inner.spans), (3, 1));
    assert_eq!((outer.open_at_start, outer.open_at_end), (0, 0), "open at neither outer mark");
    let (outer_ms, inner_ms) = (outer.cpu_ns as f64 / 1e6, inner.cpu_ns as f64 / 1e6);
    assert!((19.0..=24.0).contains(&inner_ms), "{inner_ms} ms");
    assert!((39.0..=46.0).contains(&outer_ms), "{outer_ms} ms");
    assert!(inner.cpu_ns <= outer.cpu_ns);
    stop();
}
