//! Steamworks-based remote cube sync.
//!
//! This supplements LAN broadcast sync by allowing direct P2P transform packets
//! between Steam users for remote multiplayer testing.

use bevy::prelude::*;

#[cfg(feature = "steamworks")]
mod imp {
    use super::*;
    use bevy::math::primitives::Sphere;
    use bevy::mesh::Mesh3d;
    use bevy::pbr::{MeshMaterial3d, StandardMaterial};
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    const STEAM_SYNC_MAGIC: [u8; 4] = *b"STMC";
    const STEAM_SYNC_VERSION: u8 = 1;
    const PACKET_STATE: u8 = 1;
    const PACKET_JOIN: u8 = 2;
    const PACKET_LEAVE: u8 = 3;
    const PACKET_FREEZE: u8 = 4;
    const PACKET_PROJECTILE: u8 = 5;
    const SEND_INTERVAL: Duration = Duration::from_millis(50);
    const REMOTE_TIMEOUT: Duration = Duration::from_secs(5);
    const PROJECTILE_DEDUP_TTL: Duration = Duration::from_secs(15);
    const FREEZE_DURATION_SECS: f32 = 2.0;

    #[derive(Component, Debug, Clone, Copy)]
    pub struct SteamRemoteCube {
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
    pub struct SteamSync {
        pub client: steamworks::Client,
        pub targets: Vec<steamworks::SteamId>,
        pub last_send: Instant,
        pub announced_presence: bool,
        remote_states: HashMap<u64, RemoteState>,
        spawned_entities: HashMap<u64, Entity>,
        departed_players: Vec<u64>,
        pending_freezes: Vec<u64>,
        pending_projectiles: Vec<RemoteProjectileSpawn>,
        seen_projectiles: HashMap<(u64, u32), Instant>,
    }

    pub fn setup_steam_sync(mut commands: Commands) {
        let targets = std::env::var("STEAM_REMOTE_IDS")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .filter_map(|s| s.trim().parse::<u64>().ok())
                    .map(steamworks::SteamId::from_raw)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let Ok((client, _single)) = steamworks::Client::init() else {
            eprintln!("[steam-mp] Steam API init failed; Steam transport disabled");
            return;
        };

        let my_id = client.user().steam_id();
        println!("[steam-mp] local steam id: {}", my_id.raw());

        if targets.is_empty() {
            println!(
                "[steam-mp] no STEAM_REMOTE_IDS configured; set comma-separated Steam64 IDs to enable P2P sync"
            );
        } else {
            let target_list = targets
                .iter()
                .map(|id| id.raw().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            println!("[steam-mp] targets: {}", target_list);
        }

        commands.insert_resource(SteamSync {
            client,
            targets,
            last_send: Instant::now(),
            announced_presence: false,
            remote_states: HashMap::new(),
            spawned_entities: HashMap::new(),
            departed_players: Vec::new(),
            pending_freezes: Vec::new(),
            pending_projectiles: Vec::new(),
            seen_projectiles: HashMap::new(),
        });
    }

    pub fn send_freeze_target(steam: &mut SteamSync, sender_id: u64, target_id: u64) {
        if steam.targets.is_empty() {
            return;
        }

        let payload = encode_freeze_packet(sender_id, target_id);
        let networking = steam.client.networking();

        for target in &steam.targets {
            networking.accept_p2p_session(*target);
            let _ = networking.send_p2p_packet(
                *target,
                steamworks::SendType::UnreliableNoDelay,
                &payload,
            );
        }
    }

    pub fn send_projectile_spawn(
        steam: &mut SteamSync,
        spawn: &crate::scene::ProjectileSpawnData,
    ) {
        if steam.targets.is_empty() {
            return;
        }

        let sender_id = steam.client.user().steam_id().raw();
        let payload = encode_projectile_packet(sender_id, spawn);
        let networking = steam.client.networking();

        for target in &steam.targets {
            networking.accept_p2p_session(*target);
            let _ = networking.send_p2p_packet(
                *target,
                steamworks::SendType::UnreliableNoDelay,
                &payload,
            );
        }
    }

    pub fn apply_local_freeze(
        _local_player: Res<crate::multiplayer::LocalPlayerId>,
        mut freeze: ResMut<crate::controls::MovementFreeze>,
        mut steam: Option<ResMut<SteamSync>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        let local_steam_id = steam.client.user().steam_id().raw();

        let should_freeze = steam
            .pending_freezes
            .drain(..)
            .any(|target_id| target_id == local_steam_id);

        if should_freeze {
            freeze.activate_for(FREEZE_DURATION_SECS);
        }
    }

    pub fn announce_local_presence(
        local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
        hud: Res<crate::ui::HudState>,
        mut steam: Option<ResMut<SteamSync>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        if steam.announced_presence || steam.targets.is_empty() {
            return;
        }

        let Ok(transform) = local_cube_query.single() else {
            return;
        };

        let local_id = steam.client.user().steam_id().raw();
        let payload = encode_packet(PACKET_JOIN, local_id, transform, hud.selected_color());
        let networking = steam.client.networking();

        for target in &steam.targets {
            networking.accept_p2p_session(*target);
            let _ = networking.send_p2p_packet(
                *target,
                steamworks::SendType::UnreliableNoDelay,
                &payload,
            );
        }

        steam.announced_presence = true;
    }

    pub fn send_local_leave(
        exit_requested: Res<crate::ExitRequested>,
        local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
        mut steam: Option<ResMut<SteamSync>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        if !exit_requested.0 {
            return;
        }

        if steam.targets.is_empty() {
            return;
        }

        let Ok(local_transform) = local_cube_query.single() else {
            return;
        };

        let local_id = steam.client.user().steam_id().raw();
        let payload = encode_packet(PACKET_LEAVE, local_id, local_transform, Color::WHITE);
        let networking = steam.client.networking();

        for target in &steam.targets {
            networking.accept_p2p_session(*target);
            let _ = networking.send_p2p_packet(
                *target,
                steamworks::SendType::UnreliableNoDelay,
                &payload,
            );
        }
    }

    pub fn process_callbacks() {}

    pub fn send_local_state(
        local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
        hud: Res<crate::ui::HudState>,
        mut steam: Option<ResMut<SteamSync>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };
        if steam.targets.is_empty() || steam.last_send.elapsed() < SEND_INTERVAL {
            return;
        }

        let Ok(transform) = local_cube_query.single() else {
            return;
        };

        let local_id = steam.client.user().steam_id().raw();
        let payload = encode_packet(PACKET_STATE, local_id, transform, hud.selected_color());
        let networking = steam.client.networking();

        for target in &steam.targets {
            networking.accept_p2p_session(*target);
            let _ = networking.send_p2p_packet(
                *target,
                steamworks::SendType::UnreliableNoDelay,
                &payload,
            );
        }

        steam.last_send = Instant::now();
    }

