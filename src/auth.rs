//! Shared auth primitives for server-authoritative networking.
//!
//! This is intentionally lightweight and dependency-free so the project can
//! enforce trust boundaries before introducing full cryptographic plumbing.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

pub type SessionToken = u128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthProof {
    pub player_id: u64,
    pub nonce: u64,
    pub digest: u64,
}

pub fn make_auth_proof(shared_secret: &str, player_id: u64, nonce: u64) -> AuthProof {
    AuthProof {
        player_id,
        nonce,
        digest: proof_digest(shared_secret, player_id, nonce),
    }
}

pub fn verify_auth_proof(shared_secret: &str, proof: AuthProof) -> bool {
    proof.digest == proof_digest(shared_secret, proof.player_id, proof.nonce)
}

pub fn mint_session_token(server_secret: &str, player_id: u64, nonce: u64) -> SessionToken {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let low = proof_digest(server_secret, player_id, nonce) as u128;
    let high = (proof_digest(server_secret, nonce, player_id) as u128) << 64;

    high ^ low ^ now
}

fn proof_digest(secret: &str, a: u64, b: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    secret.hash(&mut hasher);
    a.hash(&mut hasher);
    b.hash(&mut hasher);
    hasher.finish()
}
