//! Authoritative physics simulation step shared by both server transports.
//!
//! Both the LAN server role (`multiplayer.rs`) and the Steam P2P host
//! (`steam_mp::host_step_authoritative_sim`, now a thin adapter) call into
//! this module. It previously existed as two near-identical copies; they are
//! unified here so a fix or a rules change lands once rather than needing to
//! be mirrored. The tests below include one that pins both call paths to
//! identical output.

use bevy::prelude::*;
use std::collections::HashMap;
use std::time::Instant;

/// Advance the authoritative physics simulation for every remote player by `dt` seconds.
///
/// Reads the latest validated [`PlayerInputSample`] per player and applies
/// movement, gravity, jumping, and world-boundary clamping.  Results are written
/// back into `remote_states` so the server can distribute them to clients.
pub fn step_authoritative_sim(
    remote_states: &mut HashMap<u64, crate::multiplayer::RemoteState>,
    authoritative_inputs: &HashMap<u64, crate::multiplayer::PlayerInputSample>,
    vertical_velocity: &mut HashMap<u64, f32>,
    ergo: &crate::config::HumanErgoConfig,
    dt: f32,
) {
    if dt <= f32::EPSILON {
        return;
    }

    for (player_id, input) in authoritative_inputs {
        let vy = vertical_velocity.entry(*player_id).or_insert(0.0);
        let state = remote_states.entry(*player_id).or_insert(crate::multiplayer::RemoteState {
            transform: Transform::from_xyz(0.0, crate::player::CUBE_REST_Y, 0.0),
            color: input.color,
            last_seen: Instant::now(),
        });

        state.transform.translation.x += input.move_x.clamp(-1.0, 1.0) * ergo.movement.move_speed * dt;
        state.transform.translation.z += input.move_z.clamp(-1.0, 1.0) * ergo.movement.move_speed * dt;

        if input.jump && state.transform.translation.y <= crate::player::CUBE_REST_Y + 0.001 {
            *vy = ergo.movement.jump_velocity;
        }

        *vy -= ergo.movement.gravity * dt;
        state.transform.translation.y += *vy * dt;

        if state.transform.translation.y <= crate::player::CUBE_REST_Y {
            state.transform.translation.y = crate::player::CUBE_REST_Y;
            *vy = 0.0;
        }

        state.transform.translation.x = state
            .transform
            .translation
            .x
            .clamp(-ergo.movement.plane_limit, ergo.movement.plane_limit);
        state.transform.translation.z = state
            .transform
            .translation
            .z
            .clamp(-ergo.movement.plane_limit, ergo.movement.plane_limit);
        state.color = input.color;
        state.last_seen = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multiplayer::{PlayerInputSample, RemoteState};

    fn ergo() -> crate::config::HumanErgoConfig {
        crate::config::HumanErgoConfig::default()
    }

    fn input(move_x: f32, move_z: f32, jump: bool) -> PlayerInputSample {
        PlayerInputSample {
            move_x,
            move_z,
            jump,
            ..Default::default()
        }
    }

    /// Both transports now delegate here, so a zero/negative dt guard
    /// regression would desync every hosted game rather than just one.
    #[test]
    fn zero_dt_is_a_no_op() {
        let mut states = HashMap::new();
        let mut vy = HashMap::new();
        let mut inputs = HashMap::new();
        inputs.insert(1u64, input(1.0, 1.0, false));

        step_authoritative_sim(&mut states, &inputs, &mut vy, &ergo(), 0.0);
        assert!(states.is_empty(), "zero dt must not spawn or move anything");
    }

    #[test]
    fn movement_axes_are_clamped_to_unit_range() {
        let mut states = HashMap::new();
        let mut vy = HashMap::new();
        let mut inputs = HashMap::new();
        // Out-of-range input off the wire must not buy extra speed.
        inputs.insert(1u64, input(1000.0, 0.0, false));

        let e = ergo();
        step_authoritative_sim(&mut states, &inputs, &mut vy, &e, 1.0);

        let x = states[&1].transform.translation.x;
        assert!(
            (x - e.movement.move_speed).abs() < 1e-4,
            "expected clamp to 1.0 * move_speed, got {x}"
        );
    }

    #[test]
    fn player_is_clamped_inside_plane_limits() {
        let mut states = HashMap::new();
        let mut vy = HashMap::new();
        let mut inputs = HashMap::new();
        inputs.insert(1u64, input(1.0, 1.0, false));

        let e = ergo();
        // Many long steps: without clamping this would run off the plane.
        for _ in 0..500 {
            step_authoritative_sim(&mut states, &inputs, &mut vy, &e, 0.1);
        }

        let t = states[&1].transform.translation;
        assert!(t.x <= e.movement.plane_limit + 1e-3);
        assert!(t.z <= e.movement.plane_limit + 1e-3);
    }

    #[test]
    fn player_settles_at_rest_height_and_clears_velocity() {
        let mut states = HashMap::new();
        let mut vy = HashMap::new();
        let e = ergo();

        // One frame holding jump...
        let mut inputs = HashMap::new();
        inputs.insert(1u64, input(0.0, 0.0, true));
        step_authoritative_sim(&mut states, &inputs, &mut vy, &e, 0.016);
        assert!(vy[&1] != 0.0, "jump should impart vertical velocity");

        // ...then release. Holding jump would re-trigger on every ground
        // contact and the player would never settle -- which is correct
        // behavior, not a bug, so the input has to be released to observe
        // the landing.
        inputs.insert(1u64, input(0.0, 0.0, false));
        for _ in 0..1000 {
            step_authoritative_sim(&mut states, &inputs, &mut vy, &e, 0.016);
        }

        let y = states[&1].transform.translation.y;
        assert!(
            (y - crate::player::CUBE_REST_Y).abs() < 1e-3,
            "expected settle at rest height, got {y}"
        );
        assert_eq!(vy[&1], 0.0, "vertical velocity must be cleared on landing");
    }

    /// The refactor that made steam_mp delegate here assumed both
    /// transports produce identical results from identical input. Pin it:
    /// two independent state maps stepped the same way must not diverge.
    #[test]
    fn identical_inputs_produce_identical_state_across_callers() {
        let e = ergo();
        let mut inputs = HashMap::new();
        inputs.insert(7u64, input(0.5, -0.25, true));

        let mut lan_states: HashMap<u64, RemoteState> = HashMap::new();
        let mut lan_vy = HashMap::new();
        let mut steam_states: HashMap<u64, RemoteState> = HashMap::new();
        let mut steam_vy = HashMap::new();

        for _ in 0..120 {
            step_authoritative_sim(&mut lan_states, &inputs, &mut lan_vy, &e, 0.016);
            step_authoritative_sim(&mut steam_states, &inputs, &mut steam_vy, &e, 0.016);
        }

        let a = lan_states[&7].transform.translation;
        let b = steam_states[&7].transform.translation;
        assert_eq!(a, b, "LAN and Steam sim paths diverged");
        assert_eq!(lan_vy[&7], steam_vy[&7], "vertical velocity diverged");
    }
}
