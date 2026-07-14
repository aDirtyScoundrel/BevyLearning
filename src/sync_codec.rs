//! Binary codec for LAN and Steam P2P transport packets.
//!
//! Packets are framed with a 4-byte magic tag, a 1-byte version, and a 1-byte
//! type discriminator.  All numeric fields are little-endian.
//!
//! The `leave_packet_type` parameter in the state encode/decode functions lets
//! callers share this codec across transports that use different discriminator
//! values: when `packet_type == leave_packet_type` the position/rotation/color
//! body is omitted so the receiver can detect a departure without position data.

use bevy::prelude::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A projectile replication snapshot decoded from the wire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileSyncData {
    pub projectile_id: u32,
    pub position: Vec3,
    pub velocity: Vec3,
    pub lifetime_secs: f32,
}

/// Read 4 raw bytes as a little-endian f32; returns `None` on slice-length mismatch.
fn read_f32(slice: &[u8]) -> Option<f32> {
    Some(f32::from_le_bytes(slice.try_into().ok()?))
}

/// Like [`read_f32`] but also rejects NaN and infinity.
fn read_finite_f32(slice: &[u8]) -> Option<f32> {
    let value = read_f32(slice)?;
    value.is_finite().then_some(value)
}

pub fn encode_state_packet(
    magic: [u8; 4],
    version: u8,
    packet_type: u8,
    leave_packet_type: u8,
    player_id: u64,
    transform: &Transform,
    color: Color,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 1 + 8 + (3 + 4 + 3) * 4);
    out.extend_from_slice(&magic);
    out.push(version);
    out.push(packet_type);
    out.extend_from_slice(&player_id.to_le_bytes());

    if packet_type == leave_packet_type {
        return out;
    }

    out.extend_from_slice(&transform.translation.x.to_le_bytes());
    out.extend_from_slice(&transform.translation.y.to_le_bytes());
    out.extend_from_slice(&transform.translation.z.to_le_bytes());

    out.extend_from_slice(&transform.rotation.x.to_le_bytes());
    out.extend_from_slice(&transform.rotation.y.to_le_bytes());
    out.extend_from_slice(&transform.rotation.z.to_le_bytes());
    out.extend_from_slice(&transform.rotation.w.to_le_bytes());

    let srgba = color.to_srgba();
    out.extend_from_slice(&srgba.red.to_le_bytes());
    out.extend_from_slice(&srgba.green.to_le_bytes());
    out.extend_from_slice(&srgba.blue.to_le_bytes());

    out
}

pub fn encode_freeze_packet(
    magic: [u8; 4],
    version: u8,
    packet_type: u8,
    sender_id: u64,
    target_id: u64,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 1 + 8 + 8);
    out.extend_from_slice(&magic);
    out.push(version);
    out.push(packet_type);
    out.extend_from_slice(&sender_id.to_le_bytes());
    out.extend_from_slice(&target_id.to_le_bytes());
    out
}

pub fn encode_projectile_packet(
    magic: [u8; 4],
    version: u8,
    packet_type: u8,
    sender_id: u64,
    projectile_id: u32,
    position: Vec3,
    velocity: Vec3,
    lifetime_secs: f32,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 1 + 8 + 4 + 7 * 4);
    out.extend_from_slice(&magic);
    out.push(version);
    out.push(packet_type);
    out.extend_from_slice(&sender_id.to_le_bytes());
    out.extend_from_slice(&projectile_id.to_le_bytes());
    out.extend_from_slice(&position.x.to_le_bytes());
    out.extend_from_slice(&position.y.to_le_bytes());
    out.extend_from_slice(&position.z.to_le_bytes());
    out.extend_from_slice(&velocity.x.to_le_bytes());
    out.extend_from_slice(&velocity.y.to_le_bytes());
    out.extend_from_slice(&velocity.z.to_le_bytes());
    out.extend_from_slice(&lifetime_secs.to_le_bytes());
    out
}

