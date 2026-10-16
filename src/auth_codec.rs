//! Wire-format codecs for the Steam-transport authentication and input protocol.
//!
//! Each function encodes or decodes a single packet type identified by a
//! 1-byte discriminator that follows the shared magic/version header.
//! All multi-byte integers are little-endian.

use bevy::prelude::*;
use learning::auth::{AuthProof, SessionToken};

// Auth handshake sequence: hello → challenge → proof → accept
pub const PACKET_AUTH_HELLO: u8 = 10;
pub const PACKET_AUTH_CHALLENGE: u8 = 11;
pub const PACKET_AUTH_PROOF: u8 = 12;
pub const PACKET_AUTH_ACCEPT: u8 = 13;
// Ongoing game traffic once a session is established
pub const PACKET_INPUT: u8 = 14;     // client → host: movement axes + color
pub const PACKET_SNAPSHOT: u8 = 15; // host → clients: authoritative state for all players
pub const MAX_PACKET_BYTES: usize = 1200;
pub const MAX_SNAPSHOT_STATES: usize = (MAX_PACKET_BYTES - 10) / 24;

pub fn snapshot_packets(magic: [u8; 4], version: u8, states: &[(u64, Transform, Color)]) -> Vec<Vec<u8>> {
    states.chunks(MAX_SNAPSHOT_STATES)
        .map(|chunk| encode_snapshot_packet(magic, version, chunk))
        .collect()
}

/// Reject unexpected senders and packets outside the current handshake stage.
pub fn client_packet_allowed(
    trusted_sender: bool,
    state: learning::client::ClientAuthState,
    data: &[u8],
) -> bool {
    use learning::client::ClientAuthState;
    if !trusted_sender {
        return false;
    }
    match data.get(5).copied() {
        Some(PACKET_AUTH_CHALLENGE) => matches!(
            state,
            ClientAuthState::AwaitingChallenge | ClientAuthState::AwaitingAcceptance
        ),
        Some(PACKET_AUTH_ACCEPT) => state == ClientAuthState::AwaitingAcceptance,
        Some(PACKET_SNAPSHOT) => state == ClientAuthState::Authenticated,
        _ => false,
    }
}

#[cfg(test)]
mod client_packet_policy_tests {
    #[test]
    fn reject_non_finite_movement_on_both_axes() {
        use super::*;
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(decode_input_payload(&encode_input_payload(bad, 0.0, false, Color::WHITE)).is_none());
            assert!(decode_input_payload(&encode_input_payload(0.0, bad, false, Color::WHITE)).is_none());
        }
        let result = decode_input_payload(&encode_input_payload(4.0, -4.0, true, Color::WHITE)).unwrap();
        assert_eq!((result.0, result.1, result.2), (1.0, -1.0, true));
    }

    use super::*;
    use learning::client::ClientAuthState::*;

    #[test]
    fn reject_untrusted_sender_in_every_stage() {
        for state in [Unauthenticated, AwaitingChallenge, AwaitingAcceptance, Authenticated] {
            for kind in [PACKET_AUTH_CHALLENGE, PACKET_AUTH_ACCEPT, PACKET_SNAPSHOT] {
                assert!(!client_packet_allowed(false, state, &[0, 0, 0, 0, 1, kind]));
            }
        }
    }

    #[test]
    fn enforce_handshake_packet_order() {
        let challenge = [0, 0, 0, 0, 1, PACKET_AUTH_CHALLENGE];
        let accept = [0, 0, 0, 0, 1, PACKET_AUTH_ACCEPT];
        let snapshot = [0, 0, 0, 0, 1, PACKET_SNAPSHOT];
        assert!(client_packet_allowed(true, AwaitingChallenge, &challenge));
        assert!(client_packet_allowed(true, AwaitingAcceptance, &challenge));
        assert!(client_packet_allowed(true, AwaitingAcceptance, &accept));
        assert!(client_packet_allowed(true, Authenticated, &snapshot));
        for state in [Unauthenticated, AwaitingChallenge, AwaitingAcceptance] {
            assert!(!client_packet_allowed(true, state, &snapshot));
        }
        assert!(!client_packet_allowed(true, AwaitingChallenge, &accept));
        assert!(!client_packet_allowed(true, Authenticated, &accept));
        assert!(!client_packet_allowed(true, Authenticated, &challenge));
        assert!(!client_packet_allowed(true, Authenticated, &[]));
    }
}

pub fn encode_auth_hello(magic: [u8; 4], version: u8, player_id: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(14);
    out.extend_from_slice(&magic);
    out.push(version);
    out.push(PACKET_AUTH_HELLO);
    out.extend_from_slice(&player_id.to_le_bytes());
    out
}

