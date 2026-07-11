use bevy::pbr::StandardMaterial;
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub fn drain_remote_projectiles(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    pending_projectiles: &mut Vec<crate::scene::ProjectileSpawnData>,
) {
    for spawn in pending_projectiles.drain(..) {
        let entity = crate::scene::spawn_projectile_entity(
            commands,
            meshes,
            materials,
            spawn.position,
            spawn.velocity,
            spawn.lifetime_secs,
        );

        commands
            .entity(entity)
            .insert(crate::scene::ReplicatedProjectileVisual);
    }
}

pub fn apply_departures<T>(
    commands: &mut Commands,
    departed_players: &mut HashSet<u64>,
    remote_states: &mut HashMap<u64, T>,
    spawned_entities: &mut HashMap<u64, Entity>,
) {
    for player_id in departed_players.drain() {
        remote_states.remove(&player_id);
        if let Some(entity) = spawned_entities.remove(&player_id) {
            commands.entity(entity).despawn();
        }
    }
}

pub fn prune_remote_states<T>(
    remote_states: &mut HashMap<u64, T>,
    now: Instant,
    timeout: Duration,
    mut last_seen: impl FnMut(&T) -> Instant,
) {
    remote_states.retain(|_, state| now.duration_since(last_seen(state)) <= timeout);
}

pub fn prune_seen_projectiles(
    seen_projectiles: &mut HashMap<(u64, u32), Instant>,
    now: Instant,
    ttl: Duration,
) {
    seen_projectiles.retain(|_, seen_at| now.duration_since(*seen_at) <= ttl);
}

pub fn broadcast_projectile_spawn(
    sender_id: u64,
    spawn: &crate::scene::ProjectileSpawnData,
    lan_network: Option<&mut crate::multiplayer::NetworkSync>,
    steam_sync: Option<&mut crate::steam_mp::SteamSync>,
) {
    if let Some(network) = lan_network {
        crate::multiplayer::send_projectile_spawn(network, sender_id, spawn);
    }

    if let Some(steam) = steam_sync {
        crate::steam_mp::send_projectile_spawn(steam, sender_id, spawn);
    }
}

pub fn broadcast_freeze_target(
    sender_id: u64,
    target_id: u64,
    lan_network: Option<&mut crate::multiplayer::NetworkSync>,
    steam_sync: Option<&mut crate::steam_mp::SteamSync>,
) {
    if let Some(network) = lan_network {
        crate::multiplayer::send_freeze_target(network, sender_id, target_id);
    }

    if let Some(steam) = steam_sync {
        crate::steam_mp::send_freeze_target(steam, sender_id, target_id);
    }
}

pub fn find_remote_hit_target(
    projectile_pos: Vec3,
    hit_radius_sq: f32,
    lan_remote_cubes: &Query<(&Transform, &crate::multiplayer::RemoteCube)>,
    steam_remote_cubes: &Query<(&Transform, &crate::steam_mp::SteamRemoteCube)>,
) -> Option<u64> {
    for (remote_transform, remote_cube) in lan_remote_cubes {
        if projectile_pos.distance_squared(remote_transform.translation) <= hit_radius_sq {
            return Some(remote_cube.player_id);
        }
    }

    for (remote_transform, remote_cube) in steam_remote_cubes {
        if projectile_pos.distance_squared(remote_transform.translation) <= hit_radius_sq {
            return Some(remote_cube.player_id);
        }
    }

    None
}
