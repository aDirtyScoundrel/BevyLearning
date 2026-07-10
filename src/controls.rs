use bevy::input::{mouse::AccumulatedMouseMotion, ButtonInput};
use bevy::prelude::*;

use crate::RotationControl;

#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct MovementState {
    pub vertical_velocity: f32,
}

#[derive(Resource, Debug, Default, Clone)]
pub struct MovementFreeze {
    timer: Option<Timer>,
}

impl MovementFreeze {
    pub fn activate_for(&mut self, duration_secs: f32) {
        self.timer = Some(Timer::from_seconds(duration_secs, TimerMode::Once));
    }

    pub fn active(&self) -> bool {
        self.timer
            .as_ref()
            .is_some_and(|timer| !timer.is_finished())
    }
}

pub fn tick_movement_freeze(time: Res<Time>, mut freeze: ResMut<MovementFreeze>) {
    if let Some(timer) = freeze.timer.as_mut() {
        timer.tick(time.delta());
        if timer.is_finished() {
            freeze.timer = None;
        }
    }
}

#[derive(Component, Debug, Clone, Copy)]
pub struct CameraOrbitRig {
    pub yaw: f32,
    pub pitch: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlAction {
    TogglePause,
    ResetVertical,
    ResetSpeed,
    Jump,
    MoveForward,
    MoveBackward,
    MoveLeft,
    MoveRight,
    TurnRight,
    TurnLeft,
    PitchUp,
    PitchDown,
}

impl ControlAction {
    pub const ALL: [ControlAction; 12] = [
        ControlAction::TogglePause,
        ControlAction::ResetVertical,
        ControlAction::ResetSpeed,
        ControlAction::Jump,
        ControlAction::MoveForward,
        ControlAction::MoveBackward,
        ControlAction::MoveLeft,
        ControlAction::MoveRight,
        ControlAction::TurnRight,
        ControlAction::TurnLeft,
        ControlAction::PitchUp,
        ControlAction::PitchDown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ControlAction::TogglePause => "Toggle pause",
            ControlAction::ResetVertical => "Reset vertical speed",
            ControlAction::ResetSpeed => "Reset horizontal speed",
            ControlAction::Jump => "Jump",
            ControlAction::MoveForward => "Move forward",
            ControlAction::MoveBackward => "Move backward",
            ControlAction::MoveLeft => "Move left",
            ControlAction::MoveRight => "Move right",
            ControlAction::TurnRight => "Turn right",
            ControlAction::TurnLeft => "Turn left",
            ControlAction::PitchUp => "Pitch up",
            ControlAction::PitchDown => "Pitch down",
        }
    }
}

#[derive(Resource, Debug, Clone)]
pub struct ControlBindings {
    pub toggle_pause: KeyCode,
    pub reset_vertical: KeyCode,
    pub reset_speed: KeyCode,
    pub jump: KeyCode,
    pub move_forward: KeyCode,
    pub move_backward: KeyCode,
    pub move_left: KeyCode,
    pub move_right: KeyCode,
    pub turn_right: KeyCode,
    pub turn_left: KeyCode,
    pub pitch_up: KeyCode,
    pub pitch_down: KeyCode,
}

impl Default for ControlBindings {
    fn default() -> Self {
        Self {
            toggle_pause: KeyCode::KeyF,
            reset_vertical: KeyCode::KeyX,
            reset_speed: KeyCode::KeyR,
            jump: KeyCode::Space,
            move_forward: KeyCode::KeyW,
            move_backward: KeyCode::KeyS,
            move_left: KeyCode::KeyA,
            move_right: KeyCode::KeyD,
            turn_right: KeyCode::ArrowRight,
            turn_left: KeyCode::ArrowLeft,
            pitch_up: KeyCode::ArrowUp,
            pitch_down: KeyCode::ArrowDown,
        }
    }
}

impl ControlBindings {
    pub fn key_for(&self, action: ControlAction) -> KeyCode {
        match action {
            ControlAction::TogglePause => self.toggle_pause,
            ControlAction::ResetVertical => self.reset_vertical,
            ControlAction::ResetSpeed => self.reset_speed,
            ControlAction::Jump => self.jump,
            ControlAction::MoveForward => self.move_forward,
            ControlAction::MoveBackward => self.move_backward,
            ControlAction::MoveLeft => self.move_left,
            ControlAction::MoveRight => self.move_right,
            ControlAction::TurnRight => self.turn_right,
            ControlAction::TurnLeft => self.turn_left,
            ControlAction::PitchUp => self.pitch_up,
            ControlAction::PitchDown => self.pitch_down,
        }
    }

    pub fn set_key(&mut self, action: ControlAction, key: KeyCode) {
        match action {
            ControlAction::TogglePause => self.toggle_pause = key,
            ControlAction::ResetVertical => self.reset_vertical = key,
            ControlAction::ResetSpeed => self.reset_speed = key,
            ControlAction::Jump => self.jump = key,
            ControlAction::MoveForward => self.move_forward = key,
            ControlAction::MoveBackward => self.move_backward = key,
            ControlAction::MoveLeft => self.move_left = key,
            ControlAction::MoveRight => self.move_right = key,
            ControlAction::TurnRight => self.turn_right = key,
            ControlAction::TurnLeft => self.turn_left = key,
            ControlAction::PitchUp => self.pitch_up = key,
            ControlAction::PitchDown => self.pitch_down = key,
        }
    }
}