pub fn decode_auth_hello(magic: [u8; 4], version: u8, data: &[u8]) -> Option<u64> {
    if data.len() != 14 || data[0..4] != magic || data[4] != version || data[5] != PACKET_AUTH_HELLO {
        return None;
    }
    Some(u64::from_le_bytes(data[6..14].try_into().ok()?))
}

pub fn encode_auth_challenge(magic: [u8; 4], version: u8, nonce: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(14);
    out.extend_from_slice(&magic);
    out.push(version);
    out.push(PACKET_AUTH_CHALLENGE);
    out.extend_from_slice(&nonce.to_le_bytes());
    out
}

pub fn decode_auth_challenge(magic: [u8; 4], version: u8, data: &[u8]) -> Option<u64> {
    if data.len() != 14 || data[0..4] != magic || data[4] != version || data[5] != PACKET_AUTH_CHALLENGE {
        return None;
    }
    Some(u64::from_le_bytes(data[6..14].try_into().ok()?))
}

pub fn encode_auth_proof(magic: [u8; 4], version: u8, proof: AuthProof) -> Vec<u8> {
    let mut out = Vec::with_capacity(30);
    out.extend_from_slice(&magic);
    out.push(version);
    out.push(PACKET_AUTH_PROOF);
    out.extend_from_slice(&proof.player_id.to_le_bytes());
    out.extend_from_slice(&proof.nonce.to_le_bytes());
    out.extend_from_slice(&proof.digest.to_le_bytes());
    out
}

pub fn decode_auth_proof(magic: [u8; 4], version: u8, data: &[u8]) -> Option<AuthProof> {
    if data.len() != 30 || data[0..4] != magic || data[4] != version || data[5] != PACKET_AUTH_PROOF {
        return None;
    }
    Some(AuthProof {
        player_id: u64::from_le_bytes(data[6..14].try_into().ok()?),
        nonce: u64::from_le_bytes(data[14..22].try_into().ok()?),
        digest: u64::from_le_bytes(data[22..30].try_into().ok()?),
    })
}

pub fn encode_auth_accept(magic: [u8; 4], version: u8, token: SessionToken) -> Vec<u8> {
    let mut out = Vec::with_capacity(22);
    out.extend_from_slice(&magic);
    out.push(version);
    out.push(PACKET_AUTH_ACCEPT);
    out.extend_from_slice(&token.to_le_bytes());
    out
}

pub fn decode_auth_accept(magic: [u8; 4], version: u8, data: &[u8]) -> Option<SessionToken> {
    if data.len() != 22 || data[0..4] != magic || data[4] != version || data[5] != PACKET_AUTH_ACCEPT {
        return None;
    }
    Some(u128::from_le_bytes(data[6..22].try_into().ok()?))
}

pub fn encode_input_payload(move_x: f32, move_z: f32, jump: bool, color: Color) -> Vec<u8> {
    let srgba = color.to_srgba();
    let mut out = Vec::with_capacity(12);
    out.extend_from_slice(&move_x.to_le_bytes());
    out.extend_from_slice(&move_z.to_le_bytes());
    out.push(u8::from(jump));
    out.extend_from_slice(&[
        (srgba.red.clamp(0.0, 1.0) * 255.0) as u8,
        (srgba.green.clamp(0.0, 1.0) * 255.0) as u8,
        (srgba.blue.clamp(0.0, 1.0) * 255.0) as u8,
    ]);
    out
}

pub fn decode_input_payload(payload: &[u8]) -> Option<(f32, f32, bool, Color)> {
    if payload.len() != 12 {
        return None;
    }
    let move_x = f32::from_le_bytes(payload.get(0..4)?.try_into().ok()?);
    let move_z = f32::from_le_bytes(payload.get(4..8)?.try_into().ok()?);
    if !move_x.is_finite() || !move_z.is_finite() {
        return None;
    }
    Some((
        move_x.clamp(-1.0, 1.0),
        move_z.clamp(-1.0, 1.0),
        payload[8] != 0,
        Color::srgb(
            payload[9] as f32 / 255.0,
            payload[10] as f32 / 255.0,
            payload[11] as f32 / 255.0,
        ),
    ))
}

