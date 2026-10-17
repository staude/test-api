use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// In-memory limiter for failed login attempts, keyed by e-mail and by IP.
///
/// Sufficient for a single instance; a multi-instance setup would move this
/// into the database or Redis.
pub struct LoginLimiter {
    window: Duration,
    max_per_email: usize,
    max_per_ip: usize,
    /// Upper bound for tracked keys (e-mails and IPs).
    max_keys: usize,
    failures: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(15 * 60),
            max_per_email: 10,
            max_per_ip: 30,
            max_keys: 100_000,
            failures: Mutex::new(HashMap::new()),
        }
    }
}

impl LoginLimiter {
    pub fn is_blocked(&self, email: &str, ip: Option<&str>) -> bool {
        let now = Instant::now();
        let Ok(mut map) = self.failures.lock() else {
            return false;
        };
        let over = |map: &mut HashMap<String, VecDeque<Instant>>, key: String, max: usize| {
            map.get_mut(&key).is_some_and(|q| {
                while q
                    .front()
                    .is_some_and(|t| now.duration_since(*t) > self.window)
                {
                    q.pop_front();
                }
                q.len() >= max
            })
        };
        let email_blocked = over(&mut map, email_key(email), self.max_per_email);
        let ip_blocked = ip.is_some_and(|ip| over(&mut map, ip_key(ip), self.max_per_ip));
        email_blocked || ip_blocked
    }

    pub fn record_failure(&self, email: &str, ip: Option<&str>) {
        let now = Instant::now();
        if let Ok(mut map) = self.failures.lock() {
            map.entry(email_key(email)).or_default().push_back(now);
            if let Some(ip) = ip {
                map.entry(ip_key(ip)).or_default().push_back(now);
            }
            if map.len() > self.max_keys {
                self.shrink(&mut map, now);
            }
        }
    }

    /// Keeps memory bounded under attack without resetting the counters
    /// that matter: expired entries go first, then the keys with the fewest
    /// failures. Clearing everything would let an attacker reset the
    /// counter of a targeted account by spraying random e-mails.
    fn shrink(&self, map: &mut HashMap<String, VecDeque<Instant>>, now: Instant) {
        map.retain(|_, q| {
            while q
                .front()
                .is_some_and(|t| now.duration_since(*t) > self.window)
            {
                q.pop_front();
            }
            !q.is_empty()
        });
        let mut threshold = 1;
        while map.len() > self.max_keys {
            map.retain(|_, q| q.len() > threshold);
            threshold += 1;
        }
    }

    pub fn record_success(&self, email: &str) {
        if let Ok(mut map) = self.failures.lock() {
            map.remove(&email_key(email));
        }
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

    #[test]
    fn spraying_other_emails_does_not_reset_a_target() {
        let l = LoginLimiter {
            max_keys: 100,
            ..LoginLimiter::default()
        };
        for _ in 0..10 {
            l.record_failure("victim@example.org", None);
        }
        assert!(l.is_blocked("victim@example.org", None));
        for i in 0..1_000 {
            l.record_failure(&format!("spray{i}@example.org"), None);
        }
        assert!(l.is_blocked("victim@example.org", None));
        assert!(l.failures.lock().map(|m| m.len() <= 101).unwrap_or(false));
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
