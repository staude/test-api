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
    /// While set, so many keys are blocked that the map cannot take more;
    /// unknown e-mails count as blocked until this point in time, when
    /// enough blocked keys have expired.
    saturated_until: Option<Instant>,
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
        self.is_blocked_at(email, ip, Instant::now())
    }

    pub fn record_failure(&self, email: &str, ip: Option<&str>) {
        self.record_failure_at(email, ip, Instant::now());
    }

    pub fn record_success(&self, email: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.failures.remove(&email_key(email));
        }
    }

    fn is_blocked_at(&self, email: &str, ip: Option<&str>, now: Instant) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return true;
        };
        // Saturation ends on its own once enough blocked keys expire; no
        // failure needs to be recorded for that (unknown keys are refused
        // before any password check, so none would be).
        if state.saturated_until.is_some_and(|t| now >= t) {
            self.shrink(&mut state, now);
        }
        let saturated = state.saturated_until.is_some();
        let mut over =
            |key: String, max: usize, unknown_blocked: bool| match state.failures.get_mut(&key) {
                Some(q) => {
                    self.expire(q, now);
                    q.len() >= max
                }
                None => unknown_blocked,
            };
        let email_blocked = over(email_key(email), self.max_per_email, saturated);
        let ip_blocked = ip.is_some_and(|ip| over(ip_key(ip), self.max_per_ip, false));
        email_blocked || ip_blocked
    }

    fn record_failure_at(&self, email: &str, ip: Option<&str>, now: Instant) {
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
    ///
    /// Shrinks to 90 % of the limit, so this O(n) pass runs only once per
    /// `max_keys / 10` new keys, not on every failure.
    fn shrink(&self, state: &mut State, now: Instant) {
        state.failures.retain(|_, q| {
            self.expire(q, now);
            !q.is_empty()
        });

        let low_water = self.max_keys - self.max_keys / 10;
        if state.failures.len() > low_water {
            let mut evictable: Vec<(Instant, &String)> = state
                .failures
                .iter()
                .filter(|(k, q)| q.len() < self.limit_for(k))
                .filter_map(|(k, q)| q.back().map(|last| (*last, k)))
                .collect();
            let count = (state.failures.len() - low_water).min(evictable.len());
            if count > 0 && count < evictable.len() {
                evictable.select_nth_unstable(count - 1);
            }
            let doomed: Vec<String> = evictable
                .into_iter()
                .take(count)
                .map(|(_, k)| k.clone())
                .collect();
            for key in doomed {
                state.failures.remove(&key);
            }
        }

        // When does a blocked key stop being blocked? When its oldest
        // failure that still counts towards the limit expires.
        let mut releases: Vec<Instant> = state
            .failures
            .iter()
            .filter_map(|(k, q)| {
                let limit = self.limit_for(k);
                (q.len() >= limit)
                    .then(|| q.get(q.len() - limit))
                    .flatten()
                    .map(|t| *t + self.window)
            })
            .collect();
        state.saturated_until = if releases.len() >= self.max_keys {
            // Saturation ends once the blocked count drops below the limit.
            let nth = releases.len() - self.max_keys;
            releases.select_nth_unstable(nth);
            releases.get(nth).copied()
        } else {
            None
        };
        if state.saturated_until.is_some() {
            tracing::warn!(
                blocked = releases.len(),
                "login limiter saturated with blocked keys; unknown e-mails are blocked"
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
    fn saturation_ends_when_blocked_keys_expire() {
        let l = small();
        let t0 = Instant::now();
        for i in 0..150 {
            for _ in 0..10 {
                l.record_failure_at(&format!("spray{i}@example.org"), None, t0);
            }
        }
        assert!(l.is_blocked_at("new@example.org", None, t0));
        // Nothing is recorded meanwhile; the next check after the window
        // must lift the saturation by itself.
        let later = t0 + l.window + Duration::from_secs(1);
        assert!(!l.is_blocked_at("new@example.org", None, later));
        assert!(!l.is_blocked_at("spray0@example.org", None, later));
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