pub fn decode_state_packet(
    magic: [u8; 4],
    version: u8,
    leave_packet_type: u8,
    data: &[u8],
) -> Option<(u8, u64, Option<Transform>, Option<Color>)> {
    if data.len() < 4 + 1 + 1 + 8 {
        return None;
    }
    if data[0..4] != magic || data[4] != version {
        return None;
    }

    let packet_type = data[5];
    let mut idx = 6;
    let player_id = u64::from_le_bytes(data[idx..idx + 8].try_into().ok()?);
    idx += 8;

    if packet_type == leave_packet_type {
        if data.len() != idx {
            return None;
        }
        return Some((packet_type, player_id, None, None));
    }

    if data.len() != idx + (3 + 4 + 3) * 4 {
        return None;
    }

    let tx = read_finite_f32(&data[idx..idx + 4])?;
    idx += 4;
    let ty = read_finite_f32(&data[idx..idx + 4])?;
    idx += 4;
    let tz = read_finite_f32(&data[idx..idx + 4])?;
    idx += 4;

    let rx = read_finite_f32(&data[idx..idx + 4])?;
    idx += 4;
    let ry = read_finite_f32(&data[idx..idx + 4])?;
    idx += 4;
    let rz = read_finite_f32(&data[idx..idx + 4])?;
    idx += 4;
    let rw = read_finite_f32(&data[idx..idx + 4])?;
    idx += 4;

    let red = read_finite_f32(&data[idx..idx + 4])?.clamp(0.0, 1.0);
    idx += 4;
    let green = read_finite_f32(&data[idx..idx + 4])?.clamp(0.0, 1.0);
    idx += 4;
    let blue = read_finite_f32(&data[idx..idx + 4])?.clamp(0.0, 1.0);

    let rotation_length_sq = rx * rx + ry * ry + rz * rz + rw * rw;
    if !rotation_length_sq.is_finite() || rotation_length_sq <= f32::EPSILON {
        return None;
    }

    let rotation = Quat::from_xyzw(rx, ry, rz, rw).normalize();
    if !rotation.is_finite() {
        return None;
    }

    let mut transform = Transform::from_xyz(tx, ty, tz);
    transform.rotation = rotation;

    Some((packet_type, player_id, Some(transform), Some(Color::srgb(red, green, blue))))
}

pub fn decode_freeze_packet(
    magic: [u8; 4],
    version: u8,
    packet_type: u8,
    data: &[u8],
) -> Option<(u64, u64)> {
    if data.len() != 4 + 1 + 1 + 8 + 8 {
        return None;
    }
    if data[0..4] != magic || data[4] != version || data[5] != packet_type {
        return None;
    }

    let sender_id = u64::from_le_bytes(data[6..14].try_into().ok()?);
    let target_id = u64::from_le_bytes(data[14..22].try_into().ok()?);
    Some((sender_id, target_id))
}

pub fn decode_projectile_packet(
    magic: [u8; 4],
    version: u8,
    packet_type: u8,
    data: &[u8],
) -> Option<(u64, ProjectileSyncData)> {
    if data.len() != 4 + 1 + 1 + 8 + 4 + 7 * 4 {
        return None;
    }
    if data[0..4] != magic || data[4] != version || data[5] != packet_type {
        return None;
    }

    let mut idx = 6;
    let player_id = u64::from_le_bytes(data[idx..idx + 8].try_into().ok()?);
    idx += 8;

    let projectile_id = u32::from_le_bytes(data[idx..idx + 4].try_into().ok()?);
    idx += 4;

    let position = Vec3::new(
        read_finite_f32(&data[idx..idx + 4])?,
        read_finite_f32(&data[idx + 4..idx + 8])?,
        read_finite_f32(&data[idx + 8..idx + 12])?,
    );
    idx += 12;

    let velocity = Vec3::new(
        read_finite_f32(&data[idx..idx + 4])?,
        read_finite_f32(&data[idx + 4..idx + 8])?,
        read_finite_f32(&data[idx + 8..idx + 12])?,
    );
    idx += 12;

    let lifetime_secs = read_finite_f32(&data[idx..idx + 4])?;

    Some((
        player_id,
        ProjectileSyncData {
            projectile_id,
            position,
            velocity,
            lifetime_secs,
        },
    ))
}

