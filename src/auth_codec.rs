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
    if payload.len() < 12 {
        return None;
    }

    Some((
        f32::from_le_bytes(payload[0..4].try_into().ok()?),
        f32::from_le_bytes(payload[4..8].try_into().ok()?),
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
    if data.len() < 30 + payload_len {
        return None;
    }

    Some((session_token, input_sequence, data[30..30 + payload_len].to_vec()))
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
    if data.len() < 10 || data[0..4] != magic || data[4] != version || data[5] != PACKET_SNAPSHOT {
        return None;
    }

    let count = u32::from_le_bytes(data[6..10].try_into().ok()?) as usize;
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
}
