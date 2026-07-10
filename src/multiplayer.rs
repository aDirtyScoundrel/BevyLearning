//! Lightweight LAN multiplayer cube sync.
//!
//! This is a transitional sync layer so multiple machines can drive independent
//! cubes before full server-authoritative netcode is implemented.

use bevy::math::primitives::Sphere;
use bevy::mesh::Mesh3d;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
use std::env;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SYNC_MAGIC: [u8; 4] = *b"CUBE";
const SYNC_VERSION: u8 = 1;
const PACKET_STATE: u8 = 1;
const PACKET_JOIN: u8 = 2;
const PACKET_LEAVE: u8 = 3;
const PACKET_FREEZE: u8 = 4;
const PACKET_PROJECTILE: u8 = 5;
const SYNC_PORT: u16 = 34567;
const SEND_INTERVAL: Duration = Duration::from_millis(50);
const REMOTE_TIMEOUT: Duration = Duration::from_secs(5);
const FREEZE_DURATION_SECS: f32 = 2.0;

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
    color: Color,
    last_seen: Instant,
}

#[derive(Debug, Clone, Copy)]
struct RemoteProjectileSpawn {
    spawn: crate::scene::ProjectileSpawnData,
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
    pending_freezes: Vec<u64>,
    pending_projectiles: Vec<RemoteProjectileSpawn>,
    seen_projectiles: HashSet<(u64, u32)>,
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
        pending_freezes: Vec::new(),
        pending_projectiles: Vec::new(),
        seen_projectiles: HashSet::new(),
    });
}

pub fn send_freeze_target(network: &mut NetworkSync, sender_id: u64, target_id: u64) {
    let payload = encode_freeze_packet(sender_id, target_id);
    let _ = network.socket.send_to(&payload, network.target_addr);
}

pub fn send_projectile_spawn(
    network: &mut NetworkSync,
    sender_id: u64,
    spawn: &crate::scene::ProjectileSpawnData,
) {
    let payload = encode_projectile_packet(sender_id, spawn);
    let _ = network.socket.send_to(&payload, network.target_addr);
}

pub fn apply_local_freeze(
    local_player: Res<LocalPlayerId>,
    mut freeze: ResMut<crate::controls::MovementFreeze>,
    mut network: Option<ResMut<NetworkSync>>,
) {
    let Some(network) = network.as_deref_mut() else {
        return;
    };

    let should_freeze = network
        .pending_freezes
        .drain(..)
        .any(|target_id| target_id == local_player.value);

    if should_freeze {
        freeze.activate_for(FREEZE_DURATION_SECS);
    }
}

pub fn announce_local_presence(
    local_player: Res<LocalPlayerId>,
    local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
    hud: Res<crate::ui::HudState>,
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

    let payload = encode_state_packet(PACKET_JOIN, local_player.value, transform, hud.selected_color());
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

    let payload = encode_state_packet(PACKET_LEAVE, local_player.value, transform, Color::WHITE);
    let _ = network.socket.send_to(&payload, network.target_addr);
}