/// Insert `(player_id, projectile_id)` into `seen_projectiles` and return `true`
/// if this is the first time the pair has been seen within `ttl`.
///
/// Stale entries older than `ttl` are pruned on every call.
pub fn accept_recent_projectile(
    seen_projectiles: &mut HashMap<(u64, u32), Instant>,
    player_id: u64,
    projectile_id: u32,
    now: Instant,
    ttl: Duration,
) -> bool {
    seen_projectiles.retain(|_, seen_at| now.duration_since(*seen_at) <= ttl);

    let key = (player_id, projectile_id);
    match seen_projectiles.get(&key) {
        Some(seen_at) if now.duration_since(*seen_at) <= ttl => false,
        _ => {
            seen_projectiles.insert(key, now);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MAGIC: [u8; 4] = *b"TST0";
    const TEST_VERSION: u8 = 1;

    #[test]
    fn test_state_packet_roundtrip() {
        let mut transform = Transform::from_xyz(1.25, 2.5, -3.0);
        transform.rotation = Quat::from_xyzw(0.1, 0.2, 0.3, 0.9).normalize();
        let color = Color::srgb(0.25, 0.5, 0.75);

        let packet = encode_state_packet(TEST_MAGIC, TEST_VERSION, 1, 3, 42, &transform, color);
        let parsed = decode_state_packet(TEST_MAGIC, TEST_VERSION, 3, &packet).unwrap();
        let decoded_rotation = parsed.2.unwrap().rotation;
        let expected_rotation = transform.rotation;

        assert_eq!(parsed.0, 1);
        assert_eq!(parsed.1, 42);
        assert_eq!(parsed.2.unwrap().translation, transform.translation);
        assert!((decoded_rotation.x - expected_rotation.x).abs() < 0.00001);
        assert!((decoded_rotation.y - expected_rotation.y).abs() < 0.00001);
        assert!((decoded_rotation.z - expected_rotation.z).abs() < 0.00001);
        assert!((decoded_rotation.w - expected_rotation.w).abs() < 0.00001);
        assert_eq!(parsed.3.unwrap().to_srgba().red, color.to_srgba().red);
    }

    #[test]
    fn test_state_packet_rejects_nonfinite_values() {
        let transform = Transform::from_xyz(0.0, 0.0, 0.0);
        let color = Color::srgb(0.1, 0.2, 0.3);
        let mut packet = encode_state_packet(TEST_MAGIC, TEST_VERSION, 1, 3, 7, &transform, color);

        let nan = f32::NAN.to_le_bytes();
        let rotation_x_offset = 6 + 8 + 12;
        packet[rotation_x_offset..rotation_x_offset + 4].copy_from_slice(&nan);

        assert!(decode_state_packet(TEST_MAGIC, TEST_VERSION, 3, &packet).is_none());
    }

    #[test]
    fn test_state_packet_normalizes_malformed_finite_rotation() {
        let transform = Transform::from_xyz(0.0, 0.0, 0.0);
        let color = Color::srgb(0.1, 0.2, 0.3);
        let mut packet = encode_state_packet(TEST_MAGIC, TEST_VERSION, 1, 3, 7, &transform, color);

        let rotation_x_offset = 6 + 8 + 12;
        packet[rotation_x_offset..rotation_x_offset + 4].copy_from_slice(&2.0f32.to_le_bytes());
        packet[rotation_x_offset + 4..rotation_x_offset + 8].copy_from_slice(&0.0f32.to_le_bytes());
        packet[rotation_x_offset + 8..rotation_x_offset + 12].copy_from_slice(&0.0f32.to_le_bytes());
        packet[rotation_x_offset + 12..rotation_x_offset + 16].copy_from_slice(&0.0f32.to_le_bytes());

        let parsed = decode_state_packet(TEST_MAGIC, TEST_VERSION, 3, &packet).unwrap();
        assert!((parsed.2.unwrap().rotation.length_squared() - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_state_packet_leave_contract_rejects_trailing_payload() {
        let transform = Transform::from_xyz(0.0, 0.0, 0.0);
        let mut packet = encode_state_packet(
            TEST_MAGIC,
            TEST_VERSION,
            3,
            3,
            99,
            &transform,
            Color::WHITE,
        );

        packet.extend_from_slice(&[0u8; 4]);
        assert!(decode_state_packet(TEST_MAGIC, TEST_VERSION, 3, &packet).is_none());
    }

    #[test]
    fn test_state_packet_clamps_color_components() {
        let transform = Transform::from_xyz(0.0, 0.0, 0.0);
        let mut packet = encode_state_packet(
            TEST_MAGIC,
            TEST_VERSION,
            1,
            3,
            77,
            &transform,
            Color::WHITE,
        );

        let color_offset = 6 + 8 + 12 + 16;
        packet[color_offset..color_offset + 4].copy_from_slice(&(-1.0f32).to_le_bytes());
        packet[color_offset + 4..color_offset + 8].copy_from_slice(&(0.5f32).to_le_bytes());
        packet[color_offset + 8..color_offset + 12].copy_from_slice(&(2.0f32).to_le_bytes());

        let parsed = decode_state_packet(TEST_MAGIC, TEST_VERSION, 3, &packet).unwrap();
        let color = parsed.3.unwrap().to_srgba();
        assert_eq!(color.red, 0.0);
        assert_eq!(color.green, 0.5);
        assert_eq!(color.blue, 1.0);
    }

    #[test]
    fn test_freeze_packet_roundtrip() {
        let packet = encode_freeze_packet(TEST_MAGIC, TEST_VERSION, 4, 11, 27);
        let parsed = decode_freeze_packet(TEST_MAGIC, TEST_VERSION, 4, &packet).unwrap();

        assert_eq!(parsed, (11, 27));
    }

    #[test]
    fn test_freeze_packet_contract_matrix() {
        let packet = encode_freeze_packet(TEST_MAGIC, TEST_VERSION, 4, 11, 27);

        let mut bad_magic = packet.clone();
        bad_magic[0] = b'X';
        let mut bad_version = packet.clone();
        bad_version[4] = TEST_VERSION + 1;
        let mut bad_type = packet.clone();
        bad_type[5] = 99;
        let short = &packet[..packet.len() - 1];

        let cases: [(&[u8], &str); 4] = [
            (&bad_magic, "magic"),
            (&bad_version, "version"),
            (&bad_type, "type"),
            (short, "len"),
        ];

        for (payload, label) in cases {
            assert!(
                decode_freeze_packet(TEST_MAGIC, TEST_VERSION, 4, payload).is_none(),
                "freeze decode should reject malformed {label}"
            );
        }
    }

    #[test]
    fn test_projectile_packet_roundtrip() {
        let packet = encode_projectile_packet(
            TEST_MAGIC,
            TEST_VERSION,
            5,
            99,
            1234,
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(-4.0, -5.0, -6.0),
            1.5,
        );

        let parsed = decode_projectile_packet(TEST_MAGIC, TEST_VERSION, 5, &packet).unwrap();
        assert_eq!(parsed.0, 99);
        assert_eq!(parsed.1.projectile_id, 1234);
        assert_eq!(parsed.1.position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(parsed.1.velocity, Vec3::new(-4.0, -5.0, -6.0));
        assert_eq!(parsed.1.lifetime_secs, 1.5);
    }

    #[test]
    fn test_projectile_packet_contract_matrix() {
        let packet = encode_projectile_packet(
            TEST_MAGIC,
            TEST_VERSION,
            5,
            99,
            1234,
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(-4.0, -5.0, -6.0),
            1.5,
        );

        let mut bad_magic = packet.clone();
        bad_magic[0] = b'X';
        let mut bad_version = packet.clone();
        bad_version[4] = TEST_VERSION + 1;
        let mut bad_type = packet.clone();
        bad_type[5] = 99;
        let short = &packet[..packet.len() - 1];

        let mut non_finite = packet.clone();
        let position_x_offset = 6 + 8 + 4;
        non_finite[position_x_offset..position_x_offset + 4].copy_from_slice(&f32::NAN.to_le_bytes());

        let cases: [(&[u8], &str); 5] = [
            (&bad_magic, "magic"),
            (&bad_version, "version"),
            (&bad_type, "type"),
            (short, "len"),
            (&non_finite, "non-finite"),
        ];

        for (payload, label) in cases {
            assert!(
                decode_projectile_packet(TEST_MAGIC, TEST_VERSION, 5, payload).is_none(),
                "projectile decode should reject malformed {label}"
            );
        }
    }

    #[test]
    fn test_projectile_dedup_prunes_expired_entries() {
        let mut seen_projectiles = HashMap::new();
        let now = Instant::now();
        let ttl = Duration::from_secs(1);

        assert!(accept_recent_projectile(&mut seen_projectiles, 1, 7, now, ttl));
        assert!(!accept_recent_projectile(&mut seen_projectiles, 1, 7, now, ttl));
        assert!(accept_recent_projectile(
            &mut seen_projectiles,
            1,
            7,
            now + Duration::from_secs(2),
            ttl,
        ));
    }
}