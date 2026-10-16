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

// Packet framing: every packet starts with a 4-byte magic tag and a 1-byte version.
const SYNC_MAGIC: [u8; 4] = *b"CUBE";
const SYNC_VERSION: u8 = 1;

// Packet type discriminators used in the second header byte.
const PACKET_STATE: u8 = 1;       // periodic position/color update
const PACKET_JOIN: u8 = 2;        // initial announce on connection
const PACKET_LEAVE: u8 = 3;       // graceful disconnect notice
const PACKET_FREEZE: u8 = 4;      // freeze a target player briefly
const PACKET_PROJECTILE: u8 = 5;  // replicate a projectile spawn

pub(crate) const SYNC_PORT: u16 = 34567;
// Tick rate for outbound state packets (~20 Hz).
const SEND_INTERVAL: Duration = Duration::from_millis(50);
// Drop a remote peer's state after this much silence.
const REMOTE_TIMEOUT: Duration = Duration::from_secs(5);
// How long to remember projectile IDs for deduplication.
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
    // ── Core transport ───────────────────────────────────────────────────────
    socket: UdpSocket,
    /// Destination address for outbound packets; broadcast by default.
    target_addr: SocketAddr,
    last_send: Instant,
    presence_state: PresenceState,
    role: RuntimeNetRole,

    // ── Server / client managers ─────────────────────────────────────────────
    /// Present when this peer is running as the LAN auth-server.
    server_runtime: Option<learning::server::ServerNetworkManager>,
    /// Present when this peer is running as an untrusted client.
    client_manager: Option<learning::client::ClientNetworkManager>,
    last_server_broadcast: Instant,

    // ── Authoritative sim (server only) ──────────────────────────────────────
    /// Latest validated input sample per remote player.
    authoritative_inputs: HashMap<u64, PlayerInputSample>,
    /// Vertical velocity for each remote player's simulated cube.
    authoritative_vertical_velocity: HashMap<u64, f32>,

    // ── Client reconciliation ────────────────────────────────────────────────
    /// Authoritative transform snapshot pushed by the server; lerped to each frame.
    pending_local_reconciliation: Option<Transform>,

    // ── Remote entity state ───────────────────────────────────────────────────
    /// Latest known position/color per remote player ID.
    remote_states: HashMap<u64, RemoteState>,
    /// Bevy entity spawned for each remote player.
    spawned_entities: HashMap<u64, Entity>,
    /// Players that sent a LEAVE packet; cleaned up on the next sync pass.
    departed_players: HashSet<u64>,
    /// Freeze targets queued from incoming packets; drained each frame.
    pending_freezes: Vec<u64>,
    /// Projectile spawns queued from incoming packets; drained each frame.
    pending_projectiles: Vec<crate::scene::ProjectileSpawnData>,
    /// Recently seen (player_id, projectile_id) pairs used for deduplication.
    seen_projectiles: HashMap<(u64, u32), Instant>,
}

impl NetworkSync {
    /// True when this peer is running as the LAN authoritative server —
    /// its own local player must be simulated the same way as every remote
    /// player, not trusted as ground truth. See
    /// docs/THIN_CLIENT_MIGRATION.md slice 1.
    pub fn is_auth_server(&self) -> bool {
        self.role == RuntimeNetRole::AuthServer
    }
}

/// Send `payload` to `target_addr` via UDP (fire-and-forget; errors are silently dropped).
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

/// Generate a locally-unique player ID from wall-clock nanoseconds XOR'd with
/// the process ID, giving distinct values across machines and across multiple
/// game instances on the same machine.
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

/// Overrides `setup_network`'s usual env-var-based role detection to force
/// a loopback `UntrustedClient` connection to an embedded server
/// subprocess. Inserted by `singleplayer.rs` before entering `AppState::InGame`
/// in single-player mode; absent for every other run mode (LAN/Steam/
/// headless server all keep using env vars exactly as before).
#[derive(Resource, Debug, Clone)]
pub struct SinglePlayerConnect {
    pub target_addr: SocketAddr,
    pub auth_secret: String,
}

