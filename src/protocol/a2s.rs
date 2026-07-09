//! Steamworks A2S protocol scaffolding.
//!
//! This module starts the migration away from legacy GameSpy queries.
//! It provides request builders plus challenge/fragment helpers used by the
//! server implementation.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// A2S packet prefix used by Steam query packets.
pub const A2S_HEADER: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];

/// A2S request opcodes.
pub const A2S_INFO: u8 = 0x54;
pub const A2S_PLAYER: u8 = 0x55;
pub const A2S_RULES: u8 = 0x56;
pub const A2S_CHALLENGE_RESPONSE: u8 = 0x41;

/// Builds a raw A2S_INFO request packet.
pub fn build_info_request() -> Vec<u8> {
    let mut packet = Vec::with_capacity(4 + 1 + 20);
    packet.extend_from_slice(&A2S_HEADER);
    packet.push(A2S_INFO);
    packet.extend_from_slice(b"Source Engine Query\0");
    packet
}

/// Builds a raw A2S_PLAYER request packet.
///
/// Use `-1` to request a challenge first.
pub fn build_player_request(challenge: i32) -> Vec<u8> {
    let mut packet = Vec::with_capacity(9);
    packet.extend_from_slice(&A2S_HEADER);
    packet.push(A2S_PLAYER);
    packet.extend_from_slice(&challenge.to_le_bytes());
    packet
}

/// Builds a raw A2S_RULES request packet.
///
/// Use `-1` to request a challenge first.
pub fn build_rules_request(challenge: i32) -> Vec<u8> {
    let mut packet = Vec::with_capacity(9);
    packet.extend_from_slice(&A2S_HEADER);
    packet.push(A2S_RULES);
    packet.extend_from_slice(&challenge.to_le_bytes());
    packet
}

/// Parses the response opcode from a raw A2S packet.
pub fn parse_response_opcode(data: &[u8]) -> Option<u8> {
    if data.len() < 5 || data[0..4] != A2S_HEADER {
        return None;
    }
    Some(data[4])
}

/// Tracks issued A2S challenges per peer.
#[derive(Debug)]
pub struct ChallengeManager {
    ttl: Duration,
    by_peer: HashMap<SocketAddr, (i32, Instant)>,
}

impl ChallengeManager {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            by_peer: HashMap::new(),
        }
    }

    /// Issues a deterministic challenge for now.
    /// TODO: replace with cryptographically strong random values.
    pub fn issue(&mut self, peer: SocketAddr) -> i32 {
        let challenge = (peer.port() as i32) ^ 0x5A5A_1234;
        self.by_peer.insert(peer, (challenge, Instant::now()));
        challenge
    }

    pub fn validate(&mut self, peer: SocketAddr, challenge: i32) -> bool {
        self.cleanup_expired();
        match self.by_peer.get(&peer) {
            Some((stored, _)) => *stored == challenge,
            None => false,
        }
    }

    pub fn cleanup_expired(&mut self) {
        let now = Instant::now();
        self.by_peer
            .retain(|_, (_, issued_at)| now.duration_since(*issued_at) <= self.ttl);
    }
}

/// Minimal fragment buffer scaffold for multi-packet A2S responses.
#[derive(Debug, Default)]
pub struct FragmentBuffer {
    fragments: HashMap<u32, Vec<Vec<u8>>>,
}

impl FragmentBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, request_id: u32, fragment: Vec<u8>) {
        self.fragments
            .entry(request_id)
            .or_default()
            .push(fragment);
    }

    /// TODO: Reassembly should honor fragment order metadata.
    pub fn take_reassembled(&mut self, request_id: u32) -> Option<Vec<u8>> {
        let mut parts = self.fragments.remove(&request_id)?;
        let total_len = parts.iter().map(Vec::len).sum();
        let mut out = Vec::with_capacity(total_len);
        for part in parts.drain(..) {
            out.extend_from_slice(&part);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    #[test]
    fn test_info_request_format() {
        let pkt = build_info_request();
        assert_eq!(&pkt[0..4], &A2S_HEADER);
        assert_eq!(pkt[4], A2S_INFO);
        assert!(pkt.ends_with(b"Source Engine Query\0"));
    }

    #[test]
    fn test_challenge_validation() {
        let mut manager = ChallengeManager::new(Duration::from_secs(60));
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 28000);
        let challenge = manager.issue(peer);
        assert!(manager.validate(peer, challenge));
        assert!(!manager.validate(peer, challenge.wrapping_add(1)));
    }
}
