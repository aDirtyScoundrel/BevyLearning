//! Steamworks-based remote cube sync.
//!
//! This supplements LAN broadcast sync by allowing direct P2P transform packets
//! between Steam users for remote multiplayer testing.

use bevy::prelude::*;

#[cfg(feature = "steamworks")]
mod imp {
    use super::*;
    use bevy::math::primitives::Cuboid;
    use bevy::mesh::Mesh3d;
    use bevy::pbr::{MeshMaterial3d, StandardMaterial};
    use std::collections::{HashMap, HashSet};
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
        seen_projectiles: HashSet<(u64, u32)>,
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
            seen_projectiles: HashSet::new(),
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
                        && steam
                            .seen_projectiles
                            .insert((player_id, spawn.projectile_id))
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
                        Mesh3d(meshes.add(Cuboid::new(1.5, 1.5, 1.5).mesh().build())),
                        MeshMaterial3d(materials.add(player_material(state.color))),
                        state.transform,
                        GlobalTransform::default(),
                        SteamRemoteCube {
                            player_id: *player_id,
                        },
                    ))
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
        let mut out = Vec::with_capacity(4 + 1 + 1 + 8 + (3 + 4 + 3) * 4);
        out.extend_from_slice(&STEAM_SYNC_MAGIC);
        out.push(STEAM_SYNC_VERSION);
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
        out.extend_from_slice(&STEAM_SYNC_MAGIC);
        out.push(STEAM_SYNC_VERSION);
        out.push(PACKET_FREEZE);
        out.extend_from_slice(&sender_id.to_le_bytes());
        out.extend_from_slice(&target_id.to_le_bytes());
        out
    }

    fn encode_projectile_packet(
        sender_id: u64,
        spawn: &crate::scene::ProjectileSpawnData,
    ) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + 1 + 1 + 8 + 4 + 7 * 4);
        out.extend_from_slice(&STEAM_SYNC_MAGIC);
        out.push(STEAM_SYNC_VERSION);
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

    fn decode_packet(data: &[u8]) -> Option<(u8, u64, Option<Transform>, Option<Color>)> {
        if data.len() < 4 + 1 + 1 + 8 {
            return None;
        }
        if data[0..4] != STEAM_SYNC_MAGIC || data[4] != STEAM_SYNC_VERSION {
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
        if data[0..4] != STEAM_SYNC_MAGIC || data[4] != STEAM_SYNC_VERSION || data[5] != PACKET_FREEZE {
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
        if data[0..4] != STEAM_SYNC_MAGIC || data[4] != STEAM_SYNC_VERSION || data[5] != PACKET_PROJECTILE {
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
