use bevy::prelude::*;
use std::collections::HashMap;
use std::time::Instant;

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