pub fn setup_network(
    mut commands: Commands,
    local_player: Res<LocalPlayerId>,
    single_player: Option<Res<SinglePlayerConnect>>,
) {
    let role = if single_player.is_some() {
        RuntimeNetRole::UntrustedClient
    } else if env::var("CUBE_AUTH_SERVER")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        RuntimeNetRole::AuthServer
    } else if env::var("CUBE_AUTH_SERVER_ADDR").is_ok() {
        RuntimeNetRole::UntrustedClient
    } else {
        RuntimeNetRole::LegacyPeer
    };

    // AuthServer and LegacyPeer both need the well-known port — the server
    // must be discoverable there, and LegacyPeer's broadcast model assumes
    // every peer listens on the same fixed port. UntrustedClient has no such
    // requirement (the auth handshake replies to whatever source address the
    // packet arrived from) and binding an ephemeral port instead lets a
    // client run in the same process/host as its own embedded server — see
    // `singleplayer.rs`, which spawns a headless server subprocess still
    // bound to SYNC_PORT while this process connects to it as a client.
    let bind_addr = match role {
        RuntimeNetRole::UntrustedClient => "0.0.0.0:0".to_string(),
        RuntimeNetRole::AuthServer | RuntimeNetRole::LegacyPeer => format!("0.0.0.0:{}", SYNC_PORT),
    };
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

    let target_addr = single_player
        .as_ref()
        .map(|sp| sp.target_addr)
        .or_else(|| {
            env::var("CUBE_AUTH_SERVER_ADDR")
                .ok()
                .and_then(|s| s.parse::<SocketAddr>().ok())
        })
        .or_else(|| {
            env::var("CUBE_SYNC_TARGET")
                .ok()
                .and_then(|s| s.parse::<SocketAddr>().ok())
        })
        .unwrap_or_else(|| SocketAddr::from(([255, 255, 255, 255], SYNC_PORT)));

    let server_runtime = if role == RuntimeNetRole::AuthServer {
        let auth_secret = env::var("CUBE_AUTH_SECRET").unwrap_or_else(|_| "dev-auth-secret".to_string());
        let auth_secret = match learning::map_identity::configured_auth_secret(&auth_secret) {
            Ok(secret) => secret,
            Err(error) => {
                eprintln!("[multiplayer] {error}");
                return;
            }
        };
        Some(learning::server::ServerNetworkManager::with_auth(
            SocketAddr::from(([0, 0, 0, 0], SYNC_PORT)),
            learning::server::ServerAuthConfig {
                auth_secret: auth_secret.clone(),
                token_secret: auth_secret,
            },
        ))
    } else {
        None
    };

    let client_manager = if role == RuntimeNetRole::UntrustedClient {
        let auth_secret = single_player
            .as_ref()
            .map(|sp| sp.auth_secret.clone())
            .or_else(|| env::var("CUBE_AUTH_SECRET").ok())
            .unwrap_or_else(|| "dev-auth-secret".to_string());
        let auth_secret = match learning::map_identity::configured_auth_secret(&auth_secret) {
            Ok(secret) => secret,
            Err(error) => {
                eprintln!("[multiplayer] {error}");
                return;
            }
        };
        let mut manager = learning::client::ClientNetworkManager::with_identity(
            target_addr,
            local_player.value,
            auth_secret,
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
        server_runtime,
        client_manager,
        last_server_broadcast: Instant::now(),
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
    mut retry: Local<learning::client::HandshakeRetry>,
) {
    let Some(network) = network.as_deref_mut() else {
        return;
    };

    if network.role == RuntimeNetRole::UntrustedClient {
        let authenticated = network.client_manager.as_ref().is_some_and(|manager|
            manager.auth_state == learning::client::ClientAuthState::Authenticated);
        match retry.poll(Instant::now(), authenticated) {
            learning::client::HandshakeAction::SendHello => {
                send_payload(network, &encode_auth_hello(local_player.value));
            }
            learning::client::HandshakeAction::TimedOut => {
                if let Some(manager) = network.client_manager.as_mut() {
                    manager.auth_state = learning::client::ClientAuthState::Unauthenticated;
                }
                eprintln!("[net] Connection timed out. Check the server address and authentication settings.");
            }
            learning::client::HandshakeAction::Wait => {}
        }
        return;
    }
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
    map: Option<Res<crate::doom_wad::WadCollisionWorld>>,
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

            // The host's own player goes through the exact same
            // authoritative sim as every remote player — it is not
            // trusted as ground truth. See docs/THIN_CLIENT_MIGRATION.md
            // slice 1. `move_cube` no longer writes physics for this
            // player when `is_auth_server()` is true; the result comes
            // back via `apply_local_sim_result`.
            if let (Some(hud), Some(input_intent)) = (hud.as_ref(), input_intent.as_ref()) {
                network.authoritative_inputs.insert(
                    local_player.value,
                    PlayerInputSample {
                        move_x: input_intent.move_x,
                        move_z: input_intent.move_z,
                        jump: input_intent.jump,
                        color: hud.selected_color(),
                    },
                );
            }

            crate::server_tick::step_authoritative_sim(
                &mut network.remote_states,
                &network.authoritative_inputs,
                &mut network.authoritative_vertical_velocity,
                &ergo,
                SEND_INTERVAL.as_secs_f32(),
                map.as_deref(),
            );
            server_broadcast_snapshot(network);
            network.last_server_broadcast = Instant::now();
            network.last_send = Instant::now();
        }
    }
}

