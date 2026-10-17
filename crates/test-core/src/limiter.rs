use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// In-memory limiter for failed login attempts, keyed by e-mail and by IP.
///
/// Sufficient for a single instance; a multi-instance setup would move this
/// into the database or Redis.
///
/// Under attack it fails closed: blocked keys are never evicted, and when
/// memory is full of blocked keys only, unknown keys count as blocked until
/// entries expire.
///
/// Residual risk: a key that is not blocked yet can still be evicted if more
/// than `max_keys` newer failures arrive between two attempts on it, which
/// takes over 100,000 requests from thousands of IPs. Closing that needs a
/// shared store (database or Redis).
pub struct LoginLimiter {
    window: Duration,
    max_per_email: usize,
    max_per_ip: usize,
    /// Upper bound for tracked keys (e-mails and IPs).
    max_keys: usize,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    failures: HashMap<String, VecDeque<Instant>>,
    /// Set when the map holds only blocked keys and cannot take more.
    saturated: bool,
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(15 * 60),
            max_per_email: 10,
            max_per_ip: 30,
            max_keys: 100_000,
            state: Mutex::new(State::default()),
        }
    }
}

impl LoginLimiter {
    pub fn is_blocked(&self, email: &str, ip: Option<&str>) -> bool {
        let now = Instant::now();
        let Ok(mut state) = self.state.lock() else {
            return true;
        };
        let saturated = state.saturated;
        let mut over = |key: String, max: usize| match state.failures.get_mut(&key) {
            Some(q) => {
                self.expire(q, now);
                q.len() >= max
            }
            None => saturated,
        };
        let email_blocked = over(email_key(email), self.max_per_email);
        let ip_blocked = ip.is_some_and(|ip| over(ip_key(ip), self.max_per_ip));
        email_blocked || ip_blocked
    }

    pub fn record_failure(&self, email: &str, ip: Option<&str>) {
        let now = Instant::now();
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state
            .failures
            .entry(email_key(email))
            .or_default()
            .push_back(now);
        if let Some(ip) = ip {
            state.failures.entry(ip_key(ip)).or_default().push_back(now);
        }
        if state.failures.len() > self.max_keys {
            self.shrink(&mut state, now);
        }
    }

    pub fn record_success(&self, email: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.failures.remove(&email_key(email));
        }
    }

    fn expire(&self, q: &mut VecDeque<Instant>, now: Instant) {
        while q
            .front()
            .is_some_and(|t| now.duration_since(*t) > self.window)
        {
            q.pop_front();
        }
    }

    fn limit_for(&self, key: &str) -> usize {
        if key.starts_with("i:") {
            self.max_per_ip
        } else {
            self.max_per_email
        }
    }

    /// Keeps memory bounded without resetting counters that matter. Expired
    /// entries go first, then the least recently failed unblocked keys, so
    /// a key under active attack keeps its counter. Blocked keys always
    /// stay; once they alone fill the limit, the limiter saturates.
    fn shrink(&self, state: &mut State, now: Instant) {
        state.failures.retain(|_, q| {
            self.expire(q, now);
            !q.is_empty()
        });
        let blocked = state
            .failures
            .iter()
            .filter(|(k, q)| q.len() >= self.limit_for(k))
            .count();
        if state.failures.len() > self.max_keys {
            let mut evictable: Vec<(Instant, String)> = state
                .failures
                .iter()
                .filter(|(k, q)| q.len() < self.limit_for(k))
                .filter_map(|(k, q)| q.back().map(|last| (*last, k.clone())))
                .collect();
            evictable.sort();
            let excess = state.failures.len() - self.max_keys;
            for (_, key) in evictable.into_iter().take(excess) {
                state.failures.remove(&key);
            }
        }
        state.saturated = blocked >= self.max_keys;
        if state.saturated {
            tracing::warn!(
                keys = state.failures.len(),
                "login limiter saturated with blocked keys; unknown keys are blocked"
            );
        }
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.state.lock().map(|s| s.failures.len()).unwrap_or(0)
    }
}

fn email_key(email: &str) -> String {
    format!("e:{}", email.to_lowercase())
}

fn ip_key(ip: &str) -> String {
    format!("i:{ip}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> LoginLimiter {
        LoginLimiter {
            max_keys: 100,
            ..LoginLimiter::default()
        }
    }

    fn block(l: &LoginLimiter, email: &str) {
        for _ in 0..10 {
            l.record_failure(email, None);
        }
    }

    #[test]
    fn spraying_unblocked_emails_does_not_reset_a_target() {
        let l = small();
        block(&l, "victim@example.org");
        for i in 0..1_000 {
            l.record_failure(&format!("spray{i}@example.org"), None);
        }
        assert!(l.is_blocked("victim@example.org", None));
        assert!(l.tracked() <= 101);
        assert!(!l.is_blocked("new@example.org", None));
    }

    #[test]
    fn a_new_target_still_gets_blocked_while_memory_is_full() {
        let l = small();
        for i in 0..1_000 {
            l.record_failure(&format!("spray{i}@example.org"), None);
        }
        block(&l, "target@example.org");
        assert!(l.is_blocked("target@example.org", None));
    }

    #[test]
    fn spraying_blocked_emails_fails_closed() {
        let l = small();
        block(&l, "victim@example.org");
        // An attacker with many IPs blocks far more keys than fit.
        for i in 0..300 {
            block(&l, &format!("spray{i}@example.org"));
        }
        assert!(l.is_blocked("victim@example.org", None));
        // Memory is full of blocked keys: unknown keys are refused too.
        assert!(l.is_blocked("new@example.org", None));
    }

    #[test]
    fn blocks_after_max_failures_and_resets_on_success() {
        let l = LoginLimiter::default();
        for _ in 0..10 {
            assert!(!l.is_blocked("a@b.c", None));
            l.record_failure("A@b.c", None);
        }
        assert!(l.is_blocked("a@b.c", None));
        l.record_success("a@b.c");
        assert!(!l.is_blocked("a@b.c", None));
    }
}
