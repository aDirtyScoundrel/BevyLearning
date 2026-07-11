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
const PROJECTILE_DEDUP_TTL: Duration = Duration::from_secs(15);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresenceState {
    Pending,
    Announced,
}

#[derive(Resource)]
pub struct NetworkSync {
    socket: UdpSocket,
    target_addr: SocketAddr,
    last_send: Instant,
    presence_state: PresenceState,
    remote_states: HashMap<u64, RemoteState>,
    spawned_entities: HashMap<u64, Entity>,
    departed_players: HashSet<u64>,
    pending_freezes: Vec<u64>,
    pending_projectiles: Vec<crate::scene::ProjectileSpawnData>,
    seen_projectiles: HashMap<(u64, u32), Instant>,
}

fn send_payload(network: &NetworkSync, payload: &[u8]) {
    let _ = network.socket.send_to(payload, network.target_addr);
}

fn process_incoming_packet(network: &mut NetworkSync, local_player_id: u64, data: &[u8]) {
    if let Some((_sender_id, target_id)) = decode_freeze_packet(data) {
        network.pending_freezes.push(target_id);
        return;
    }

    if let Some((player_id, spawn)) = decode_projectile_packet(data) {
        if player_id != local_player_id
            && crate::sync_codec::accept_recent_projectile(
                &mut network.seen_projectiles,
                player_id,
                spawn.projectile_id,
                Instant::now(),
                PROJECTILE_DEDUP_TTL,
            )
        {
            network.pending_projectiles.push(spawn);
        }
        return;
    }

    if let Some((packet_type, player_id, transform, color)) = decode_state_packet(data) {
        if player_id == local_player_id {
            return;
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
                network.departed_players.insert(player_id);
            }
            _ => {}
        }
    }
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
        presence_state: PresenceState::Pending,
        remote_states: HashMap::new(),
        spawned_entities: HashMap::new(),
        departed_players: HashSet::new(),
        pending_freezes: Vec::new(),
        pending_projectiles: Vec::new(),
        seen_projectiles: HashMap::new(),
    });
}

pub fn send_freeze_target(network: &mut NetworkSync, sender_id: u64, target_id: u64) {
    let payload = encode_freeze_packet(sender_id, target_id);
    send_payload(network, &payload);
}

pub fn send_projectile_spawn(
    network: &mut NetworkSync,
    sender_id: u64,
    spawn: &crate::scene::ProjectileSpawnData,
) {
    let payload = encode_projectile_packet(sender_id, spawn);
    send_payload(network, &payload);
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

    if network.presence_state == PresenceState::Announced {
        return;
    }

    let Ok(transform) = local_cube_query.single() else {
        return;
    };

    let payload = encode_state_packet(PACKET_JOIN, local_player.value, transform, hud.selected_color());
    send_payload(network, &payload);
    network.presence_state = PresenceState::Announced;
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
    send_payload(network, &payload);
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
    send_payload(network, &payload);
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
            Ok((len, _from)) => process_incoming_packet(network, local_player.value, &buf[..len]),
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

    crate::remote_runtime::drain_remote_projectiles(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut network.pending_projectiles,
    );
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

    crate::remote_runtime::apply_departures(
        &mut commands,
        &mut network.departed_players,
        &mut network.remote_states,
        &mut network.spawned_entities,
    );

    let now = Instant::now();
    crate::remote_runtime::prune_remote_states(
        &mut network.remote_states,
        now,
        REMOTE_TIMEOUT,
        |state| state.last_seen,
    );
    crate::remote_runtime::prune_seen_projectiles(
        &mut network.seen_projectiles,
        now,
        PROJECTILE_DEDUP_TTL,
    );

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
    crate::sync_codec::encode_state_packet(
        SYNC_MAGIC,
        SYNC_VERSION,
        packet_type,
        PACKET_LEAVE,
        player_id,
        transform,
        color,
    )
}

fn encode_freeze_packet(sender_id: u64, target_id: u64) -> Vec<u8> {
    crate::sync_codec::encode_freeze_packet(
        SYNC_MAGIC,
        SYNC_VERSION,
        PACKET_FREEZE,
        sender_id,
        target_id,
    )
}

fn encode_projectile_packet(sender_id: u64, spawn: &crate::scene::ProjectileSpawnData) -> Vec<u8> {
    crate::sync_codec::encode_projectile_packet(
        SYNC_MAGIC,
        SYNC_VERSION,
        PACKET_PROJECTILE,
        sender_id,
        spawn.projectile_id,
        spawn.position,
        spawn.velocity,
        spawn.lifetime_secs,
    )
}

fn decode_state_packet(data: &[u8]) -> Option<(u8, u64, Option<Transform>, Option<Color>)> {
    crate::sync_codec::decode_state_packet(SYNC_MAGIC, SYNC_VERSION, PACKET_LEAVE, data)
}