pub fn send_local_state(
    local_player: Res<LocalPlayerId>,
    local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
    hud: Res<crate::ui::HudState>,
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

    let payload = encode_state_packet(PACKET_STATE, local_player.value, transform, hud.selected_color());
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
                if let Some((_sender_id, target_id)) = decode_freeze_packet(&buf[..len]) {
                    network.pending_freezes.push(target_id);
                } else if let Some((player_id, spawn)) = decode_projectile_packet(&buf[..len]) {
                    if player_id != local_player.value
                        && network
                            .seen_projectiles
                            .insert((player_id, spawn.projectile_id))
                    {
                        network.pending_projectiles.push(RemoteProjectileSpawn { spawn });
                    }
                } else if let Some((packet_type, player_id, transform, color)) = decode_state_packet(&buf[..len]) {
                    if player_id == local_player.value {
                        continue;
                    }

                    match packet_type {
                        PACKET_STATE | PACKET_JOIN => {
                            if let (Some(transform), Some(color)) = (transform, color) {
                                network.remote_states.insert(
                                    player_id,
                                    RemoteState {
                                        transform,
                                        color,
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

pub fn sync_remote_projectiles(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut network: Option<ResMut<NetworkSync>>,
) {
    let Some(network) = network.as_deref_mut() else {
        return;
    };

    for remote_projectile in network.pending_projectiles.drain(..) {
        let entity = crate::scene::spawn_projectile_entity(
            &mut commands,
            &mut meshes,
            &mut materials,
            remote_projectile.spawn.position,
            remote_projectile.spawn.velocity,
            remote_projectile.spawn.lifetime_secs,
        );

        commands.entity(entity).insert((
            crate::scene::ReplicatedProjectileVisual,
        ));
    }
}

pub fn sync_remote_cubes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cube_query: Query<(Entity, &mut Transform, &RemoteCube, &MeshMaterial3d<StandardMaterial>)>,
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
            if let Ok((_entity, mut transform, remote, material_handle)) = cube_query.get_mut(entity) {
                let _ = remote.player_id;
                *transform = state.transform;

                if let Some(mut material) = materials.get_mut(&material_handle.0) {
                    material.base_color = state.color;
                }
            } else {
                network.spawned_entities.remove(player_id);
            }
        } else {
            let entity = commands
                .spawn((
                    Mesh3d(meshes.add(Sphere::new(crate::player::CHICKEN_BODY_RADIUS).mesh().uv(32, 18))),
                    MeshMaterial3d(materials.add(player_material(state.color))),
                    state.transform,
                    GlobalTransform::default(),
                    RemoteCube {
                        player_id: *player_id,
                    },
                    crate::player::HeadTurnDelayTimer {
                        elapsed: 0.0,
                        delay_secs: 0.5,
                    },
                ))
                .with_children(|chicken| {
                    crate::player::spawn_chicken_parts(chicken, &mut *meshes, &mut *materials);
                })
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

fn player_material(color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        metallic: 0.1,
        perceptual_roughness: 0.55,
        ..default()
    }
}

fn encode_state_packet(packet_type: u8, player_id: u64, transform: &Transform, color: Color) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 1 + 8 + (3 + 4 + 3) * 4);
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

    let srgba = color.to_srgba();
    out.extend_from_slice(&srgba.red.to_le_bytes());
    out.extend_from_slice(&srgba.green.to_le_bytes());
    out.extend_from_slice(&srgba.blue.to_le_bytes());

    out
}

fn encode_freeze_packet(sender_id: u64, target_id: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 1 + 8 + 8);
    out.extend_from_slice(&SYNC_MAGIC);
    out.push(SYNC_VERSION);
    out.push(PACKET_FREEZE);
    out.extend_from_slice(&sender_id.to_le_bytes());
    out.extend_from_slice(&target_id.to_le_bytes());
    out
}

fn encode_projectile_packet(sender_id: u64, spawn: &crate::scene::ProjectileSpawnData) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 1 + 8 + 4 + 7 * 4);
    out.extend_from_slice(&SYNC_MAGIC);
    out.push(SYNC_VERSION);
    out.push(PACKET_PROJECTILE);
    out.extend_from_slice(&sender_id.to_le_bytes());
    out.extend_from_slice(&spawn.projectile_id.to_le_bytes());
    out.extend_from_slice(&spawn.position.x.to_le_bytes());
    out.extend_from_slice(&spawn.position.y.to_le_bytes());
    out.extend_from_slice(&spawn.position.z.to_le_bytes());
    out.extend_from_slice(&spawn.velocity.x.to_le_bytes());
    out.extend_from_slice(&spawn.velocity.y.to_le_bytes());
    out.extend_from_slice(&spawn.velocity.z.to_le_bytes());
    out.extend_from_slice(&spawn.lifetime_secs.to_le_bytes());
    out
}

fn decode_state_packet(data: &[u8]) -> Option<(u8, u64, Option<Transform>, Option<Color>)> {
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
        if data.len() != idx {
            return None;
        }
        return Some((packet_type, player_id, None, None));
    }

    if data.len() != idx + (3 + 4 + 3) * 4 {
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
    idx += 4;

    let red = read_f32(&data[idx..idx + 4])?;
    idx += 4;
    let green = read_f32(&data[idx..idx + 4])?;
    idx += 4;
    let blue = read_f32(&data[idx..idx + 4])?;

    let mut transform = Transform::from_xyz(tx, ty, tz);
    transform.rotation = Quat::from_xyzw(rx, ry, rz, rw);

    Some((packet_type, player_id, Some(transform), Some(Color::srgb(red, green, blue))))
}

fn decode_freeze_packet(data: &[u8]) -> Option<(u64, u64)> {
    if data.len() != 4 + 1 + 1 + 8 + 8 {
        return None;
    }
    if data[0..4] != SYNC_MAGIC || data[4] != SYNC_VERSION || data[5] != PACKET_FREEZE {
        return None;
    }

    let sender_id = u64::from_le_bytes(data[6..14].try_into().ok()?);
    let target_id = u64::from_le_bytes(data[14..22].try_into().ok()?);
    Some((sender_id, target_id))
}

fn decode_projectile_packet(
    data: &[u8],
) -> Option<(u64, crate::scene::ProjectileSpawnData)> {
    if data.len() != 4 + 1 + 1 + 8 + 4 + 7 * 4 {
        return None;
    }
    if data[0..4] != SYNC_MAGIC || data[4] != SYNC_VERSION || data[5] != PACKET_PROJECTILE {
        return None;
    }

    let mut idx = 6;
    let player_id = u64::from_le_bytes(data[idx..idx + 8].try_into().ok()?);
    idx += 8;

    let projectile_id = u32::from_le_bytes(data[idx..idx + 4].try_into().ok()?);
    idx += 4;

    let read_f32 = |slice: &[u8]| -> Option<f32> {
        Some(f32::from_le_bytes(slice.try_into().ok()?))
    };

    let position = Vec3::new(
        read_f32(&data[idx..idx + 4])?,
        read_f32(&data[idx + 4..idx + 8])?,
        read_f32(&data[idx + 8..idx + 12])?,
    );
    idx += 12;

    let velocity = Vec3::new(
        read_f32(&data[idx..idx + 4])?,
        read_f32(&data[idx + 4..idx + 8])?,
        read_f32(&data[idx + 8..idx + 12])?,
    );
    idx += 12;

    let lifetime_secs = read_f32(&data[idx..idx + 4])?;

    Some((
        player_id,
        crate::scene::ProjectileSpawnData {
            projectile_id,
            position,
            velocity,
            lifetime_secs,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_packet_roundtrip() {
        let mut transform = Transform::from_xyz(1.25, 2.5, -3.0);
        transform.rotation = Quat::from_xyzw(0.1, 0.2, 0.3, 0.9);
        let color = Color::srgb(0.25, 0.5, 0.75);

        let packet = encode_state_packet(PACKET_STATE, 42, &transform, color);
        let parsed = decode_state_packet(&packet).unwrap();

        assert_eq!(parsed.0, PACKET_STATE);
        assert_eq!(parsed.1, 42);
        assert_eq!(parsed.2.unwrap().translation, transform.translation);
        assert_eq!(parsed.2.unwrap().rotation, transform.rotation);
        assert_eq!(parsed.3.unwrap().to_srgba().red, color.to_srgba().red);
    }

    #[test]
    fn test_leave_packet_roundtrip() {
        let transform = Transform::from_xyz(0.0, 0.0, 0.0);

        let packet = encode_state_packet(PACKET_LEAVE, 77, &transform, Color::WHITE);
        let parsed = decode_state_packet(&packet).unwrap();

        assert_eq!(parsed.0, PACKET_LEAVE);
        assert_eq!(parsed.1, 77);
        assert!(parsed.2.is_none());
        assert!(parsed.3.is_none());
    }
}