    pub fn receive_remote_states(mut steam: Option<ResMut<SteamSync>>) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        let local_id = steam.client.user().steam_id().raw();
        let networking = steam.client.networking();

        while let Some(size) = networking.is_p2p_packet_available() {
            let mut buf = vec![0u8; size];
            if let Some((_remote, packet_size)) = networking.read_p2p_packet(&mut buf) {
                if let Some((packet_type, player_id, transform, color)) = decode_packet(&buf[..packet_size]) {
                    if player_id == local_id {
                        continue;
                    }
                    match packet_type {
                        PACKET_STATE | PACKET_JOIN => {
                            if let (Some(transform), Some(color)) = (transform, color) {
                                steam.remote_states.insert(
                                    player_id,
                                    RemoteState {
                                        transform,
                                        color,
                                        last_seen: Instant::now(),
                                    },
                                );
                            }
                        }
                        PACKET_LEAVE => steam.departed_players.push(player_id),
                        _ => {}
                    }
                } else if let Some((player_id, spawn)) = decode_projectile_packet(&buf[..packet_size]) {
                    if player_id != local_id
                        && crate::sync_codec::accept_recent_projectile(
                            &mut steam.seen_projectiles,
                            player_id,
                            spawn.projectile_id,
                            Instant::now(),
                            PROJECTILE_DEDUP_TTL,
                        )
                    {
                        steam.pending_projectiles.push(RemoteProjectileSpawn { spawn });
                    }
                } else if let Some((_sender_id, target_id)) = decode_freeze_packet(&buf[..packet_size]) {
                    steam.pending_freezes.push(target_id);
                }
            } else {
                break;
            }
        }
    }

