use bevy::prelude::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileSyncData {
    pub projectile_id: u32,
    pub position: Vec3,
    pub velocity: Vec3,
    pub lifetime_secs: f32,
}

fn read_f32(slice: &[u8]) -> Option<f32> {
    Some(f32::from_le_bytes(slice.try_into().ok()?))
}

fn read_finite_f32(slice: &[u8]) -> Option<f32> {
    let value = read_f32(slice)?;
    value.is_finite().then_some(value)
}

pub fn encode_state_packet(
    magic: [u8; 4],
    version: u8,
    packet_type: u8,
    player_id: u64,
    transform: &Transform,
    color: Color,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 1 + 8 + (3 + 4 + 3) * 4);
    out.extend_from_slice(&magic);
    out.push(version);
    out.push(packet_type);
    out.extend_from_slice(&player_id.to_le_bytes());

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

        let packet = encode_state_packet(TEST_MAGIC, TEST_VERSION, 1, 42, &transform, color);
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
        let mut packet = encode_state_packet(TEST_MAGIC, TEST_VERSION, 1, 7, &transform, color);

        let nan = f32::NAN.to_le_bytes();
        let rotation_x_offset = 6 + 8 + 12;
        packet[rotation_x_offset..rotation_x_offset + 4].copy_from_slice(&nan);

        assert!(decode_state_packet(TEST_MAGIC, TEST_VERSION, 3, &packet).is_none());
    }

    #[test]
    fn test_state_packet_normalizes_malformed_finite_rotation() {
        let transform = Transform::from_xyz(0.0, 0.0, 0.0);
        let color = Color::srgb(0.1, 0.2, 0.3);
        let mut packet = encode_state_packet(TEST_MAGIC, TEST_VERSION, 1, 7, &transform, color);

        let rotation_x_offset = 6 + 8 + 12;
        packet[rotation_x_offset..rotation_x_offset + 4].copy_from_slice(&2.0f32.to_le_bytes());
        packet[rotation_x_offset + 4..rotation_x_offset + 8].copy_from_slice(&0.0f32.to_le_bytes());
        packet[rotation_x_offset + 8..rotation_x_offset + 12].copy_from_slice(&0.0f32.to_le_bytes());
        packet[rotation_x_offset + 12..rotation_x_offset + 16].copy_from_slice(&0.0f32.to_le_bytes());

        let parsed = decode_state_packet(TEST_MAGIC, TEST_VERSION, 3, &packet).unwrap();
        assert!((parsed.2.unwrap().rotation.length_squared() - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_freeze_packet_roundtrip() {
        let packet = encode_freeze_packet(TEST_MAGIC, TEST_VERSION, 4, 11, 27);
        let parsed = decode_freeze_packet(TEST_MAGIC, TEST_VERSION, 4, &packet).unwrap();

        assert_eq!(parsed, (11, 27));
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