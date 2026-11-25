use super::{ExecRuntime, OwnedChildKey};
use std::collections::{BTreeMap, VecDeque};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};

const FALLBACK_INTERVAL: Duration = Duration::from_millis(100);

/// One process-wide reactor, initialized fallibly before spawning any owned child. Its lifetime is
/// the runtime host's lifetime; completed registrations release their runtime/map references.
/// SIGCHLD cannot safely be redirected after other host threads/subsystems already exist. Only
/// children whose native subscription fails use a nonblocking exact-child deadline probe.
pub(super) struct CompletionReactor {
    inner: Arc<Inner>,
}

struct Inner {
    poller: Poller,
    state: Mutex<State>,
    #[cfg(test)]
    changed: std::sync::Condvar,
}

#[derive(Default)]
struct State {
    next_token: u64,
    entries: BTreeMap<u64, Entry>,
    ready: VecDeque<u64>,
    deadlines: BTreeMap<(Instant, u64), ()>,
}

struct Entry {
    runtime: ExecRuntime,
    key: OwnedChildKey,
    _subscription: Option<NativeSubscription>,
    fallback: bool,
    deadline: Option<Instant>,
    retry_delay: Duration,
    #[cfg(test)]
    fallback_scans: usize,
}

impl CompletionReactor {
    pub(super) fn get() -> Result<&'static Self> {
        static REACTOR: LazyLock<std::result::Result<CompletionReactor, String>> =
            LazyLock::new(|| CompletionReactor::start().map_err(|error| format!("{error:#}")));
        REACTOR.as_ref().map_err(|error| anyhow::anyhow!("initialize owned child reactor: {error}"))
    }

    fn start() -> Result<Self> {
        Self::start_with(|worker| {
            std::thread::Builder::new().name("exec-completion".into()).spawn(move || {
                if let Err(error) = worker.run() {
                    tracing::error!(%error, "owned child completion reactor failed");
                }
            }).map(|_worker| ())
        })
    }

    fn start_with(spawn: impl FnOnce(Arc<Inner>) -> std::io::Result<()>) -> Result<Self> {
        let inner = Arc::new(Inner {
            poller: Poller::new()?,
            state: Mutex::new(State::default()),
            #[cfg(test)]
            changed: std::sync::Condvar::new(),
        });
        spawn(Arc::clone(&inner)).context("start owned child completion reactor")?;
        Ok(Self { inner })
    }

    pub(super) fn register(&self, runtime: ExecRuntime, key: OwnedChildKey, start_token: u64) -> u64 {
        self.register_inner(runtime, key, Some(start_token))
    }

    #[cfg(test)]
    pub(super) fn register_fallback(&self, runtime: ExecRuntime, key: OwnedChildKey) -> u64 {
        self.register_inner(runtime, key, None)
    }

    fn register_inner(&self, runtime: ExecRuntime, key: OwnedChildKey, start_token: Option<u64>) -> u64 {
        let mut state = self.inner.state.lock().expect("exec reactor mutex poisoned");
        let token = loop {
            state.next_token = state.next_token.wrapping_add(1).max(1);
            if !state.entries.contains_key(&state.next_token) {
                break state.next_token;
            }
        };
        let subscription = start_token.map_or_else(
            || Err(anyhow::anyhow!("native subscription unavailable")),
            |start_token| self.inner.poller.subscribe(key.pid, start_token, token),
        );
        let (subscription, fallback, ready) = match subscription {
            Ok(Some(subscription)) => (Some(subscription), false, false),
            Ok(None) => (None, false, true),
            Err(error) => {
                tracing::warn!(runtime_id = key.id, generation = key.generation, pid = key.pid,
                    %error, "owned child uses bounded exact-child fallback probes");
                (None, true, false)
            }
        };
        let deadline = fallback.then(|| Instant::now() + FALLBACK_INTERVAL);
        state.entries.insert(token, Entry {
            runtime, key, _subscription: subscription, fallback, deadline,
            retry_delay: Duration::from_millis(100),
            #[cfg(test)]
            fallback_scans: 0,
        });
        if let Some(deadline) = deadline {
            state.deadlines.insert((deadline, token), ());
        }
        if ready {
            state.ready.push_back(token);
        }
        drop(state);
        self.inner.poller.wake();
        token
    }

    #[cfg(test)]
    pub(super) fn wait_for_fallback_scan(&self, token: u64) -> bool {
        let state = self.inner.state.lock().expect("exec reactor mutex poisoned");
        let (state, _) = self.inner.changed.wait_timeout_while(state, Duration::from_secs(5), |state| {
            state.entries.get(&token).is_none_or(|entry| entry.fallback_scans == 0)
        }).expect("exec reactor mutex poisoned");
        state.entries.get(&token).is_some_and(|entry| entry.fallback_scans > 0)
    }
}

