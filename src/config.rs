use bevy::prelude::*;

#[derive(Resource, Debug, Clone)]
pub struct HumanErgoConfig {
    pub movement: MovementTuning,
    pub camera: CameraTuning,
    pub wing_flap: WingFlapTuning,
    pub walk_cycle: WalkCycleTuning,
    pub autoload_preset_slot: String,
}

#[derive(Debug, Clone)]
pub struct MovementTuning {
    pub move_speed: f32,
    pub jump_velocity: f32,
    pub gravity: f32,
    pub plane_limit: f32,
}

#[derive(Debug, Clone)]
pub struct CameraTuning {
    pub sensitivity_x: f32,
    pub sensitivity_y: f32,
    pub pitch_limit: f32,
}

#[derive(Debug, Clone)]
pub struct WingFlapTuning {
    pub duration_secs: f32,
    pub angle_radians: f32,
}

#[derive(Debug, Clone)]
pub struct WalkCycleTuning {
    pub cycle_rate: f32,
    pub max_swing_radians: f32,
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