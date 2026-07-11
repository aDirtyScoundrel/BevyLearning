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