impl Inner {
    fn run(&self) -> Result<()> {
        loop {
            let token = {
                let mut state = self.state.lock().expect("exec reactor mutex poisoned");
                let now = Instant::now();
                while state.deadlines.first_key_value().is_some_and(|((deadline, _), ())| *deadline <= now) {
                    let ((_, token), ()) = state.deadlines.pop_first().expect("deadline present");
                    state.ready.push_back(token);
                }
                state.ready.pop_front()
            };
            if let Some(token) = token {
                self.finish(token);
                continue;
            }
            let timeout = {
                let state = self.state.lock().expect("exec reactor mutex poisoned");
                state.deadlines.first_key_value().map_or(-1, |((deadline, _), ())| {
                    i32::try_from(deadline.saturating_duration_since(Instant::now()).as_millis()
                        .saturating_add(1)).unwrap_or(i32::MAX)
                })
            };
            let mut ready = [0; 64];
            let count = self.poller.wait(timeout, &mut ready)?;
            let mut state = self.state.lock().expect("exec reactor mutex poisoned");
            state.ready.extend(ready[..count].iter().copied());
        }
    }

    fn finish(&self, token: u64) {
        let Some(mut entry) = ({
            let mut state = self.state.lock().expect("exec reactor mutex poisoned");
            let entry = state.entries.remove(&token);
            if let Some(deadline) = entry.as_ref().and_then(|entry| entry.deadline) {
                state.deadlines.remove(&(deadline, token));
            }
            entry
        }) else { return };
        // Never hold the reactor lock while taking a child-map lock or performing receipt I/O.
        // Once status is consumed, receipt retries must not probe a possibly reused PID.
        let needs_probe = entry.fallback && matches!(
            entry.runtime.owned_children.lock().expect("exec child mutex poisoned").get(&entry.key),
            Some(super::OwnedChild::Running(_))
        );
        let completion = if needs_probe {
            super::owned_child_waitable(entry.key.pid).and_then(|ready| {
                if ready { entry.runtime.finish_owned_child(&entry.key) } else { Ok(false) }
            })
        } else {
            entry.runtime.finish_owned_child(&entry.key)
        };
        let delay = match completion {
            Ok(true) => return,
            Ok(false) => {
                // A native notification should mean whole-process waitability. If it does not,
                // retain custody and use the same bounded nonblocking fallback, never block here.
                entry.fallback = true;
                #[cfg(test)]
                { entry.fallback_scans += 1; }
                FALLBACK_INTERVAL
            }
            Err(error) => {
                tracing::warn!(runtime_id = entry.key.id, generation = entry.key.generation,
                    pid = entry.key.pid, %error, "owned child completion remains pending");
                let delay = entry.retry_delay;
                entry.retry_delay = (entry.retry_delay * 2).min(Duration::from_secs(5));
                delay
            }
        };
        let deadline = Instant::now() + delay;
        entry.deadline = Some(deadline);
        let mut state = self.state.lock().expect("exec reactor mutex poisoned");
        state.entries.insert(token, entry);
        state.deadlines.insert((deadline, token), ());
        #[cfg(test)]
        self.changed.notify_all();
    }
}