pub fn encode_input_packet(
    magic: [u8; 4],
    version: u8,
    session_token: SessionToken,
    input_sequence: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(30 + payload.len());
    out.extend_from_slice(&magic);
    out.push(version);
    out.push(PACKET_INPUT);
    out.extend_from_slice(&session_token.to_le_bytes());
    out.extend_from_slice(&input_sequence.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn decode_input_packet(
    magic: [u8; 4],
    version: u8,
    data: &[u8],
) -> Option<(SessionToken, u32, Vec<u8>)> {
    if data.len() < 30 || data[0..4] != magic || data[4] != version || data[5] != PACKET_INPUT {
        return None;
    }

    let session_token = u128::from_le_bytes(data[6..22].try_into().ok()?);
    let input_sequence = u32::from_le_bytes(data[22..26].try_into().ok()?);
    let payload_len = u32::from_le_bytes(data[26..30].try_into().ok()?) as usize;
    // Exact length: trailing bytes after the declared payload are a protocol
    // violation, and the addition must not wrap on 32-bit targets before the
    // bounds comparison.
    let expected_len = payload_len.checked_add(30)?;
    if data.len() != expected_len {
        return None;
    }

    Some((session_token, input_sequence, data[30..expected_len].to_vec()))
}

pub fn encode_snapshot_packet(magic: [u8; 4], version: u8, states: &[(u64, Transform, Color)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(10 + states.len() * 24);
    out.extend_from_slice(&magic);
    out.push(version);
    out.push(PACKET_SNAPSHOT);
    out.extend_from_slice(&(states.len() as u32).to_le_bytes());

    for (player_id, transform, color) in states {
        let srgba = color.to_srgba();
        out.extend_from_slice(&player_id.to_le_bytes());
        out.extend_from_slice(&transform.translation.x.to_le_bytes());
        out.extend_from_slice(&transform.translation.y.to_le_bytes());
        out.extend_from_slice(&transform.translation.z.to_le_bytes());
        out.extend_from_slice(&[
            (srgba.red.clamp(0.0, 1.0) * 255.0) as u8,
            (srgba.green.clamp(0.0, 1.0) * 255.0) as u8,
            (srgba.blue.clamp(0.0, 1.0) * 255.0) as u8,
        ]);
        out.push(0); // 1-byte alignment pad to keep each record at 24 bytes
    }

    out
}

pub fn decode_snapshot_packet(magic: [u8; 4], version: u8, data: &[u8]) -> Option<Vec<(u64, Transform, Color)>> {
    if data.len() < 10 || data.len() > MAX_PACKET_BYTES || data[0..4] != magic || data[4] != version || data[5] != PACKET_SNAPSHOT {
        return None;
    }

    const RECORD_SIZE: usize = 24;
    let count = u32::from_le_bytes(data[6..10].try_into().ok()?) as usize;
    // Never pre-allocate for more records than the buffer could possibly
    // contain — a raw wire-controlled `count` near u32::MAX would otherwise
    // trigger a multi-GB allocation attempt before any bounds are checked.
    let max_possible_records = data.len().saturating_sub(10) / RECORD_SIZE;
    if count > max_possible_records || data.len() != 10 + count * RECORD_SIZE {
        return None;
    }
    let mut idx = 10;
    let mut out = Vec::with_capacity(count);

    for _ in 0..count {
        if data.len() < idx + 24 {
            return None;
        }
        let player_id = u64::from_le_bytes(data[idx..idx + 8].try_into().ok()?);
        idx += 8;
        let x = f32::from_le_bytes(data[idx..idx + 4].try_into().ok()?);
        idx += 4;
        let y = f32::from_le_bytes(data[idx..idx + 4].try_into().ok()?);
        idx += 4;
        let z = f32::from_le_bytes(data[idx..idx + 4].try_into().ok()?);
        idx += 4;
        // One non-finite coordinate would NaN-poison local reconciliation
        // permanently (lerp/distance never settle). Reject the whole packet,
        // matching sync_codec::read_finite_f32 and decode_input_payload.
        if !x.is_finite() || !y.is_finite() || !z.is_finite() {
            return None;
        }
        let r = data[idx] as f32 / 255.0;
        let g = data[idx + 1] as f32 / 255.0;
        let b = data[idx + 2] as f32 / 255.0;
        idx += 4;

        out.push((player_id, Transform::from_xyz(x, y, z), Color::srgb(r, g, b)));
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MAGIC: [u8; 4] = *b"TST0";
    const TEST_VERSION: u8 = 1;

    #[test]
    fn auth_hello_roundtrip() {
        let encoded = encode_auth_hello(TEST_MAGIC, TEST_VERSION, 42);
        let decoded = decode_auth_hello(TEST_MAGIC, TEST_VERSION, &encoded);
        assert_eq!(decoded, Some(42));
    }

    #[test]
    fn auth_challenge_roundtrip() {
        let encoded = encode_auth_challenge(TEST_MAGIC, TEST_VERSION, 99);
        let decoded = decode_auth_challenge(TEST_MAGIC, TEST_VERSION, &encoded);
        assert_eq!(decoded, Some(99));
    }

    #[test]
    fn auth_proof_roundtrip() {
        let proof = AuthProof {
            player_id: 7,
            nonce: 77,
            digest: 777,
        };
        let encoded = encode_auth_proof(TEST_MAGIC, TEST_VERSION, proof);
        let decoded = decode_auth_proof(TEST_MAGIC, TEST_VERSION, &encoded);
        assert_eq!(decoded, Some(proof));
    }

    #[test]
    fn auth_accept_roundtrip() {
        let token: SessionToken = 123456789;
        let encoded = encode_auth_accept(TEST_MAGIC, TEST_VERSION, token);
        let decoded = decode_auth_accept(TEST_MAGIC, TEST_VERSION, &encoded);
        assert_eq!(decoded, Some(token));
    }

    #[test]
    fn input_packet_roundtrip() {
        let payload = encode_input_payload(0.5, -0.25, true, Color::srgb(0.2, 0.4, 0.6));
        let encoded = encode_input_packet(TEST_MAGIC, TEST_VERSION, 88, 5, &payload);
        let decoded = decode_input_packet(TEST_MAGIC, TEST_VERSION, &encoded).unwrap();
        assert_eq!(decoded.0, 88);
        assert_eq!(decoded.1, 5);

        let input = decode_input_payload(&decoded.2).unwrap();
        assert!((input.0 - 0.5).abs() < 0.0001);
        assert!((input.1 + 0.25).abs() < 0.0001);
        assert!(input.2);
    }

    #[test]
    fn snapshot_roundtrip() {
        let states = vec![
            (1, Transform::from_xyz(1.0, 2.0, 3.0), Color::srgb(0.1, 0.2, 0.3)),
            (2, Transform::from_xyz(-1.0, 0.5, 4.0), Color::srgb(0.8, 0.4, 0.2)),
        ];

        let encoded = encode_snapshot_packet(TEST_MAGIC, TEST_VERSION, &states);
        let decoded = decode_snapshot_packet(TEST_MAGIC, TEST_VERSION, &encoded).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].0, 1);
        assert_eq!(decoded[1].0, 2);
        assert!((decoded[0].1.translation.x - 1.0).abs() < 0.0001);
        assert!((decoded[1].1.translation.z - 4.0).abs() < 0.0001);
    }

    #[test]
    fn input_packet_rejects_trailing_bytes_and_huge_lengths() {
        let payload = encode_input_payload(0.5, -0.25, true, Color::srgb(0.2, 0.4, 0.6));
        let mut encoded = encode_input_packet(TEST_MAGIC, TEST_VERSION, 88, 5, &payload);
        // Control: the exact packet decodes.
        assert!(decode_input_packet(TEST_MAGIC, TEST_VERSION, &encoded).is_some());
        // Trailing bytes after the declared payload are rejected.
        encoded.push(0);
        assert_eq!(decode_input_packet(TEST_MAGIC, TEST_VERSION, &encoded), None);
        // A length field near u32::MAX must not wrap the bounds check.
        let mut crafted = encode_input_packet(TEST_MAGIC, TEST_VERSION, 88, 5, &payload);
        crafted[26..30].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode_input_packet(TEST_MAGIC, TEST_VERSION, &crafted), None);
    }

    #[test]
    fn snapshot_decode_rejects_nonfinite_coordinates() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for states in [
                vec![(1, Transform::from_xyz(bad, 0.0, 0.0), Color::WHITE)],
                vec![(1, Transform::from_xyz(0.0, bad, 0.0), Color::WHITE)],
                vec![(1, Transform::from_xyz(0.0, 0.0, bad), Color::WHITE)],
                // One bad record must sink the whole packet.
                vec![
                    (1, Transform::from_xyz(1.0, 2.0, 3.0), Color::WHITE),
                    (2, Transform::from_xyz(bad, 0.0, 0.0), Color::WHITE),
                ],
            ] {
                let encoded = encode_snapshot_packet(TEST_MAGIC, TEST_VERSION, &states);
                assert_eq!(decode_snapshot_packet(TEST_MAGIC, TEST_VERSION, &encoded), None);
            }
        }
        // Control: finite coordinates still decode.
        let ok = vec![(1, Transform::from_xyz(1.0, 2.0, 3.0), Color::WHITE)];
        let encoded = encode_snapshot_packet(TEST_MAGIC, TEST_VERSION, &ok);
        assert_eq!(
            decode_snapshot_packet(TEST_MAGIC, TEST_VERSION, &encoded).unwrap().len(),
            1
        );
    }

    #[test]
    fn snapshot_decode_rejects_oversized_count_without_allocating() {
        // A crafted `count` near u32::MAX in a tiny buffer must be rejected
        // before any large Vec::with_capacity allocation is attempted.
        let mut data = Vec::new();
        data.extend_from_slice(&TEST_MAGIC);
        data.push(TEST_VERSION);
        data.push(PACKET_SNAPSHOT);
        data.extend_from_slice(&u32::MAX.to_le_bytes()); // malicious count
        // No record bytes follow — buffer is far too small for u32::MAX records.

        let decoded = decode_snapshot_packet(TEST_MAGIC, TEST_VERSION, &data);
        assert_eq!(decoded, None);
    }
}
