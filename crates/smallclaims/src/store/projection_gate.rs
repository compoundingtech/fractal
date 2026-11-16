//! FIFO ownership of projection passes, independent of the SQLite writer loan.
//!
//! Tickets are assigned under a short mutex, not by condvar wake order. A pass keeps
//! its ticket while lending the writer back; dropping its guard admits the oldest
//! remaining waiter. Deadline cancellation never lets a newer live ticket jump ahead.

use std::collections::BTreeSet;
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::Instant;

/// Serializes projection passes in ticket order, without holding a mutex during work.
#[derive(Default)]
pub struct ProjectionGate {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    next: u64,
    serving: u64,
    canceled: BTreeSet<u64>,
}

impl State {
    fn advance(&mut self) {
        self.serving = self.serving.wrapping_add(1);
        while self.canceled.remove(&self.serving) {
            self.serving = self.serving.wrapping_add(1);
        }
    }

    fn cancel(&mut self, ticket: u64) {
        if ticket == self.serving {
            self.advance();
        } else {
            self.canceled.insert(ticket);
        }
    }
}

impl ProjectionGate {
    /// Wait for every older non-canceled projection pass to finish.
    pub fn lock(&self) -> ProjectionGuard<'_> {
        self.acquire(None).expect("a wait without a deadline cannot expire")
    }

    /// Acquire before `deadline`, or remove this ticket and return `None`.
    ///
    /// The deadline includes queueing time; an expired deadline cannot acquire even
    /// an idle gate.
    pub fn lock_until(&self, deadline: Instant) -> Option<ProjectionGuard<'_>> {
        self.acquire(Some(deadline))
    }

    fn acquire(&self, deadline: Option<Instant>) -> Option<ProjectionGuard<'_>> {
        // No projection work executes under this bookkeeping mutex. An owner can
        // unwind after durable turns without invalidating the ticket queue.
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let ticket = state.next;
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return None;
        }
        state.next = state.next.wrapping_add(1);
        // Let tests observe enrollment without sleeps or scheduler assumptions.
        #[cfg(test)]
        self.changed.notify_all();
        loop {
            let remaining = deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
            if remaining.is_some_and(|remaining| remaining.is_zero()) {
                state.cancel(ticket);
                self.changed.notify_all();
                return None;
            }
            if ticket == state.serving {
                return Some(ProjectionGuard { gate: self });
            }
            state = if let Some(remaining) = remaining {
                self.changed
                    .wait_timeout(state, remaining)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            } else {
                self.changed.wait(state).unwrap_or_else(PoisonError::into_inner)
            };
        }
    }
}

/// Releases the next live ticket on drop, including during unwinding.
#[must_use = "dropping the guard releases the projection gate"]
pub struct ProjectionGuard<'a> {
    gate: &'a ProjectionGate,
}

impl Drop for ProjectionGuard<'_> {
    fn drop(&mut self) {
        // Returning the ticket during unwinding is safe: completed rebuilds have
        // committed and the current SQLite transaction rolls back independently.
        let mut state = self.gate.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.advance();
        self.gate.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::{PoisonError, ProjectionGate};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    fn wait_for_tickets(gate: &ProjectionGate, count: u64) {
        let state = gate.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (state, timeout) = gate
            .changed
            .wait_timeout_while(state, Duration::from_secs(5), |state| state.next < count)
            .unwrap_or_else(PoisonError::into_inner);
        assert!(state.next >= count, "waiters did not enroll: {timeout:?}");
    }

    #[test]
    fn fifo_waiter_is_never_overtaken_by_later_arrivals() {
        let gate = Arc::new(ProjectionGate::default());
        let held = gate.lock();
        let (acquired, received) = mpsc::channel();
        let mut threads = Vec::new();
        for id in 0..16_u64 {
            let gate_for_waiter = Arc::clone(&gate);
            let acquired = acquired.clone();
            threads.push(thread::spawn(move || {
                let _guard = gate_for_waiter.lock();
                acquired.send(id).unwrap();
            }));
            // Each ticket is enrolled before a later arrival starts. The owner
            // remains held until all waiters are queued, so wake order is contested.
            wait_for_tickets(&gate, id + 2);
        }
        drop(held);
        for expected in 0..16_u64 {
            assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), expected);
        }
        for thread in threads {
            thread.join().unwrap();
        }
    }

    #[test]
    fn canceled_ticket_does_not_block_later_waiter() {
        let gate = Arc::new(ProjectionGate::default());
        let held = gate.lock();
        let (acquired, received) = mpsc::channel();
        let gate_for_older = Arc::clone(&gate);
        let older_acquired = acquired.clone();
        let older = thread::spawn(move || {
            let _guard = gate_for_older.lock();
            older_acquired.send(0).unwrap();
        });
        wait_for_tickets(&gate, 2);
        assert!(
            gate.lock_until(Instant::now() + Duration::from_millis(10))
                .is_none()
        );
        let tickets_before_later = gate.state.lock().unwrap_or_else(PoisonError::into_inner).next;
        let gate_for_later = Arc::clone(&gate);
        let later = thread::spawn(move || {
            let _guard = gate_for_later.lock();
            acquired.send(1).unwrap();
        });
        wait_for_tickets(&gate, tickets_before_later + 1);
        drop(held);
        assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
        assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
        older.join().unwrap();
        later.join().unwrap();
    }

    #[test]
    fn expired_head_ticket_leaves_idle_gate_available() {
        let gate = ProjectionGate::default();
        assert!(gate.lock_until(Instant::now()).is_none());
        let _guard = gate.lock();
    }

    #[test]
    fn panicking_owner_releases_the_next_fifo_waiter() {
        let gate = Arc::new(ProjectionGate::default());
        let (owner_ready, owner_started) = mpsc::channel();
        let (release_owner, released) = mpsc::channel();
        let gate_for_owner = Arc::clone(&gate);
        let owner = thread::spawn(move || {
            let _guard = gate_for_owner.lock();
            owner_ready.send(()).unwrap();
            released.recv().unwrap();
            panic!("projection owner failed");
        });
        owner_started.recv_timeout(Duration::from_secs(5)).unwrap();
        let (acquired, received) = mpsc::channel();
        let mut waiters = Vec::new();
        for id in 0..2_u64 {
            let gate_for_waiter = Arc::clone(&gate);
            let acquired = acquired.clone();
            waiters.push(thread::spawn(move || {
                let _guard = gate_for_waiter.lock();
                acquired.send(id).unwrap();
            }));
            wait_for_tickets(&gate, id + 2);
        }
        release_owner.send(()).unwrap();
        assert!(owner.join().is_err());
        for expected in 0..2_u64 {
            assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), expected);
        }
        for waiter in waiters {
            waiter.join().unwrap();
        }
        let _guard = gate.lock();
    }
}