struct NativeSubscription {
    #[cfg(target_os = "linux")]
    _descriptor: OwnedFd,
}

struct Poller {
    descriptor: OwnedFd,
    #[cfg(target_os = "linux")]
    wake: OwnedFd,
}

impl Poller {
    #[cfg(target_os = "linux")]
    fn new() -> Result<Self> {
        let descriptor = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        anyhow::ensure!(descriptor >= 0, "create child epoll: {}", std::io::Error::last_os_error());
        // SAFETY: the successful syscall returned a fresh owned descriptor.
        let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
        let wake = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        anyhow::ensure!(wake >= 0, "create child reactor wake: {}", std::io::Error::last_os_error());
        // SAFETY: the successful syscall returned a fresh owned descriptor.
        let wake = unsafe { OwnedFd::from_raw_fd(wake) };
        let mut event = libc::epoll_event { events: libc::EPOLLIN as u32, u64: 0 };
        anyhow::ensure!(unsafe {
            libc::epoll_ctl(descriptor.as_raw_fd(), libc::EPOLL_CTL_ADD, wake.as_raw_fd(), &mut event)
        } == 0, "register child reactor wake: {}", std::io::Error::last_os_error());
        Ok(Self { descriptor, wake })
    }

    #[cfg(target_os = "linux")]
    fn subscribe(&self, pid: u32, start_token: u64, token: u64) -> Result<Option<NativeSubscription>> {
        let Some(subscription) = super::subscribe_process_exit(pid, start_token)? else { return Ok(None) };
        let mut event = libc::epoll_event {
            events: (libc::EPOLLIN | libc::EPOLLONESHOT) as u32, u64: token,
        };
        anyhow::ensure!(unsafe {
            libc::epoll_ctl(self.descriptor.as_raw_fd(), libc::EPOLL_CTL_ADD,
                subscription.descriptor.as_raw_fd(), &mut event)
        } == 0, "register owned child pidfd: {}", std::io::Error::last_os_error());
        Ok(Some(NativeSubscription { _descriptor: subscription.descriptor }))
    }

    #[cfg(target_os = "linux")]
    fn wake(&self) {
        let value = 1_u64;
        loop {
            let result = unsafe {
                libc::write(self.wake.as_raw_fd(), std::ptr::from_ref(&value).cast(), std::mem::size_of_val(&value))
            };
            if result >= 0 { return }
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted { continue }
            if error.raw_os_error() != Some(libc::EAGAIN) {
                tracing::error!(%error, "wake owned child reactor failed");
            }
            return;
        }
    }

    #[cfg(target_os = "linux")]
    fn wait(&self, timeout: i32, ready: &mut [u64; 64]) -> Result<usize> {
        let mut events = [libc::epoll_event { events: 0, u64: 0 }; 64];
        let count = loop {
            let count = unsafe {
                libc::epoll_wait(self.descriptor.as_raw_fd(), events.as_mut_ptr(), 64, timeout)
            };
            if count >= 0 { break count as usize }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted { return Err(error.into()) }
        };
        let mut ready_count = 0;
        for event in &events[..count] {
            if event.u64 == 0 {
                let mut value = 0_u64;
                unsafe {
                    libc::read(self.wake.as_raw_fd(), std::ptr::from_mut(&mut value).cast(), std::mem::size_of_val(&value));
                }
            } else {
                ready[ready_count] = event.u64;
                ready_count += 1;
            }
        }
        Ok(ready_count)
    }

