//! Runtime tuning parameters surfaced through the Ergo panel and preset files.
//!
//! [`HumanErgoConfig`] is the single source of truth for all movement physics
//! constants, camera sensitivity, animation rates, and the active preset slot.

use bevy::prelude::*;

/// Top-level resource holding all per-session tuning values.
#[derive(Resource, Debug, Clone)]
pub struct HumanErgoConfig {
    pub movement: MovementTuning,
    pub camera: CameraTuning,
    pub wing_flap: WingFlapTuning,
    pub walk_cycle: WalkCycleTuning,
    /// Name of the preset slot loaded automatically on startup.
    pub autoload_preset_slot: String,
}

/// Physics constants for player movement and world bounds.
#[derive(Debug, Clone)]
pub struct MovementTuning {
    /// Horizontal movement speed in world-units per second.
    pub move_speed: f32,
    /// Vertical impulse applied when jumping (world-units/s).
    pub jump_velocity: f32,
    /// Downward acceleration applied each frame (world-units/s²).
    pub gravity: f32,
    /// Half-extent of the playfield square; player is clamped to ±this value.
    pub plane_limit: f32,
    /// Uniform scale applied to the player chicken entity.
    pub player_scale: f32,
}

/// Mouse look sensitivity and vertical-pitch clamp.
#[derive(Debug, Clone)]
pub struct CameraTuning {
    /// Radians rotated per pixel of horizontal mouse movement.
    pub sensitivity_x: f32,
    /// Radians rotated per pixel of vertical mouse movement.
    pub sensitivity_y: f32,
    /// Maximum pitch away from horizontal (clamped symmetrically up and down).
    pub pitch_limit: f32,
}

/// Wing-flap animation parameters.
#[derive(Debug, Clone)]
pub struct WingFlapTuning {
    /// Total duration of one flap cycle in seconds.
    pub duration_secs: f32,
    /// Peak wing rotation angle in radians at the apex of the flap.
    pub angle_radians: f32,
}

/// Procedural walk-cycle animation parameters.
#[derive(Debug, Clone)]
pub struct WalkCycleTuning {
    /// How fast the walk phase advances per world-unit of horizontal travel.
    pub cycle_rate: f32,
    /// Maximum leg swing angle in radians at full stride.
    pub max_swing_radians: f32,
    /// How much each leg rises off the ground at the peak of its swing.
    pub lift_amount: f32,
}

impl Default for HumanErgoConfig {
    fn default() -> Self {
        Self {
            movement: MovementTuning {
                move_speed: 4.0,
                jump_velocity: 5.5,
                gravity: 9.81,
                plane_limit: 9.5,
                player_scale: 1.0,
            },
            camera: CameraTuning {
                sensitivity_x: 0.003,
                sensitivity_y: 0.0025,
                pitch_limit: std::f32::consts::FRAC_PI_2 - 0.1,
            },
            wing_flap: WingFlapTuning {
                duration_secs: 0.3,
                angle_radians: 0.8,
            },
            walk_cycle: WalkCycleTuning {
                cycle_rate: 9.0,
                max_swing_radians: 0.55,
                lift_amount: 0.045,
            },
            autoload_preset_slot: "grounded".to_string(),
        }
    }
}