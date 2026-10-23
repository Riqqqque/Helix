//! Per-server operation locks.
//!
//! Reads need no lock. File writes share the server, so several uploads can run at
//! once. Backups, restores, updates, transfers, and lifecycle actions hold it
//! exclusively. Background jobs wait in line for the lock instead of failing, and a
//! caller that does have to give up is told which operation holds the server.

use std::{
    collections::HashMap,
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};

/// How long a direct API call waits for a busy server before it gives up. It must
/// stay well inside the 15-second gateway timeout.
pub(super) const DIRECT_WAIT: Duration = Duration::from_secs(8);
/// How long a background job waits in line for the server.
pub(super) const QUEUED_WAIT: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
pub(super) struct OperationRegistry {
    state: Mutex<HashMap<String, Holders>>,
    changed: Condvar,
}

#[derive(Default)]
struct Holders {
    exclusive: Option<Holder>,
    shared: Vec<Holder>,
    /// Exclusive callers waiting; new shared holders wait behind them so a backup is
    /// not starved by a steady stream of uploads.
    exclusive_waiting: usize,
}

#[derive(Clone)]
struct Holder {
    token: u64,
    operation: String,
    started: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Shared,
    Exclusive,
}

pub(crate) struct InstanceOperationGuard<'a> {
    registry: &'a OperationRegistry,
    key: String,
    token: u64,
}

impl Drop for InstanceOperationGuard<'_> {
    fn drop(&mut self) {
        let mut state = self
            .registry
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(holders) = state.get_mut(&self.key) {
            if holders
                .exclusive
                .as_ref()
                .is_some_and(|holder| holder.token == self.token)
            {
                holders.exclusive = None;
            }
            holders.shared.retain(|holder| holder.token != self.token);
            if holders.exclusive.is_none()
                && holders.shared.is_empty()
                && holders.exclusive_waiting == 0
            {
                state.remove(&self.key);
            }
        }
        self.registry.changed.notify_all();
    }
}

fn describe(holder: &Holder) -> String {
    let seconds = holder.started.elapsed().as_secs();
    let age = if seconds < 90 {
        format!("{seconds} seconds ago")
    } else {
        format!("{} minutes ago", seconds / 60)
    };
    format!("{} (started {age})", holder.operation)
}

impl OperationRegistry {
    fn next_token() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    /// Who is holding `key` right now, for messages and the server detail.
    pub(super) fn holder(&self, key: &str) -> Option<String> {
        let state = self.state.lock().ok()?;
        let holders = state.get(key)?;
        holders
            .exclusive
            .as_ref()
            .or_else(|| holders.shared.first())
            .map(describe)
    }

