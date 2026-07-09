use bevy::input::ButtonInput;
use bevy::prelude::*;

use crate::RotationControl;

pub fn rotation_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut rotation_control: ResMut<RotationControl>,
) {
    if keyboard.just_pressed(KeyCode::Space) {
        rotation_control.paused = !rotation_control.paused;
    }

    if keyboard.just_pressed(KeyCode::KeyX) {
        rotation_control.vertical_speed = 0.0;
    }

    if keyboard.just_pressed(KeyCode::KeyR) {
        rotation_control.speed = 1.2;
    }

    const HORIZONTAL_ACCEL: f32 = 3.0;
    const MAX_HORIZONTAL_SPEED: f32 = 12.0;
    const VERTICAL_ACCEL: f32 = 3.0;
    const MAX_VERTICAL_SPEED: f32 = 12.0;
    let delta = HORIZONTAL_ACCEL * time.delta_secs();

    if keyboard.pressed(KeyCode::ArrowRight) {
        rotation_control.speed = (rotation_control.speed + delta).min(MAX_HORIZONTAL_SPEED);
    }
    if keyboard.pressed(KeyCode::ArrowLeft) {
        rotation_control.speed = (rotation_control.speed - delta).max(-MAX_HORIZONTAL_SPEED);
    }

    let vertical_delta = VERTICAL_ACCEL * time.delta_secs();
    if keyboard.pressed(KeyCode::ArrowUp) {
        rotation_control.vertical_speed =
            (rotation_control.vertical_speed + vertical_delta).min(MAX_VERTICAL_SPEED);
    }
    if keyboard.pressed(KeyCode::ArrowDown) {
        rotation_control.vertical_speed =
            (rotation_control.vertical_speed - vertical_delta).max(-MAX_VERTICAL_SPEED);
    }
}

pub fn spin_cube(
    time: Res<Time>,
    mut query: Query<&mut Transform, With<crate::RotatingCube>>,
    rotation_control: Res<crate::RotationControl>,
) {
    if rotation_control.paused {
        return;
    }
    for mut transform in &mut query {
        transform.rotate_y(time.delta_secs() * rotation_control.speed);
        transform.rotate_x(time.delta_secs() * rotation_control.vertical_speed);
    }
}