    #[cfg(target_os = "macos")]
    fn new() -> Result<Self> {
        let descriptor = unsafe { libc::kqueue() };
        anyhow::ensure!(descriptor >= 0, "create child kqueue: {}", std::io::Error::last_os_error());
        // SAFETY: the successful syscall returned a fresh owned descriptor.
        let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
        let poller = Self { descriptor };
        poller.change(0, libc::EVFILT_USER, libc::EV_ADD | libc::EV_CLEAR, 0, 0)?;
        Ok(poller)
    }

    #[cfg(target_os = "macos")]
    fn change(&self, pid: u32, filter: i16, flags: u16, fflags: u32, token: u64) -> Result<()> {
        // SAFETY: kevent is a plain C event record, initialized before passing it to the kernel.
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        event.ident = pid as libc::uintptr_t;
        event.filter = filter;
        event.flags = flags;
        event.fflags = fflags;
        event.udata = token as usize as *mut libc::c_void;
        let result = unsafe {
            libc::kevent(self.descriptor.as_raw_fd(), &event, 1, std::ptr::null_mut(), 0, std::ptr::null())
        };
        if result < 0 { return Err(std::io::Error::last_os_error().into()) }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn subscribe(&self, pid: u32, _start_token: u64, token: u64) -> Result<Option<NativeSubscription>> {
        match self.change(pid, libc::EVFILT_PROC, libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT,
            libc::NOTE_EXIT, token)
        {
            Ok(()) => Ok(Some(NativeSubscription {})),
            Err(error) if error.downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.raw_os_error() == Some(libc::ESRCH)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    #[cfg(target_os = "macos")]
    fn wake(&self) {
        if let Err(error) = self.change(0, libc::EVFILT_USER, 0, libc::NOTE_TRIGGER, 0) {
            tracing::error!(%error, "wake owned child reactor failed");
        }
    }

    #[cfg(target_os = "macos")]
    fn wait(&self, timeout: i32, ready: &mut [u64; 64]) -> Result<usize> {
        // SAFETY: the kernel initializes the returned plain C event records.
        let mut events: [libc::kevent; 64] = unsafe { std::mem::zeroed() };
        let deadline = (timeout >= 0).then(|| libc::timespec {
            tv_sec: (timeout / 1000).into(), tv_nsec: ((timeout % 1000) * 1_000_000).into(),
        });
        let count = loop {
            let count = unsafe {
                libc::kevent(self.descriptor.as_raw_fd(), std::ptr::null(), 0, events.as_mut_ptr(), 64,
                    deadline.as_ref().map_or(std::ptr::null(), std::ptr::from_ref))
            };
            if count >= 0 { break count as usize }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted { return Err(error.into()) }
        };
        let mut ready_count = 0;
        for event in &events[..count] {
            // kevent is packed on macOS; formatting must borrow an aligned local.
            let data = event.data;
            anyhow::ensure!(event.flags & libc::EV_ERROR == 0, "owned child kqueue error: {data}");
            if event.filter == libc::EVFILT_PROC && event.fflags & libc::NOTE_EXIT != 0 {
                ready[ready_count] = event.udata as usize as u64;
                ready_count += 1;
            }
        }
        Ok(ready_count)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn new() -> Result<Self> {
        anyhow::bail!("owned child completion reactor requires epoll or kqueue")
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn subscribe(&self, _pid: u32, _start_token: u64, _token: u64) -> Result<Option<NativeSubscription>> {
        anyhow::bail!("owned child completion subscriptions unavailable")
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn wake(&self) {}

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn wait(&self, _timeout: i32, _ready: &mut [u64; 64]) -> Result<usize> {
        anyhow::bail!("owned child completion reactor unavailable")
    }
}

#[cfg(test)]
mod tests {
    use super::CompletionReactor;

    #[test]
    fn owned_reap_regression_thread_spawn_failure_returns_error() {
        let result = CompletionReactor::start_with(|_worker| {
            Err(std::io::Error::other("thread creation rejected"))
        });
        let error = result.err().expect("thread creation failure must be returned");
        assert!(format!("{error:#}").contains("thread creation rejected"));
    }
}
