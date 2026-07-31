//! Shared auth primitives for server-authoritative networking.
//!
//! Proofs are authenticated with HMAC-SHA256 (a real keyed MAC, not a
//! general-purpose hash) and session tokens/nonces are drawn from a CSPRNG.
//! This replaces an earlier DefaultHasher+timestamp scheme that was
//! forgeable/guessable by anyone who knew the algorithm.

use hmac::{Hmac, KeyInit, Mac};
use rand::RngExt;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

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
    // u64 equality here is a single-word compare; the real integrity
    // guarantee comes from HMAC-SHA256 being unforgeable without the
    // secret, not from constant-time comparison of the truncated digest.
    proof.digest == proof_digest(shared_secret, proof.player_id, proof.nonce)
}

pub fn mint_session_token(server_secret: &str, player_id: u64, nonce: u64) -> SessionToken {
    let low = proof_digest(server_secret, player_id, nonce) as u128;
    let high = (proof_digest(server_secret, nonce, player_id) as u128) << 64;
    let random = fresh_nonce() as u128;

    high ^ low ^ random
}

/// Draw a fresh random u64 from a CSPRNG. Used for nonces and to salt
/// session tokens — replaces the old low-entropy wall-clock timestamp.
pub fn fresh_nonce() -> u64 {
    rand::rng().random::<u64>()
}

fn proof_digest(secret: &str, a: u64, b: u64) -> u64 {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts keys of any length");
    mac.update(&a.to_le_bytes());
    mac.update(&b.to_le_bytes());
    let full = mac.finalize().into_bytes();
    u64::from_le_bytes(full[0..8].try_into().expect("HMAC-SHA256 output is 32 bytes"))
}
