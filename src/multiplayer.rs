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
pub(crate) struct RemoteState {
    pub(crate) transform: Transform,
    pub(crate) color: Color,
    pub(crate) last_seen: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresenceState {
    Pending,
    Announced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeNetRole {
    LegacyPeer,
    AuthServer,
    UntrustedClient,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PlayerInputSample {
    pub(crate) move_x: f32,
    pub(crate) move_z: f32,
    pub(crate) jump: bool,
    pub(crate) color: Color,
}

impl Default for PlayerInputSample {
    fn default() -> Self {
        Self {
            move_x: 0.0,
            move_z: 0.0,
            jump: false,
            color: Color::srgb(0.96, 0.94, 0.88),
        }
    }
}

#[derive(Resource)]
pub struct NetworkSync {
    socket: UdpSocket,
    target_addr: SocketAddr,
    last_send: Instant,
    presence_state: PresenceState,
    role: RuntimeNetRole,
    auth_runtime: Option<learning::server::ServerNetworkManager>,
    client_manager: Option<learning::client::ClientNetworkManager>,
    last_server_broadcast: Instant,
    token_to_player: HashMap<learning::auth::SessionToken, u64>,
    player_ingress_addr: HashMap<u64, SocketAddr>,
    last_input_sequence_by_player: HashMap<u64, u32>,
    authoritative_inputs: HashMap<u64, PlayerInputSample>,
    authoritative_vertical_velocity: HashMap<u64, f32>,
    pending_local_reconciliation: Option<Transform>,
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

    let role = if env::var("CUBE_AUTH_SERVER")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        RuntimeNetRole::AuthServer
    } else if env::var("CUBE_AUTH_SERVER_ADDR").is_ok() {
        RuntimeNetRole::UntrustedClient
    } else {
        RuntimeNetRole::LegacyPeer
    };

    let target_addr = env::var("CUBE_AUTH_SERVER_ADDR")
        .ok()
        .and_then(|s| s.parse::<SocketAddr>().ok())
        .or_else(|| {
            env::var("CUBE_SYNC_TARGET")
                .ok()
                .and_then(|s| s.parse::<SocketAddr>().ok())
        })
        .unwrap_or_else(|| SocketAddr::from(([255, 255, 255, 255], SYNC_PORT)));

    let auth_runtime = if role == RuntimeNetRole::AuthServer {
        Some(learning::server::ServerNetworkManager::new(
            SocketAddr::from(([0, 0, 0, 0], SYNC_PORT)),
        ))
    } else {
        None
    };

    let client_manager = if role == RuntimeNetRole::UntrustedClient {
        let mut manager = learning::client::ClientNetworkManager::with_identity(
            target_addr,
            generate_local_player_id().value,
            env::var("CUBE_AUTH_SECRET").unwrap_or_else(|_| "dev-auth-secret".to_string()),
        );
        let _ = manager.connect();
        Some(manager)
    } else {
        None
    };

    println!(
        "[multiplayer] listening on {} and broadcasting to {}",
        bind_addr, target_addr
    );

    commands.insert_resource(NetworkSync {
        socket,
        target_addr,
        last_send: Instant::now(),
        presence_state: PresenceState::Pending,
        role,
        auth_runtime,
        client_manager,
        last_server_broadcast: Instant::now(),
        token_to_player: HashMap::new(),
        player_ingress_addr: HashMap::new(),
        last_input_sequence_by_player: HashMap::new(),
        authoritative_inputs: HashMap::new(),
        authoritative_vertical_velocity: HashMap::new(),
        pending_local_reconciliation: None,
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

pub fn apply_local_reconciliation(
    time: Res<Time>,
    mut network: Option<ResMut<NetworkSync>>,
    mut local_cube: Query<&mut Transform, With<crate::RotatingCube>>,
) {
    let Some(network) = network.as_deref_mut() else {
        return;
    };

    let Some(target) = network.pending_local_reconciliation else {
        return;
    };

    let Ok(mut transform) = local_cube.single_mut() else {
        return;
    };

    let alpha = (time.delta_secs() * 12.0).clamp(0.0, 1.0);
    transform.translation = transform.translation.lerp(target.translation, alpha);

    if transform.translation.distance(target.translation) < 0.01 {
        transform.translation = target.translation;
        network.pending_local_reconciliation = None;
    }
}

pub fn announce_local_presence(
    local_player: Res<LocalPlayerId>,
    local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
    hud: Option<Res<crate::ui::HudState>>,
    mut network: Option<ResMut<NetworkSync>>,
) {
    let Some(network) = network.as_deref_mut() else {
        return;
    };

    if network.presence_state == PresenceState::Announced {
        return;
    }

    match network.role {
        RuntimeNetRole::UntrustedClient => {
            let payload = encode_auth_hello(local_player.value);
            send_payload(network, &payload);
        }
        RuntimeNetRole::LegacyPeer => {
            let (Ok(transform), Some(hud)) = (local_cube_query.single(), hud.as_ref()) else {
                return;
            };

            let payload = encode_state_packet(PACKET_JOIN, local_player.value, transform, hud.selected_color());
            send_payload(network, &payload);
        }
        RuntimeNetRole::AuthServer => {}
    }
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
    hud: Option<Res<crate::ui::HudState>>,
    ergo: Res<crate::config::HumanErgoConfig>,
    input_intent: Option<Res<crate::controls::PlayerInputIntent>>,
    mut network: Option<ResMut<NetworkSync>>,
) {
    let Some(network) = network.as_deref_mut() else {
        return;
    };

    if network.last_send.elapsed() < SEND_INTERVAL {
        return;
    }

    match network.role {
        RuntimeNetRole::LegacyPeer => {
            let (Ok(transform), Some(hud)) = (local_cube_query.single(), hud.as_ref()) else {
                return;
            };

            let payload = encode_state_packet(PACKET_STATE, local_player.value, transform, hud.selected_color());
            send_payload(network, &payload);
            network.last_send = Instant::now();
        }
        RuntimeNetRole::UntrustedClient => {
            let (Some(manager), Some(hud), Some(input_intent)) = (
                network.client_manager.as_mut(),
                hud.as_ref(),
                input_intent.as_ref(),
            ) else {
                return;
            };
            let input_payload = encode_input_payload(
                input_intent.move_x,
                input_intent.move_z,
                input_intent.jump,
                hud.selected_color(),
            );
            let Ok(packet) = manager.next_input_packet(&input_payload) else {
                return;
            };

            let payload = encode_input_packet(packet.session_token, packet.input_sequence, packet.payload);
            send_payload(network, &payload);
            network.last_send = Instant::now();
        }
        RuntimeNetRole::AuthServer => {
            if network.last_server_broadcast.elapsed() < SEND_INTERVAL {
                return;
            }

            crate::server_tick::step_authoritative_sim(
                &mut network.remote_states,
                &network.authoritative_inputs,
                &mut network.authoritative_vertical_velocity,
                &ergo,
                SEND_INTERVAL.as_secs_f32(),
            );
            server_broadcast_snapshot(network);
            network.last_server_broadcast = Instant::now();
            network.last_send = Instant::now();
        }
    }
}

pub fn receive_remote_states(
    local_player: Res<LocalPlayerId>,
    mut network: Option<ResMut<NetworkSync>>,
) {
    let Some(network) = network.as_deref_mut() else {
        return;
    };

    let now = Instant::now();

    loop {
        let mut buf = [0u8; 96];
        match network.socket.recv_from(&mut buf) {
            Ok((len, from)) => {
                let data = &buf[..len];
                match network.role {
                    RuntimeNetRole::LegacyPeer => {
                        process_incoming_packet(network, local_player.value, data)
                    }
                    RuntimeNetRole::UntrustedClient => {
                        process_client_packet(network, local_player.value, data);
                    }
                    RuntimeNetRole::AuthServer => {
                        process_server_packet(network, from, data, now);
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
                    crate::player::ChickenBody,
                    crate::player::WalkCycleState::new(Vec2::new(
                        state.transform.translation.x,
                        state.transform.translation.z,
                    )),
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

fn process_client_packet(network: &mut NetworkSync, local_player_id: u64, data: &[u8]) {
    if let Some(nonce) = decode_auth_challenge(data) {
        if let Some(manager) = network.client_manager.as_mut() {
            let proof = manager.respond_to_challenge(nonce);
            let payload = encode_auth_proof(proof);
            send_payload(network, &payload);
        }
        return;
    }

    if let Some(token) = decode_auth_accept(data) {
        if let Some(manager) = network.client_manager.as_mut() {
            manager.mark_authenticated(token);
        }
        return;
    }

    if let Some(states) = decode_snapshot_packet(data) {
        for (player_id, transform, color) in states {
            if player_id == local_player_id {
                network.pending_local_reconciliation = Some(transform);
                continue;
            }
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
}

fn process_server_packet(network: &mut NetworkSync, from: SocketAddr, data: &[u8], now: Instant) {
    if process_auth_service_packet(network, from, data, now) {
        return;
    }

    process_game_service_packet(network, from, data, now);
}

fn process_auth_service_packet(
    network: &mut NetworkSync,
    from: SocketAddr,
    data: &[u8],
    now: Instant,
) -> bool {
    let Some(server) = network.auth_runtime.as_mut() else {
        return false;
    };

    if let Some(player_id) = decode_auth_hello(data) {
        if let Ok((_session_id, learning::server::ServerEgressPacket::AuthChallenge { nonce })) =
            server.handle_connection_request(from)
        {
            let challenge = encode_auth_challenge(nonce);
            let _ = network.socket.send_to(&challenge, from);
            if let Some(session_id) = server.session_by_addr.get(&from).copied()
                && let Some(session) = server.sessions.get_mut(&session_id)
            {
                session.player_id = Some(player_id);
            }
        }
        return true;
    }

    if let Some(proof) = decode_auth_proof(data) {
        if let Ok(disposition) =
            server.process_client_packet(from, learning::server::ServerIngressPacket::AuthResponse { proof })
            && disposition == learning::server::PacketDisposition::Accepted
            && let Some(session_id) = server.session_by_addr.get(&from).copied()
            && let Some(session) = server.sessions.get(&session_id)
            && let Some(token) = session.session_token
            && let Some(player_id) = session.player_id
        {
            let accept = encode_auth_accept(token);
            let _ = network.socket.send_to(&accept, from);
            network.token_to_player.insert(token, player_id);
            network.remote_states.entry(player_id).or_insert(RemoteState {
                transform: Transform::from_xyz(0.0, crate::player::CUBE_REST_Y, 0.0),
                color: Color::srgb(0.96, 0.94, 0.88),
                last_seen: now,
            });
        }
        return true;
    }

    false
}

fn process_game_service_packet(network: &mut NetworkSync, from: SocketAddr, data: &[u8], now: Instant) {
    if let Some((session_token, input_sequence, payload)) = decode_input_packet(data)
        && let Some(player_id) = network.token_to_player.get(&session_token).copied()
    {
        let accepted_addr = match network.player_ingress_addr.get(&player_id).copied() {
            Some(addr) => addr == from,
            None => {
                network.player_ingress_addr.insert(player_id, from);
                true
            }
        };

        if !accepted_addr {
            return;
        }

        let last_sequence = network
            .last_input_sequence_by_player
            .get(&player_id)
            .copied()
            .unwrap_or(0);
        if input_sequence <= last_sequence {
            return;
        }

        if let Some((move_x, move_z, jump, color)) = decode_input_payload(&payload) {
            network
                .last_input_sequence_by_player
                .insert(player_id, input_sequence);
            network.authoritative_inputs.insert(
                player_id,
                PlayerInputSample {
                    move_x,
                    move_z,
                    jump,
                    color,
                },
            );

            if let Some(state) = network.remote_states.get_mut(&player_id) {
                state.color = color;
                state.last_seen = now;
            }
        }
    }
}

fn server_broadcast_snapshot(network: &mut NetworkSync) {
    let Some(server) = network.auth_runtime.as_ref() else {
        return;
    };

    let snapshot = encode_snapshot_packet(
        network
            .remote_states
            .iter()
            .map(|(player_id, state)| (*player_id, state.transform, state.color))
            .collect::<Vec<_>>()
            .as_slice(),
    );

    for session in server.sessions.values() {
        if session.is_authenticated() {
            let _ = network.socket.send_to(&snapshot, session.addr);
        }
    }
}

fn encode_auth_hello(player_id: u64) -> Vec<u8> {
    crate::auth_codec::encode_auth_hello(SYNC_MAGIC, SYNC_VERSION, player_id)
}

fn decode_auth_hello(data: &[u8]) -> Option<u64> {
    crate::auth_codec::decode_auth_hello(SYNC_MAGIC, SYNC_VERSION, data)
}

fn encode_auth_challenge(nonce: u64) -> Vec<u8> {
    crate::auth_codec::encode_auth_challenge(SYNC_MAGIC, SYNC_VERSION, nonce)
}

fn decode_auth_challenge(data: &[u8]) -> Option<u64> {
    crate::auth_codec::decode_auth_challenge(SYNC_MAGIC, SYNC_VERSION, data)
}

fn encode_auth_proof(proof: learning::auth::AuthProof) -> Vec<u8> {
    crate::auth_codec::encode_auth_proof(SYNC_MAGIC, SYNC_VERSION, proof)
}

fn decode_auth_proof(data: &[u8]) -> Option<learning::auth::AuthProof> {
    crate::auth_codec::decode_auth_proof(SYNC_MAGIC, SYNC_VERSION, data)
}

fn encode_auth_accept(token: learning::auth::SessionToken) -> Vec<u8> {
    crate::auth_codec::encode_auth_accept(SYNC_MAGIC, SYNC_VERSION, token)
}

fn decode_auth_accept(data: &[u8]) -> Option<learning::auth::SessionToken> {
    crate::auth_codec::decode_auth_accept(SYNC_MAGIC, SYNC_VERSION, data)
}

fn encode_input_payload(move_x: f32, move_z: f32, jump: bool, color: Color) -> Vec<u8> {
    crate::auth_codec::encode_input_payload(move_x, move_z, jump, color)
}

fn decode_input_payload(payload: &[u8]) -> Option<(f32, f32, bool, Color)> {
    crate::auth_codec::decode_input_payload(payload)
}

fn encode_input_packet(session_token: learning::auth::SessionToken, input_sequence: u32, payload: &[u8]) -> Vec<u8> {
    crate::auth_codec::encode_input_packet(SYNC_MAGIC, SYNC_VERSION, session_token, input_sequence, payload)
}

fn decode_input_packet(data: &[u8]) -> Option<(learning::auth::SessionToken, u32, Vec<u8>)> {
    crate::auth_codec::decode_input_packet(SYNC_MAGIC, SYNC_VERSION, data)
}

fn encode_snapshot_packet(states: &[(u64, Transform, Color)]) -> Vec<u8> {
    crate::auth_codec::encode_snapshot_packet(SYNC_MAGIC, SYNC_VERSION, states)
}

fn decode_snapshot_packet(data: &[u8]) -> Option<Vec<(u64, Transform, Color)>> {
    crate::auth_codec::decode_snapshot_packet(SYNC_MAGIC, SYNC_VERSION, data)
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
            role: RuntimeNetRole::LegacyPeer,
            auth_runtime: None,
            client_manager: None,
            last_server_broadcast: Instant::now(),
            token_to_player: HashMap::new(),
            player_ingress_addr: HashMap::new(),
            last_input_sequence_by_player: HashMap::new(),
            authoritative_inputs: HashMap::new(),
            authoritative_vertical_velocity: HashMap::new(),
            pending_local_reconciliation: None,
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
