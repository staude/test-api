use std::sync::OnceLock;

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Prefix of all API tokens, so secret scanners can recognise them.
pub const TOKEN_PREFIX: &str = "tapp_";

/// 256 bit of randomness, hex encoded.
pub fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    hex::encode(bytes)
}

pub fn sha256(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}

pub fn new_api_token() -> String {
    format!("{TOKEN_PREFIX}{}", random_secret())
}

/// Each Argon2 run needs about 19 MiB. Without a limit, a burst of parallel
/// logins grows the heap by that much per login (and the allocator keeps it),
/// so at most one run per CPU core happens at a time.
fn argon2_slots() -> &'static tokio::sync::Semaphore {
    static SLOTS: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    SLOTS.get_or_init(|| {
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        tokio::sync::Semaphore::new(cores)
    })
}

/// Argon2 is CPU-bound, so it runs on the blocking thread pool.
pub async fn hash_password(password: String) -> Result<String> {
    let _slot = argon2_slots()
        .acquire()
        .await
        .map_err(|e| Error::Internal(e.to_string()))?;
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|h| h.to_string())
            .map_err(|e| Error::Internal(e.to_string()))
    })
    .await
    .map_err(|e| Error::Internal(e.to_string()))?
}

pub async fn verify_password(password: String, hash: String) -> bool {
    let Ok(_slot) = argon2_slots().acquire().await else {
        return false;
    };
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash)
            .map(|parsed| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok()
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn password_roundtrip() {
        let hash = hash_password("correct horse battery".into()).await.unwrap();
        assert!(verify_password("correct horse battery".into(), hash.clone()).await);
        assert!(!verify_password("wrong".into(), hash).await);
    }

    #[test]
    fn tokens_have_prefix_and_length() {
        let t = new_api_token();
        assert!(t.starts_with(TOKEN_PREFIX));
        assert_eq!(t.len(), TOKEN_PREFIX.len() + 64);
    }
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn many_parallel_verifications_complete() {
        let hash = hash_password("parallel-passwort".into()).await.unwrap();
        let tasks: Vec<_> = (0..64)
            .map(|_| tokio::spawn(verify_password("parallel-passwort".into(), hash.clone())))
            .collect();
        for t in tasks {
            assert!(t.await.unwrap());
        }
    }
}