    pub fn sync_remote_projectiles(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut steam: Option<ResMut<SteamSync>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        for remote_projectile in steam.pending_projectiles.drain(..) {
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
        mut cube_query: Query<(Entity, &mut Transform, &SteamRemoteCube, &MeshMaterial3d<StandardMaterial>)>,
        mut steam: Option<ResMut<SteamSync>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        for player_id in steam.departed_players.drain(..) {
            steam.remote_states.remove(&player_id);
            if let Some(entity) = steam.spawned_entities.remove(&player_id) {
                commands.entity(entity).despawn();
            }
        }

        let now = Instant::now();
        steam
            .remote_states
            .retain(|_, state| now.duration_since(state.last_seen) <= REMOTE_TIMEOUT);

        steam
            .seen_projectiles
            .retain(|_, seen_at| now.duration_since(*seen_at) <= PROJECTILE_DEDUP_TTL);

        for (player_id, state) in &steam.remote_states {
            if let Some(entity) = steam.spawned_entities.get(player_id).copied() {
                if let Ok((_entity, mut transform, remote, material_handle)) = cube_query.get_mut(entity) {
                    let _ = remote.player_id;
                    *transform = state.transform;

                    if let Some(mut material) = materials.get_mut(&material_handle.0) {
                        material.base_color = state.color;
                    }
                } else {
                    steam.spawned_entities.remove(player_id);
                }
            } else {
                let entity = commands
                    .spawn((
                        Mesh3d(meshes.add(Sphere::new(crate::player::CHICKEN_BODY_RADIUS).mesh().uv(32, 18))),
                        MeshMaterial3d(materials.add(player_material(state.color))),
                        state.transform,
                        GlobalTransform::default(),
                        SteamRemoteCube {
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
                steam.spawned_entities.insert(*player_id, entity);
            }
        }

        let active_ids: std::collections::HashSet<u64> = steam.remote_states.keys().copied().collect();
        let stale_ids: Vec<u64> = steam
            .spawned_entities
            .keys()
            .filter(|id| !active_ids.contains(id))
            .copied()
            .collect();

        for player_id in stale_ids {
            if let Some(entity) = steam.spawned_entities.remove(&player_id) {
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

    fn encode_packet(packet_type: u8, player_id: u64, transform: &Transform, color: Color) -> Vec<u8> {
        crate::sync_codec::encode_state_packet(
            STEAM_SYNC_MAGIC,
            STEAM_SYNC_VERSION,
            packet_type,
            player_id,
            transform,
            color,
        )
    }

    fn encode_freeze_packet(sender_id: u64, target_id: u64) -> Vec<u8> {
        crate::sync_codec::encode_freeze_packet(
            STEAM_SYNC_MAGIC,
            STEAM_SYNC_VERSION,
            PACKET_FREEZE,
            sender_id,
            target_id,
        )
    }

    fn encode_projectile_packet(
        sender_id: u64,
        spawn: &crate::scene::ProjectileSpawnData,
    ) -> Vec<u8> {
        crate::sync_codec::encode_projectile_packet(
            STEAM_SYNC_MAGIC,
            STEAM_SYNC_VERSION,
            PACKET_PROJECTILE,
            sender_id,
            spawn.projectile_id,
            spawn.position,
            spawn.velocity,
            spawn.lifetime_secs,
        )
    }

    fn decode_packet(data: &[u8]) -> Option<(u8, u64, Option<Transform>, Option<Color>)> {
        crate::sync_codec::decode_state_packet(STEAM_SYNC_MAGIC, STEAM_SYNC_VERSION, PACKET_LEAVE, data)
    }

    fn decode_freeze_packet(data: &[u8]) -> Option<(u64, u64)> {
        crate::sync_codec::decode_freeze_packet(
            STEAM_SYNC_MAGIC,
            STEAM_SYNC_VERSION,
            PACKET_FREEZE,
            data,
        )
    }

    fn decode_projectile_packet(
        data: &[u8],
    ) -> Option<(u64, crate::scene::ProjectileSpawnData)> {
        let (player_id, spawn) = crate::sync_codec::decode_projectile_packet(
            STEAM_SYNC_MAGIC,
            STEAM_SYNC_VERSION,
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
}

#[cfg(not(feature = "steamworks"))]
mod imp {
    use super::*;

    #[derive(Component, Debug, Clone, Copy)]
    pub struct SteamRemoteCube {
        pub player_id: u64,
    }

    #[derive(Resource)]
    pub struct SteamSync;

    pub fn setup_steam_sync(_commands: Commands) {}
    pub fn process_callbacks() {}
    pub fn send_freeze_target(_steam: &mut SteamSync, _sender_id: u64, _target_id: u64) {}
    pub fn send_projectile_spawn(
        _steam: &mut SteamSync,
        _spawn: &crate::scene::ProjectileSpawnData,
    ) {
    }
    pub fn apply_local_freeze(
        _local_player: Res<crate::multiplayer::LocalPlayerId>,
        _freeze: ResMut<crate::controls::MovementFreeze>,
    ) {
    }
    pub fn announce_local_presence(_local_cube_query: Query<&Transform, With<crate::RotatingCube>>) {}
    pub fn send_local_leave(
        _exit_requested: Res<crate::ExitRequested>,
        _local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
    ) {
    }
    pub fn send_local_state(_local_cube_query: Query<&Transform, With<crate::RotatingCube>>) {}
    pub fn receive_remote_states() {}
    pub fn sync_remote_cubes(
        _commands: Commands,
        _meshes: ResMut<Assets<Mesh>>,
        _materials: ResMut<Assets<StandardMaterial>>,
    ) {
    }
    pub fn sync_remote_projectiles(
        _commands: Commands,
        _meshes: ResMut<Assets<Mesh>>,
        _materials: ResMut<Assets<StandardMaterial>>,
    ) {
    }
}

pub use imp::*;
