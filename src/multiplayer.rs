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
    remote_states: HashMap<u64, RemoteState>,
    spawned_entities: HashMap<u64, Entity>,
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
        remote_states: HashMap::new(),
        spawned_entities: HashMap::new(),
    });
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

    let payload = encode_state_packet(local_player.value, transform);
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
                if let Some((player_id, transform)) = decode_state_packet(&buf[..len]) {
                    if player_id == local_player.value {
                        continue;
                    }
                    network.remote_states.insert(
                        player_id,
                        RemoteState {
                            transform,
                            last_seen: Instant::now(),
                        },
                    );
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
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: Color::srgb(0.2, 0.45, 0.9),
                        metallic: 0.1,
                        perceptual_roughness: 0.55,
                        ..default()
                    })),
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

fn encode_state_packet(player_id: u64, transform: &Transform) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 8 + (3 + 4) * 4);
    out.extend_from_slice(&SYNC_MAGIC);
    out.push(SYNC_VERSION);
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

fn decode_state_packet(data: &[u8]) -> Option<(u64, Transform)> {
    const SIZE: usize = 4 + 1 + 8 + (3 + 4) * 4;
    if data.len() != SIZE {
        return None;
    }
    if data[0..4] != SYNC_MAGIC || data[4] != SYNC_VERSION {
        return None;
    }

    let mut idx = 5;
    let player_id = u64::from_le_bytes(data[idx..idx + 8].try_into().ok()?);
    idx += 8;

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

    Some((player_id, transform))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_packet_roundtrip() {
        let mut transform = Transform::from_xyz(1.25, 2.5, -3.0);
        transform.rotation = Quat::from_xyzw(0.1, 0.2, 0.3, 0.9);

        let packet = encode_state_packet(42, &transform);
        let parsed = decode_state_packet(&packet).unwrap();

        assert_eq!(parsed.0, 42);
        assert_eq!(parsed.1.translation, transform.translation);
        assert_eq!(parsed.1.rotation, transform.rotation);
    }
}