pub fn rotation_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    bindings: Res<ControlBindings>,
    menu_state: Option<Res<crate::ui::EscapeMenuState>>,
    mut rotation_control: ResMut<RotationControl>,
) {
    if let Some(menu_state) = menu_state && menu_state.is_open {
        return;
    }

    if keyboard.just_pressed(bindings.toggle_pause) {
        rotation_control.paused = !rotation_control.paused;
    }

    if keyboard.just_pressed(bindings.reset_vertical) {
        rotation_control.vertical_speed = 0.0;
    }

    if keyboard.just_pressed(bindings.reset_speed) {
        rotation_control.speed = 1.2;
    }

    const HORIZONTAL_ACCEL: f32 = 3.0;
    const MAX_HORIZONTAL_SPEED: f32 = 12.0;
    const VERTICAL_ACCEL: f32 = 3.0;
    const MAX_VERTICAL_SPEED: f32 = 12.0;
    let delta = HORIZONTAL_ACCEL * time.delta_secs();

    if keyboard.pressed(bindings.turn_right) {
        rotation_control.speed = (rotation_control.speed + delta).min(MAX_HORIZONTAL_SPEED);
    }
    if keyboard.pressed(bindings.turn_left) {
        rotation_control.speed = (rotation_control.speed - delta).max(-MAX_HORIZONTAL_SPEED);
    }

    let vertical_delta = VERTICAL_ACCEL * time.delta_secs();
    if keyboard.pressed(bindings.pitch_up) {
        rotation_control.vertical_speed =
            (rotation_control.vertical_speed + vertical_delta).min(MAX_VERTICAL_SPEED);
    }
    if keyboard.pressed(bindings.pitch_down) {
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

#[allow(clippy::too_many_arguments)]
pub fn move_cube(
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    bindings: Res<ControlBindings>,
    menu_state: Option<Res<crate::ui::EscapeMenuState>>,
    mut movement_state: ResMut<MovementState>,
    freeze: Res<MovementFreeze>,
    camera_rig: Query<&CameraOrbitRig>,
    mut query: Query<&mut Transform, With<crate::RotatingCube>>,
) {
    if let Some(menu_state) = menu_state && menu_state.is_open {
        return;
    }

    const MOVE_SPEED: f32 = 4.0;
    const PLANE_LIMIT: f32 = 9.5;
    const GRAVITY: f32 = 9.81;
    const JUMP_VELOCITY: f32 = 5.5;

    let frozen = freeze.active();

    let mut direction = Vec2::ZERO;
    if !frozen {
        if keyboard.pressed(bindings.move_forward) {
            direction.y += 1.0;
        }
        if keyboard.pressed(bindings.move_backward) {
            direction.y -= 1.0;
        }
        if keyboard.pressed(bindings.move_left) {
            direction.x -= 1.0;
        }
        if keyboard.pressed(bindings.move_right) {
            direction.x += 1.0;
        }
    }

    let horizontal_movement = if direction == Vec2::ZERO {
        Vec3::ZERO
    } else {
        let yaw = camera_rig
            .single()
            .map(|rig| rig.yaw)
            .unwrap_or_default();
        let facing = Quat::from_rotation_y(yaw);
        let forward = facing * -Vec3::Z;
        let right = facing * Vec3::X;
        ((right * direction.x) + (forward * direction.y)).normalize() * MOVE_SPEED * time.delta_secs()
    };

    for mut transform in &mut query {
        if !frozen
            && transform.translation.y <= crate::scene::CUBE_REST_Y + 0.001
            && keyboard.just_pressed(bindings.jump)
        {
            movement_state.vertical_velocity = JUMP_VELOCITY;
        }

        movement_state.vertical_velocity -= GRAVITY * time.delta_secs();

        transform.translation += horizontal_movement;
        transform.translation.y += movement_state.vertical_velocity * time.delta_secs();

        transform.translation.x = transform.translation.x.clamp(-PLANE_LIMIT, PLANE_LIMIT);
        transform.translation.z = transform.translation.z.clamp(-PLANE_LIMIT, PLANE_LIMIT);

        if transform.translation.y <= crate::scene::CUBE_REST_Y {
            transform.translation.y = crate::scene::CUBE_REST_Y;
            movement_state.vertical_velocity = 0.0;
        }
    }
}

pub fn mouse_look(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    accumulated_mouse_motion: Res<AccumulatedMouseMotion>,
    menu_state: Option<Res<crate::ui::EscapeMenuState>>,
    mut rig_query: Query<(&mut Transform, &mut CameraOrbitRig)>,
) {
    if let Some(menu_state) = menu_state && menu_state.is_open {
        return;
    }

    if !mouse_buttons.pressed(MouseButton::Right) {
        return;
    }

    let delta = accumulated_mouse_motion.delta;
    if delta == Vec2::ZERO {
        return;
    }

    let Ok((mut transform, mut rig)) = rig_query.single_mut() else {
        return;
    };

    const SENSITIVITY_X: f32 = 0.003;
    const SENSITIVITY_Y: f32 = 0.0025;
    const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.1;

    rig.yaw -= delta.x * SENSITIVITY_X;
    rig.pitch = (rig.pitch - delta.y * SENSITIVITY_Y).clamp(-PITCH_LIMIT, PITCH_LIMIT);

    transform.rotation = Quat::from_euler(EulerRot::YXZ, rig.yaw, rig.pitch, 0.0);
}

pub fn follow_cube_camera(
    cube: Query<&Transform, (With<crate::RotatingCube>, Without<CameraOrbitRig>)>,
    mut camera_rig: Query<&mut Transform, (With<CameraOrbitRig>, Without<crate::RotatingCube>)>,
) {
    let Ok(cube_transform) = cube.single() else {
        return;
    };
    let Ok(mut rig_transform) = camera_rig.single_mut() else {
        return;
    };

    rig_transform.translation = cube_transform.translation + Vec3::Y * crate::scene::CUBE_REST_Y;
}