    fn acquire<'a>(
        &'a self,
        key: &str,
        operation: &str,
        mode: Mode,
        wait: Duration,
        mut waiting: impl FnMut(&str),
    ) -> Result<InstanceOperationGuard<'a>, String> {
        let deadline = Instant::now() + wait;
        let token = Self::next_token();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "the server operation registry failed".to_owned())?;
        let mut announced = false;
        let mut counted_waiting = false;
        loop {
            let holders = state.entry(key.to_owned()).or_default();
            let free = match mode {
                Mode::Exclusive => holders.exclusive.is_none() && holders.shared.is_empty(),
                Mode::Shared => holders.exclusive.is_none() && holders.exclusive_waiting == 0,
            };
            if free {
                if counted_waiting {
                    holders.exclusive_waiting = holders.exclusive_waiting.saturating_sub(1);
                }
                let holder = Holder {
                    token,
                    operation: operation.to_owned(),
                    started: Instant::now(),
                };
                match mode {
                    Mode::Exclusive => holders.exclusive = Some(holder),
                    Mode::Shared => holders.shared.push(holder),
                }
                return Ok(InstanceOperationGuard {
                    registry: self,
                    key: key.to_owned(),
                    token,
                });
            }
            let blocker = holders
                .exclusive
                .as_ref()
                .or_else(|| holders.shared.first())
                .map(describe)
                .unwrap_or_else(|| "another operation".to_owned());
            let now = Instant::now();
            if now >= deadline {
                if counted_waiting {
                    holders.exclusive_waiting = holders.exclusive_waiting.saturating_sub(1);
                }
                self.changed.notify_all();
                return Err(format!(
                    "{blocker} is still running on this server, so {operation} was not started. Try again when it finishes."
                ));
            }
            if mode == Mode::Exclusive && !counted_waiting {
                holders.exclusive_waiting += 1;
                counted_waiting = true;
            }
            if !announced {
                announced = true;
                drop(state);
                waiting(&blocker);
                state = self
                    .state
                    .lock()
                    .map_err(|_| "the server operation registry failed".to_owned())?;
                continue;
            }
            let (next, _) = self
                .changed
                .wait_timeout(state, (deadline - now).min(Duration::from_secs(1)))
                .map_err(|_| "the server operation registry failed".to_owned())?;
            state = next;
        }
    }

    pub(super) fn exclusive<'a>(
        &'a self,
        key: &str,
        operation: &str,
        wait: Duration,
        waiting: impl FnMut(&str),
    ) -> Result<InstanceOperationGuard<'a>, String> {
        self.acquire(key, operation, Mode::Exclusive, wait, waiting)
    }

    pub(super) fn shared<'a>(
        &'a self,
        key: &str,
        operation: &str,
        wait: Duration,
    ) -> Result<InstanceOperationGuard<'a>, String> {
        self.acquire(key, operation, Mode::Shared, wait, |_| {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn shared_holders_run_together_and_exclusive_waits_for_them() {
        let registry = Arc::new(OperationRegistry::default());
        let first = registry.shared("a", "upload", Duration::ZERO).unwrap();
        let second = registry.shared("a", "upload", Duration::ZERO).unwrap();
        let refused = registry.exclusive("a", "backup", Duration::ZERO, |_| {});
        assert!(refused.err().unwrap().contains("upload (started"));
        let waiter = {
            let registry = Arc::clone(&registry);
            thread::spawn(move || {
                registry
                    .exclusive("a", "backup", Duration::from_secs(5), |_| {})
                    .map(|_| ())
            })
        };
        thread::sleep(Duration::from_millis(100));
        drop(first);
        drop(second);
        assert!(waiter.join().unwrap().is_ok());
    }

    #[test]
    fn exclusive_names_the_holder_and_blocks_new_shared_work() {
        let registry = OperationRegistry::default();
        let backup = registry
            .exclusive("a", "backup", Duration::ZERO, |_| {})
            .unwrap();
        let error = registry
            .shared("a", "server files", Duration::ZERO)
            .err()
            .unwrap();
        assert!(error.starts_with("backup (started"), "{error}");
        assert!(error.contains("server files was not started"), "{error}");
        assert!(registry.holder("a").unwrap().starts_with("backup"));
        // Other servers are independent.
        assert!(registry.shared("b", "server files", Duration::ZERO).is_ok());
        drop(backup);
        assert!(registry.holder("a").is_none());
        assert!(registry.shared("a", "server files", Duration::ZERO).is_ok());
    }

    #[test]
    fn a_waiting_job_is_told_what_it_waits_for() {
        let registry = Arc::new(OperationRegistry::default());
        let held = registry
            .exclusive("a", "backup purge", Duration::ZERO, |_| {})
            .unwrap();
        let worker = {
            let registry = Arc::clone(&registry);
            thread::spawn(move || {
                let mut seen = String::new();
                let result = registry
                    .exclusive("a", "deploy", Duration::from_secs(5), |blocker| {
                        seen = blocker.to_owned()
                    })
                    .map(|_| ());
                (result, seen)
            })
        };
        thread::sleep(Duration::from_millis(150));
        drop(held);
        let (result, seen) = worker.join().unwrap();
        assert!(result.is_ok());
        assert!(seen.starts_with("backup purge"), "{seen}");
    }
}
