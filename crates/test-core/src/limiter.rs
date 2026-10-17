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
    failures: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(15 * 60),
            max_per_email: 10,
            max_per_ip: 30,
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
            // Keep memory bounded under attack.
            if map.len() > 100_000 {
                map.clear();
            }
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