/// Copies the host's own authoritative sim result (written into
/// `remote_states` by `send_local_state`'s `AuthServer` branch, keyed by
/// the host's own player id) back onto the rendered local player entity.
/// This is the other half of slice 1 in docs/THIN_CLIENT_MIGRATION.md —
/// `move_cube` intentionally stops writing this transform on the host.
pub fn apply_local_sim_result(
    local_player: Res<LocalPlayerId>,
    network: Option<Res<NetworkSync>>,
    mut local_cube: Query<&mut Transform, With<crate::RotatingCube>>,
) {
    let Some(network) = network.as_deref() else {
        return;
    };
    if !network.is_auth_server() {
        return;
    }
    let Some(state) = network.remote_states.get(&local_player.value) else {
        return;
    };
    let Ok(mut transform) = local_cube.single_mut() else {
        return;
    };
    *transform = state.transform;
}

pub fn receive_remote_states(
    local_player: Res<LocalPlayerId>,
    mut network: Option<ResMut<NetworkSync>>,
) {
    let Some(network) = network.as_deref_mut() else {
        return;
    };

    let now = Instant::now();

    for _ in 0..256 {
        let mut buf = [0u8; crate::auth_codec::MAX_PACKET_BYTES + 1];
        match network.socket.recv_from(&mut buf) {
            Ok((len, from)) => {
                if len > crate::auth_codec::MAX_PACKET_BYTES {
                    continue;
                }
                let data = &buf[..len];
                match network.role {
                    RuntimeNetRole::LegacyPeer => {
                        process_incoming_packet(network, local_player.value, data)
                    }
                    RuntimeNetRole::UntrustedClient => {
                        process_client_packet(network, from, local_player.value, data);
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
    local_player: Res<LocalPlayerId>,
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
    if let Some(server) = network.server_runtime.as_mut() {
        // Mirror steam_mp::sweep_expired_auth_sessions: an evicted player
        // must lose its sim input and velocity entries, or the shared
        // authoritative sim keeps refreshing its last_seen and its ghost
        // persists in every snapshot forever. departed_players drives the
        // visual despawn and remote-state removal in apply_departures.
        for player_id in server.sweep_expired_sessions(REMOTE_TIMEOUT) {
            network.authoritative_inputs.remove(&player_id);
            network.authoritative_vertical_velocity.remove(&player_id);
            network.departed_players.insert(player_id);
        }
    }

    // The host's own player (see `apply_local_sim_result`) is now present
    // in `remote_states` on the auth-server so it gets simulated/broadcast
    // like everyone else, but it must never get a duplicate `RemoteCube`
    // visual entity here — it renders through the local `RotatingCube`.
    for (player_id, state) in &network.remote_states {
        if *player_id == local_player.value {
            continue;
        }
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
                    crate::player::spawn_chicken_parts(chicken, &mut meshes, &mut materials);
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

fn process_client_packet(network: &mut NetworkSync, from: SocketAddr, local_player_id: u64, data: &[u8]) {
    let Some(manager) = network.client_manager.as_ref() else {
        return;
    };
    if !crate::auth_codec::client_packet_allowed(from == network.target_addr, manager.auth_state, data) {
        return;
    }
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
    let Some(server) = network.server_runtime.as_mut() else {
        return;
    };

    if let Some(player_id) = decode_auth_hello(data) {
        if let Ok((_session_id, learning::server::ServerEgressPacket::AuthChallenge { nonce })) =
            server.handle_connection_request(from, player_id)
        {
            let challenge = encode_auth_challenge(nonce);
            let _ = network.socket.send_to(&challenge, from);
        }
        return;
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
                    network
                        .remote_states
                        .entry(player_id)
                        .or_insert(RemoteState {
                            transform: Transform::from_xyz(0.0, crate::player::CUBE_REST_Y, 0.0),
                            color: Color::srgb(0.96, 0.94, 0.88),
                            last_seen: now,
                        });
                }
        return;
    }

    if let Some((PACKET_LEAVE, player_id, _, _)) = decode_state_packet(data) {
        // Graceful leave. Honor it only from the address of an
        // authenticated session bound to the claimed player id, then drop
        // the sim state immediately instead of ghosting until the session
        // timeout. Mirrors the client-side PACKET_LEAVE handling.
        if let Some(session_id) = server.session_by_addr.get(&from).copied()
            && let Some(session) = server.sessions.get(&session_id)
            && session.is_authenticated()
            && session.player_id == Some(player_id)
        {
            server.close_session(session_id);
            network.authoritative_inputs.remove(&player_id);
            network.authoritative_vertical_velocity.remove(&player_id);
            network.departed_players.insert(player_id);
        }
        return;
    }

    if let Some((session_token, input_sequence, payload)) = decode_input_packet(data)
        && let Ok(disposition) = server.process_client_packet(
            from,
            learning::server::ServerIngressPacket::ClientInput {
                session_token,
                input_sequence,
                payload: &payload,
            },
        )
            && disposition == learning::server::PacketDisposition::Accepted
                && let Some(session_id) = server.session_by_addr.get(&from).copied()
                && let Some(session) = server.sessions.get(&session_id)
                && let Some(player_id) = session.player_id
                && let Some((move_x, move_z, jump, color)) = decode_input_payload(&payload)
            {
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

fn server_broadcast_snapshot(network: &mut NetworkSync) {
    let Some(server) = network.server_runtime.as_ref() else {
        return;
    };

    let states = network.remote_states.iter()
        .map(|(player_id, state)| (*player_id, state.transform, state.color))
        .collect::<Vec<_>>();
    for snapshot in crate::auth_codec::snapshot_packets(SYNC_MAGIC, SYNC_VERSION, &states) {
        for session in server.sessions.values() {
            if session.is_authenticated() {
                let _ = network.socket.send_to(&snapshot, session.addr);
            }
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
            server_runtime: None,
            client_manager: None,
            last_server_broadcast: Instant::now(),
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

    /// Builds an AuthServer-role network with one fully authenticated
    /// client (hello → proof → one input packet) and live sim state.
    fn authed_server_network(client_addr: SocketAddr, player_id: u64) -> NetworkSync {
        let mut network = make_test_network();
        network.role = RuntimeNetRole::AuthServer;
        network.server_runtime = Some(learning::server::ServerNetworkManager::with_auth(
            network.socket.local_addr().unwrap(),
            learning::server::ServerAuthConfig {
                auth_secret: "test-only".into(),
                token_secret: "test-only".into(),
            },
        ));
        process_server_packet(&mut network, client_addr, &encode_auth_hello(player_id), Instant::now());
        let session_id = network.server_runtime.as_ref().unwrap().session_by_addr[&client_addr];
        let nonce = network.server_runtime.as_ref().unwrap().sessions[&session_id].auth_nonce;
        let proof = learning::auth::make_auth_proof("test-only", player_id, nonce);
        process_server_packet(&mut network, client_addr, &encode_auth_proof(proof), Instant::now());
        let token = network.server_runtime.as_ref().unwrap().sessions[&session_id]
            .session_token
            .expect("token after auth");
        let input = encode_input_packet(token, 1, &encode_input_payload(0.5, 0.0, false, Color::WHITE));
        process_server_packet(&mut network, client_addr, &input, Instant::now());
        assert!(network.authoritative_inputs.contains_key(&player_id));
        // Simulate a mid-step velocity entry as step_authoritative_sim would leave.
        network.authoritative_vertical_velocity.insert(player_id, 1.5);
        network
    }

    #[test]
    fn lan_server_sweep_drops_sim_state_for_silent_clients() {
        let client_addr = SocketAddr::from(([127, 0, 0, 1], 34590));
        let mut network = authed_server_network(client_addr, 42);

        // Age the session past REMOTE_TIMEOUT.
        let session_id = network.server_runtime.as_ref().unwrap().session_by_addr[&client_addr];
        network.server_runtime.as_mut().unwrap().sessions.get_mut(&session_id).unwrap().last_activity =
            Instant::now() - REMOTE_TIMEOUT - Duration::from_secs(1);

        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .insert_resource(LocalPlayerId { value: 0 })
            .insert_resource(network)
            .add_systems(Update, sync_remote_cubes);
        // First pass runs the sweep and queues the departure; second pass
        // applies it (remote-state removal + visual despawn).
        app.update();
        app.update();

        let network = app.world().resource::<NetworkSync>();
        assert!(
            network.server_runtime.as_ref().unwrap().sessions.is_empty(),
            "silent session must be evicted"
        );
        assert!(!network.authoritative_inputs.contains_key(&42), "sim input must be dropped");
        assert!(!network.authoritative_vertical_velocity.contains_key(&42), "velocity must be dropped");
        assert!(!network.remote_states.contains_key(&42), "ghost state must be dropped");
        assert!(network.departed_players.is_empty(), "departure must be applied");
    }

    #[test]
    fn lan_server_honors_leave_from_authenticated_session_only() {
        let client_addr = SocketAddr::from(([127, 0, 0, 1], 34591));
        let mut network = authed_server_network(client_addr, 42);

        // A leave claiming a foreign id must be ignored.
        let bogus = encode_state_packet(PACKET_LEAVE, 43, &Transform::default(), Color::WHITE);
        process_server_packet(&mut network, client_addr, &bogus, Instant::now());
        assert!(network.authoritative_inputs.contains_key(&42));
        assert_eq!(network.server_runtime.as_ref().unwrap().sessions.len(), 1);

        // A leave from an unrelated address must be ignored.
        let outsider = SocketAddr::from(([127, 0, 0, 1], 34592));
        let spoof = encode_state_packet(PACKET_LEAVE, 42, &Transform::default(), Color::WHITE);
        process_server_packet(&mut network, outsider, &spoof, Instant::now());
        assert!(network.authoritative_inputs.contains_key(&42));

        // The real leave closes the session and drops the sim state.
        let leave = encode_state_packet(PACKET_LEAVE, 42, &Transform::default(), Color::WHITE);
        process_server_packet(&mut network, client_addr, &leave, Instant::now());
        assert!(!network.authoritative_inputs.contains_key(&42));
        assert!(!network.authoritative_vertical_velocity.contains_key(&42));
        assert!(network.departed_players.contains(&42));
        assert!(network.server_runtime.as_ref().unwrap().sessions.is_empty());
    }

    #[test]
    fn lan_receives_multi_packet_snapshots_without_truncation() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut network = make_test_network();
        network.role = RuntimeNetRole::UntrustedClient;
        network.target_addr = server.local_addr().unwrap();
        let client_addr = network.socket.local_addr().unwrap();
        let mut manager = learning::client::ClientNetworkManager::with_identity(network.target_addr, 0, "test-only".into());
        manager.mark_authenticated(1);
        network.client_manager = Some(manager);
        let count = crate::auth_codec::MAX_SNAPSHOT_STATES + 2;
        let states: Vec<_> = (0..count).map(|id| (id as u64, Transform::default(), Color::WHITE)).collect();
        let packets = crate::auth_codec::snapshot_packets(SYNC_MAGIC, SYNC_VERSION, &states);
        assert_eq!(packets.len(), 2);
        for packet in packets {
            assert!(packet.len() <= crate::auth_codec::MAX_PACKET_BYTES);
            server.send_to(&packet, client_addr).unwrap();
        }
        let mut app = App::new();
        app.insert_resource(LocalPlayerId { value: 0 })
            .insert_resource(network)
            .add_systems(Update, receive_remote_states);
        app.update();
        let network = app.world().resource::<NetworkSync>();
        assert_eq!(network.remote_states.len(), count - 1);
        assert!(network.pending_local_reconciliation.is_some());
    }

    #[test]
    fn lan_rejects_untrusted_and_unauthenticated_snapshots() {
        let mut network = make_test_network();
        network.role = RuntimeNetRole::UntrustedClient;
        let host = network.target_addr;
        let outsider = SocketAddr::from(([127, 0, 0, 1], 34568));
        let mut manager = learning::client::ClientNetworkManager::with_identity(host, 42, "test-only".into());
        manager.connect().unwrap();
        network.client_manager = Some(manager);
        let packet = crate::auth_codec::encode_snapshot_packet(
            SYNC_MAGIC, SYNC_VERSION, &[(42, Transform::from_xyz(50.0, 0.0, 0.0), Color::WHITE)],
        );
        process_client_packet(&mut network, host, 42, &packet);
        assert!(network.pending_local_reconciliation.is_none());
        process_client_packet(&mut network, outsider, 42, &encode_auth_challenge(123));
        assert_eq!(network.client_manager.as_ref().unwrap().auth_state,
            learning::client::ClientAuthState::AwaitingChallenge);
        process_client_packet(&mut network, host, 42, &encode_auth_accept(123));
        assert!(network.client_manager.as_ref().unwrap().session_token.is_none());
        network.client_manager.as_mut().unwrap().mark_authenticated(123);
        process_client_packet(&mut network, outsider, 42, &packet);
        assert!(network.pending_local_reconciliation.is_none());
        process_client_packet(&mut network, host, 42, &packet);
        assert_eq!(network.pending_local_reconciliation.unwrap().translation.x, 50.0);
    }

    #[test]
    fn lan_handshake_preserves_local_identity() {
        handshake_roundtrip(false);
    }

    #[test]
    fn lan_map_identity_rejects_different_content_and_map_name() {
        use learning::map_identity::MapIdentity;
        for (bytes, name, accepted) in [
            (&b"fixture-a"[..], "MAP01", true),
            (&b"fixture-b"[..], "MAP01", false),
            (&b"fixture-a"[..], "MAP02", false),
        ] {
            let mut server = make_test_network();
            let mut client = make_test_network();
            server.role = RuntimeNetRole::AuthServer;
            client.role = RuntimeNetRole::UntrustedClient;
            let host = server.socket.local_addr().unwrap();
            let peer = client.socket.local_addr().unwrap();
            client.target_addr = host;
            for socket in [&server.socket, &client.socket] {
                socket.set_nonblocking(false).unwrap();
                socket.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
            }
            let identity = MapIdentity::from_reader(&b"fixture-a"[..], "MAP01").unwrap();
            let secret = learning::auth::bind_map_secret("test-only", &identity.0);
            server.server_runtime = Some(learning::server::ServerNetworkManager::with_auth(
                host, learning::server::ServerAuthConfig { auth_secret: secret.clone(), token_secret: secret },
            ));
            let identity = MapIdentity::from_reader(bytes, name).unwrap();
            let secret = learning::auth::bind_map_secret("test-only", &identity.0);
            let mut manager = learning::client::ClientNetworkManager::with_identity(host, 42, secret);
            manager.connect().unwrap();
            client.client_manager = Some(manager);
            client.socket.send_to(&encode_auth_hello(42), host).unwrap();
            let mut buffer = [0u8; 256];
            let (length, source) = server.socket.recv_from(&mut buffer).unwrap();
            assert_eq!(source, peer);
            process_server_packet(&mut server, source, &buffer[..length], Instant::now());
            let (length, source) = client.socket.recv_from(&mut buffer).unwrap();
            process_client_packet(&mut client, source, 42, &buffer[..length]);
            let (length, source) = server.socket.recv_from(&mut buffer).unwrap();
            process_server_packet(&mut server, source, &buffer[..length], Instant::now());
            assert_eq!(server.remote_states.contains_key(&42), accepted);
            if accepted {
                let (length, source) = client.socket.recv_from(&mut buffer).unwrap();
                process_client_packet(&mut client, source, 42, &buffer[..length]);
                assert_eq!(client.client_manager.as_ref().unwrap().auth_state, learning::client::ClientAuthState::Authenticated);
            } else {
                let error = client.socket.recv_from(&mut buffer).unwrap_err();
                assert!(matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut));
                assert_ne!(client.client_manager.as_ref().unwrap().auth_state, learning::client::ClientAuthState::Authenticated);
            }
        }
    }

    #[test]
    fn lan_handshake_recovers_lost_hello_and_accept() {
        handshake_roundtrip(true);
    }

    fn handshake_roundtrip(drop_packets: bool) {
        let mut server = make_test_network();
        server.role = RuntimeNetRole::AuthServer;
        server.socket.set_nonblocking(false).unwrap();
        server.socket.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let address = server.socket.local_addr().unwrap();
        server.server_runtime = Some(learning::server::ServerNetworkManager::with_auth(
            address,
            learning::server::ServerAuthConfig {
                auth_secret: "test-only".into(),
                token_secret: "test-token-only".into(),
            },
        ));
        let mut app = App::new();
        app.insert_resource(LocalPlayerId { value: 42 })
            .insert_resource(SinglePlayerConnect {
                target_addr: address,
                auth_secret: "test-only".into(),
            })
            .add_systems(Startup, setup_network)
            .add_systems(Update, (announce_local_presence, receive_remote_states).chain());
        app.update();
        let mut bytes = [0; 256];
        if drop_packets {
            let (length, _) = server.socket.recv_from(&mut bytes).unwrap();
            assert_eq!(decode_auth_hello(&bytes[..length]), Some(42));
            std::thread::sleep(Duration::from_millis(510));
            app.update();
        }
        let (length, from) = server.socket.recv_from(&mut bytes).unwrap();
        assert_eq!(decode_auth_hello(&bytes[..length]), Some(42));
        process_server_packet(&mut server, from, &bytes[..length], Instant::now());
        app.update();
        let (length, from) = server.socket.recv_from(&mut bytes).unwrap();
        assert_eq!(decode_auth_proof(&bytes[..length]).unwrap().player_id, 42);
        process_server_packet(&mut server, from, &bytes[..length], Instant::now());
        if drop_packets {
            let (length, _) = app.world().resource::<NetworkSync>().socket.recv_from(&mut bytes).unwrap();
            assert!(decode_auth_accept(&bytes[..length]).is_some());
            std::thread::sleep(Duration::from_millis(510));
            app.update();
            let (length, from) = server.socket.recv_from(&mut bytes).unwrap();
            assert_eq!(decode_auth_hello(&bytes[..length]), Some(42));
            process_server_packet(&mut server, from, &bytes[..length], Instant::now());
            app.update();
            let (length, from) = server.socket.recv_from(&mut bytes).unwrap();
            assert!(decode_auth_proof(&bytes[..length]).is_some());
            process_server_packet(&mut server, from, &bytes[..length], Instant::now());
        }
        app.update();
        let network = app.world().resource::<NetworkSync>();
        let manager = network.client_manager.as_ref().unwrap();
        assert_eq!(manager.player_id, 42);
        assert_eq!(manager.auth_state, learning::client::ClientAuthState::Authenticated);
        assert!(server.remote_states.contains_key(&42));
        server_broadcast_snapshot(&mut server);
        app.update();
        let network = app.world().resource::<NetworkSync>();
        assert!(network.pending_local_reconciliation.is_some());
        assert!(!network.remote_states.contains_key(&42));
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
