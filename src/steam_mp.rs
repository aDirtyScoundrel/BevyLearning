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
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    const STEAM_SYNC_MAGIC: [u8; 4] = *b"STMC";
    const STEAM_SYNC_VERSION: u8 = 1;
    const SEND_INTERVAL: Duration = Duration::from_millis(50);
    const REMOTE_TIMEOUT: Duration = Duration::from_secs(5);

    #[derive(Component, Debug, Clone, Copy)]
    pub struct SteamRemoteCube {
        pub player_id: u64,
    }

    #[derive(Debug, Clone)]
    struct RemoteState {
        transform: Transform,
        last_seen: Instant,
    }

    #[derive(Resource)]
    pub struct SteamSync {
        pub client: steamworks::Client,
        pub targets: Vec<steamworks::SteamId>,
        pub last_send: Instant,
        remote_states: HashMap<u64, RemoteState>,
        spawned_entities: HashMap<u64, Entity>,
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
            remote_states: HashMap::new(),
            spawned_entities: HashMap::new(),
        });
    }

    pub fn process_callbacks() {}

    pub fn send_local_state(
        local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
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
        let payload = encode_packet(local_id, transform);
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
                if let Some((player_id, transform)) = decode_packet(&buf[..packet_size]) {
                    if player_id == local_id {
                        continue;
                    }
                    steam.remote_states.insert(
                        player_id,
                        RemoteState {
                            transform,
                            last_seen: Instant::now(),
                        },
                    );
                }
            } else {
                break;
            }
        }
    }

    pub fn sync_remote_cubes(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut cube_query: Query<(Entity, &mut Transform, &SteamRemoteCube)>,
        mut steam: Option<ResMut<SteamSync>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        let now = Instant::now();
        steam
            .remote_states
            .retain(|_, state| now.duration_since(state.last_seen) <= REMOTE_TIMEOUT);

        for (player_id, state) in &steam.remote_states {
            if let Some(entity) = steam.spawned_entities.get(player_id).copied() {
                if let Ok((_entity, mut transform, remote)) = cube_query.get_mut(entity) {
                    let _ = remote.player_id;
                    *transform = state.transform;
                } else {
                    steam.spawned_entities.remove(player_id);
                }
            } else {
                let entity = commands
                    .spawn((
                        Mesh3d(meshes.add(Cuboid::new(1.5, 1.5, 1.5).mesh().build())),
                        MeshMaterial3d(materials.add(StandardMaterial {
                            base_color: Color::srgb(0.2, 0.9, 0.45),
                            metallic: 0.1,
                            perceptual_roughness: 0.55,
                            ..default()
                        })),
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

    fn encode_packet(player_id: u64, transform: &Transform) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + 1 + 8 + (3 + 4) * 4);
        out.extend_from_slice(&STEAM_SYNC_MAGIC);
        out.push(STEAM_SYNC_VERSION);
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

    fn decode_packet(data: &[u8]) -> Option<(u64, Transform)> {
        const SIZE: usize = 4 + 1 + 8 + (3 + 4) * 4;
        if data.len() != SIZE {
            return None;
        }
        if data[0..4] != STEAM_SYNC_MAGIC || data[4] != STEAM_SYNC_VERSION {
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
}

#[cfg(not(feature = "steamworks"))]
mod imp {
    use super::*;

    pub fn setup_steam_sync(_commands: Commands) {}
    pub fn process_callbacks() {}
    pub fn send_local_state(_local_cube_query: Query<&Transform, With<crate::RotatingCube>>) {}
    pub fn receive_remote_states() {}
    pub fn sync_remote_cubes(
        _commands: Commands,
        _meshes: ResMut<Assets<Mesh>>,
        _materials: ResMut<Assets<StandardMaterial>>,
    ) {
    }
}

pub use imp::*;