fn decode_freeze_packet(data: &[u8]) -> Option<(u64, u64)> {
    crate::sync_codec::decode_freeze_packet(SYNC_MAGIC, SYNC_VERSION, PACKET_FREEZE, data)
}

fn decode_projectile_packet(
    data: &[u8],
) -> Option<(u64, crate::scene::ProjectileSpawnData)> {
    let (player_id, spawn) = crate::sync_codec::decode_projectile_packet(
        SYNC_MAGIC,
        SYNC_VERSION,
        PACKET_PROJECTILE,
        data,
    )?;

    Some((
        player_id,
        crate::scene::ProjectileSpawnData {
            projectile_id: spawn.projectile_id,
            position: spawn.position,
            velocity: spawn.velocity,
            lifetime_secs: spawn.lifetime_secs,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn make_test_network() -> NetworkSync {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();

        NetworkSync {
            socket,
            target_addr: SocketAddr::from(([127, 0, 0, 1], 34567)),
            last_send: Instant::now(),
            presence_state: PresenceState::Pending,
            remote_states: HashMap::new(),
            spawned_entities: HashMap::new(),
            departed_players: HashSet::new(),
            pending_freezes: Vec::new(),
            pending_projectiles: Vec::new(),
            seen_projectiles: HashMap::new(),
        }
    }

    #[test]
    fn test_state_packet_roundtrip() {
        let mut transform = Transform::from_xyz(1.25, 2.5, -3.0);
        transform.rotation = Quat::from_xyzw(0.1, 0.2, 0.3, 0.9).normalize();
        let color = Color::srgb(0.25, 0.5, 0.75);

        let packet = encode_state_packet(PACKET_STATE, 42, &transform, color);
        let parsed = decode_state_packet(&packet).unwrap();
        let decoded_transform = parsed.2.unwrap();

        assert_eq!(parsed.0, PACKET_STATE);
        assert_eq!(parsed.1, 42);
        assert_eq!(decoded_transform.translation, transform.translation);
        assert!((decoded_transform.rotation.x - transform.rotation.x).abs() < 0.00001);
        assert!((decoded_transform.rotation.y - transform.rotation.y).abs() < 0.00001);
        assert!((decoded_transform.rotation.z - transform.rotation.z).abs() < 0.00001);
        assert!((decoded_transform.rotation.w - transform.rotation.w).abs() < 0.00001);
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

    #[test]
    fn test_ingress_lifecycle_join_state_leave() {
        let mut network = make_test_network();
        let local_player_id = 1;
        let remote_player_id = 42;

        let join_transform = Transform::from_xyz(1.0, 2.0, 3.0);
        let join_packet = encode_state_packet(
            PACKET_JOIN,
            remote_player_id,
            &join_transform,
            Color::srgb(0.2, 0.3, 0.4),
        );
        process_incoming_packet(&mut network, local_player_id, &join_packet);

        assert!(network.remote_states.contains_key(&remote_player_id));

        let state_transform = Transform::from_xyz(4.0, 5.0, 6.0);
        let state_packet = encode_state_packet(
            PACKET_STATE,
            remote_player_id,
            &state_transform,
            Color::srgb(0.8, 0.7, 0.6),
        );
        process_incoming_packet(&mut network, local_player_id, &state_packet);

        let updated_state = network.remote_states.get(&remote_player_id).unwrap();
        assert_eq!(updated_state.transform.translation, state_transform.translation);

        let leave_packet = encode_state_packet(
            PACKET_LEAVE,
            remote_player_id,
            &Transform::default(),
            Color::WHITE,
        );
        process_incoming_packet(&mut network, local_player_id, &leave_packet);

        assert!(network.departed_players.contains(&remote_player_id));
    }

    #[test]
    fn test_ingress_projectile_dedup_and_freeze() {
        let mut network = make_test_network();
        let local_player_id = 1;
        let remote_player_id = 9;

        let spawn = crate::scene::ProjectileSpawnData {
            projectile_id: 7,
            position: Vec3::new(1.0, 2.0, 3.0),
            velocity: Vec3::new(4.0, 5.0, 6.0),
            lifetime_secs: 1.25,
        };

        let projectile_packet = encode_projectile_packet(remote_player_id, &spawn);
        process_incoming_packet(&mut network, local_player_id, &projectile_packet);
        process_incoming_packet(&mut network, local_player_id, &projectile_packet);

        assert_eq!(network.pending_projectiles.len(), 1);
        assert_eq!(network.pending_projectiles[0].projectile_id, spawn.projectile_id);

        let freeze_packet = encode_freeze_packet(remote_player_id, local_player_id);
        process_incoming_packet(&mut network, local_player_id, &freeze_packet);

        assert!(network.pending_freezes.contains(&local_player_id));
    }
}
