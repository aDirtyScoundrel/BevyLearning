//! Lightweight LAN multiplayer cube sync.
//!
//! This is a transitional sync layer so multiple machines can drive independent
//! cubes before full server-authoritative netcode is implemented.

use bevy::math::primitives::Cuboid;
use bevy::mesh::Mesh3d;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use std::collections::HashMap;
use std::env;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SYNC_MAGIC: [u8; 4] = *b"CUBE";
const SYNC_VERSION: u8 = 1;
const PACKET_STATE: u8 = 1;
const PACKET_JOIN: u8 = 2;
const PACKET_LEAVE: u8 = 3;
const SYNC_PORT: u16 = 34567;
const SEND_INTERVAL: Duration = Duration::from_millis(50);
const REMOTE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Resource, Debug, Clone, Copy)]
pub struct LocalPlayerId {
    pub value: u64,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct RemoteCube {
    pub player_id: u64,
}

#[derive(Debug, Clone)]
struct RemoteState {
    transform: Transform,
    last_seen: Instant,
}

#[derive(Resource)]
pub struct NetworkSync {
    socket: UdpSocket,
    target_addr: SocketAddr,
    last_send: Instant,
    announced_presence: bool,
    remote_states: HashMap<u64, RemoteState>,
    spawned_entities: HashMap<u64, Entity>,
    departed_players: Vec<u64>,
}

pub fn generate_local_player_id() -> LocalPlayerId {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    LocalPlayerId {
        value: nanos ^ (pid << 32),
    }
}

pub fn setup_network(mut commands: Commands) {
    let bind_addr = format!("0.0.0.0:{}", SYNC_PORT);
    let socket = match UdpSocket::bind(&bind_addr) {
        Ok(socket) => socket,
        Err(err) => {
            eprintln!("[multiplayer] failed to bind {}: {}", bind_addr, err);
            return;
        }
    };

    if let Err(err) = socket.set_nonblocking(true) {
        eprintln!("[multiplayer] failed to set nonblocking: {}", err);
        return;
    }

    if let Err(err) = socket.set_broadcast(true) {
        eprintln!("[multiplayer] failed to enable broadcast: {}", err);
        return;
    }

    let target_addr = env::var("CUBE_SYNC_TARGET")
        .ok()
        .and_then(|s| s.parse::<SocketAddr>().ok())
        .unwrap_or_else(|| SocketAddr::from(([255, 255, 255, 255], SYNC_PORT)));

    println!(
        "[multiplayer] listening on {} and broadcasting to {}",
        bind_addr, target_addr
    );

    commands.insert_resource(NetworkSync {
        socket,
        target_addr,
        last_send: Instant::now(),
        announced_presence: false,
        remote_states: HashMap::new(),
        spawned_entities: HashMap::new(),
        departed_players: Vec::new(),
    });
}

pub fn announce_local_presence(
    local_player: Res<LocalPlayerId>,
    local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
    mut network: Option<ResMut<NetworkSync>>,
) {
    let Some(network) = network.as_deref_mut() else {
        return;
    };

    if network.announced_presence {
        return;
    }

    let Ok(transform) = local_cube_query.single() else {
        return;
    };

    let payload = encode_state_packet(PACKET_JOIN, local_player.value, transform);
    let _ = network.socket.send_to(&payload, network.target_addr);
    network.announced_presence = true;
}

pub fn send_local_leave(
    exit_requested: Res<crate::ExitRequested>,
    local_player: Res<LocalPlayerId>,
    local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
    mut network: Option<ResMut<NetworkSync>>,
) {
    let Some(network) = network.as_deref_mut() else {
        return;
    };

    if !exit_requested.0 {
        return;
    }

    let Ok(transform) = local_cube_query.single() else {
        return;
    };

    let payload = encode_state_packet(PACKET_LEAVE, local_player.value, transform);
    let _ = network.socket.send_to(&payload, network.target_addr);
}

pub fn send_local_state(
    local_player: Res<LocalPlayerId>,
    local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
    mut network: Option<ResMut<NetworkSync>>,
) {
    let Some(network) = network.as_deref_mut() else {
        return;
    };

    if network.last_send.elapsed() < SEND_INTERVAL {
        return;
    }

    let Ok(transform) = local_cube_query.single() else {
        return;
    };

    let payload = encode_state_packet(PACKET_STATE, local_player.value, transform);
    let _ = network.socket.send_to(&payload, network.target_addr);
    network.last_send = Instant::now();
}

pub fn receive_remote_states(
    local_player: Res<LocalPlayerId>,
    mut network: Option<ResMut<NetworkSync>>,
) {
    let Some(network) = network.as_deref_mut() else {
        return;
    };

    loop {
        let mut buf = [0u8; 96];
        match network.socket.recv_from(&mut buf) {
            Ok((len, _from)) => {
                if let Some((packet_type, player_id, transform)) = decode_state_packet(&buf[..len]) {
                    if player_id == local_player.value {
                        continue;
                    }

                    match packet_type {
                        PACKET_STATE | PACKET_JOIN => {
                            if let Some(transform) = transform {
                                network.remote_states.insert(
                                    player_id,
                                    RemoteState {
                                        transform,
                                        last_seen: Instant::now(),
                                    },
                                );
                            }
                        }
                        PACKET_LEAVE => {
                            network.departed_players.push(player_id);
                        }
                        _ => {}
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
}

pub fn sync_remote_cubes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cube_query: Query<(Entity, &mut Transform, &RemoteCube)>,
    mut network: Option<ResMut<NetworkSync>>,
) {
    let Some(network) = network.as_deref_mut() else {
        return;
    };

    for player_id in network.departed_players.drain(..) {
        network.remote_states.remove(&player_id);
        if let Some(entity) = network.spawned_entities.remove(&player_id) {
            commands.entity(entity).despawn();
        }
    }

    let now = Instant::now();
    network
        .remote_states
        .retain(|_, state| now.duration_since(state.last_seen) <= REMOTE_TIMEOUT);

    for (player_id, state) in &network.remote_states {
        if let Some(entity) = network.spawned_entities.get(player_id).copied() {
            if let Ok((_entity, mut transform, remote)) = cube_query.get_mut(entity) {
                let _ = remote.player_id;
                *transform = state.transform;
            } else {
                network.spawned_entities.remove(player_id);
            }
        } else {
            let entity = commands
                .spawn((
                    Mesh3d(meshes.add(Cuboid::new(1.5, 1.5, 1.5).mesh().build())),
                    MeshMaterial3d(materials.add(player_material(*player_id))),
                    state.transform,
                    GlobalTransform::default(),
                    RemoteCube {
                        player_id: *player_id,
                    },
                ))
                .id();
            network.spawned_entities.insert(*player_id, entity);
        }
    }

    let active_ids: std::collections::HashSet<u64> = network.remote_states.keys().copied().collect();
    let stale_ids: Vec<u64> = network
        .spawned_entities
        .keys()
        .filter(|id| !active_ids.contains(id))
        .copied()
        .collect();

    for player_id in stale_ids {
        if let Some(entity) = network.spawned_entities.remove(&player_id) {
            commands.entity(entity).despawn();
        }
    }
}

fn player_material(player_id: u64) -> StandardMaterial {
    let color = player_color(player_id);
    StandardMaterial {
        base_color: color,
        metallic: 0.1,
        perceptual_roughness: 0.55,
        ..default()
    }
}

fn player_color(player_id: u64) -> Color {
    let mut hash = player_id.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    hash ^= hash >> 32;
    hash = hash.wrapping_mul(0xD6E8_F1C1_9C47_9C9D);

    let red = 0.35 + ((hash & 0xff) as f32 / 255.0) * 0.55;
    let green = 0.35 + (((hash >> 8) & 0xff) as f32 / 255.0) * 0.55;
    let blue = 0.35 + (((hash >> 16) & 0xff) as f32 / 255.0) * 0.55;

    Color::srgb(red, green, blue)
}

fn encode_state_packet(packet_type: u8, player_id: u64, transform: &Transform) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 1 + 8 + (3 + 4) * 4);
    out.extend_from_slice(&SYNC_MAGIC);
    out.push(SYNC_VERSION);
    out.push(packet_type);
    out.extend_from_slice(&player_id.to_le_bytes());

    out.extend_from_slice(&transform.translation.x.to_le_bytes());
    out.extend_from_slice(&transform.translation.y.to_le_bytes());
    out.extend_from_slice(&transform.translation.z.to_le_bytes());

    out.extend_from_slice(&transform.rotation.x.to_le_bytes());
    out.extend_from_slice(&transform.rotation.y.to_le_bytes());
    out.extend_from_slice(&transform.rotation.z.to_le_bytes());
    out.extend_from_slice(&transform.rotation.w.to_le_bytes());

    out
}

fn decode_state_packet(data: &[u8]) -> Option<(u8, u64, Option<Transform>)> {
    if data.len() < 4 + 1 + 1 + 8 {
        return None;
    }
    if data[0..4] != SYNC_MAGIC || data[4] != SYNC_VERSION {
        return None;
    }

    let packet_type = data[5];
    let mut idx = 6;
    let player_id = u64::from_le_bytes(data[idx..idx + 8].try_into().ok()?);
    idx += 8;

    if packet_type == PACKET_LEAVE {
        return Some((packet_type, player_id, None));
    }

    if data.len() != idx + (3 + 4) * 4 {
        return None;
    }

    let read_f32 = |slice: &[u8]| -> Option<f32> {
        Some(f32::from_le_bytes(slice.try_into().ok()?))
    };

    let tx = read_f32(&data[idx..idx + 4])?;
    idx += 4;
    let ty = read_f32(&data[idx..idx + 4])?;
    idx += 4;
    let tz = read_f32(&data[idx..idx + 4])?;
    idx += 4;

    let rx = read_f32(&data[idx..idx + 4])?;
    idx += 4;
    let ry = read_f32(&data[idx..idx + 4])?;
    idx += 4;
    let rz = read_f32(&data[idx..idx + 4])?;
    idx += 4;
    let rw = read_f32(&data[idx..idx + 4])?;

    let mut transform = Transform::from_xyz(tx, ty, tz);
    transform.rotation = Quat::from_xyzw(rx, ry, rz, rw);

    Some((packet_type, player_id, Some(transform)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_packet_roundtrip() {
        let mut transform = Transform::from_xyz(1.25, 2.5, -3.0);
        transform.rotation = Quat::from_xyzw(0.1, 0.2, 0.3, 0.9);

        let packet = encode_state_packet(PACKET_STATE, 42, &transform);
        let parsed = decode_state_packet(&packet).unwrap();

        assert_eq!(parsed.0, PACKET_STATE);
        assert_eq!(parsed.1, 42);
        assert_eq!(parsed.2.unwrap().translation, transform.translation);
        assert_eq!(parsed.2.unwrap().rotation, transform.rotation);
    }

    #[test]
    fn test_leave_packet_roundtrip() {
        let transform = Transform::from_xyz(0.0, 0.0, 0.0);

        let packet = encode_state_packet(PACKET_LEAVE, 77, &transform);
        let parsed = decode_state_packet(&packet).unwrap();

        assert_eq!(parsed.0, PACKET_LEAVE);
        assert_eq!(parsed.1, 77);
        assert!(parsed.2.is_none());
    }
}
