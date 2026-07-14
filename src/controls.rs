use bevy::input::{mouse::AccumulatedMouseMotion, ButtonInput};
use bevy::prelude::*;


#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct NoclipState {
    pub enabled: bool,
}


#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct MovementState {
    pub vertical_velocity: f32,
}

#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct PlayerInputIntent {
    pub move_x: f32,
    pub move_z: f32,
    pub jump: bool,
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

pub fn toggle_noclip(
    keyboard: Res<ButtonInput<KeyCode>>,
    menu_state: Option<Res<crate::ui::EscapeMenuState>>,
    mut noclip: ResMut<NoclipState>,
    mut movement_state: ResMut<MovementState>,
) {
    if let Some(menu_state) = menu_state && menu_state.is_open {
        return;
    }

    if keyboard.just_pressed(KeyCode::F11) {
        noclip.enabled = !noclip.enabled;
        movement_state.vertical_velocity = 0.0;
        println!(
            "[controls] noclip {}",
            if noclip.enabled { "enabled" } else { "disabled" }
        );
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
    ToggleMachMenu,
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
    pub const ALL: [ControlAction; 13] = [
        ControlAction::TogglePause,
        ControlAction::ToggleMachMenu,
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
            ControlAction::ToggleMachMenu => "Toggle Mach menu",
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
    pub toggle_mach_menu: KeyCode,
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
            toggle_mach_menu: KeyCode::F8,
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
            ControlAction::ToggleMachMenu => self.toggle_mach_menu,
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
            ControlAction::ToggleMachMenu => self.toggle_mach_menu = key,
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

#[allow(clippy::too_many_arguments)]
pub fn move_cube(
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    bindings: Res<ControlBindings>,
    ergo: Res<crate::config::HumanErgoConfig>,
    menu_state: Option<Res<crate::ui::EscapeMenuState>>,
    mut movement_state: ResMut<MovementState>,
    noclip: Res<NoclipState>,
    mut input_intent: ResMut<PlayerInputIntent>,
    freeze: Res<MovementFreeze>,
    wad_collision: Option<Res<crate::doom_wad::WadCollisionWorld>>,
    camera_rig: Query<&CameraOrbitRig>,
    mut query: Query<&mut Transform, With<crate::RotatingCube>>,
) {
    if let Some(menu_state) = menu_state && menu_state.is_open {
        return;
    }

    let frozen = freeze.active() && !noclip.enabled;

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

    let (horizontal_movement, noclip_movement) = {
        let rig = camera_rig.single().ok();
        let yaw = rig.map(|value| value.yaw).unwrap_or_default();
        let pitch = rig.map(|value| value.pitch).unwrap_or_default();

        let facing = Quat::from_rotation_y(yaw);
        let forward_flat = facing * -Vec3::Z;
        let right_flat = facing * Vec3::X;

        let horizontal = if direction == Vec2::ZERO {
            Vec3::ZERO
        } else {
            ((right_flat * direction.x) + (forward_flat * direction.y)).normalize()
                * ergo.movement.move_speed
                * time.delta_secs()
        };

        let noclip_delta = if noclip.enabled {
            let mut fly_direction = Vec3::ZERO;
            if !frozen {
                let look = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0) * -Vec3::Z;
                if keyboard.pressed(bindings.move_forward) {
                    fly_direction += look;
                }
                if keyboard.pressed(bindings.move_backward) {
                    fly_direction -= look;
                }
                if keyboard.pressed(bindings.move_left) {
                    fly_direction -= right_flat;
                }
                if keyboard.pressed(bindings.move_right) {
                    fly_direction += right_flat;
                }
                if keyboard.pressed(bindings.jump) {
                    fly_direction += Vec3::Y;
                }
                if keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight) {
                    fly_direction -= Vec3::Y;
                }
            }

            if fly_direction.length_squared() > 0.0 {
                fly_direction.normalize() * ergo.movement.move_speed * time.delta_secs()
            } else {
                Vec3::ZERO
            }
        } else {
            Vec3::ZERO
        };

        (horizontal, noclip_delta)
    };

    let normalized_world_input = if horizontal_movement == Vec3::ZERO {
        Vec3::ZERO
    } else {
        horizontal_movement.normalize()
    };

    input_intent.move_x = normalized_world_input.x;
    input_intent.move_z = normalized_world_input.z;
    input_intent.jump = !frozen && keyboard.just_pressed(bindings.jump);

    for mut transform in &mut query {
        let player_scale = ergo.movement.player_scale.clamp(0.35, 3.0);
        let rest_y = crate::player::CUBE_REST_Y * player_scale;
        transform.scale = Vec3::splat(player_scale);

        if !noclip.enabled
            && !frozen
            && transform.translation.y <= rest_y + 0.001
            && keyboard.just_pressed(bindings.jump)
        {
            movement_state.vertical_velocity = ergo.movement.jump_velocity;
        }

        if noclip.enabled {
            movement_state.vertical_velocity = 0.0;
            transform.translation += noclip_movement;
        } else {
            movement_state.vertical_velocity -= ergo.movement.gravity * time.delta_secs();

            transform.translation += horizontal_movement;
            transform.translation.y += movement_state.vertical_velocity * time.delta_secs();

            transform.translation.x = transform
                .translation
                .x
                .clamp(-ergo.movement.plane_limit, ergo.movement.plane_limit);
            transform.translation.z = transform
                .translation
                .z
                .clamp(-ergo.movement.plane_limit, ergo.movement.plane_limit);

            if let Some(wad_collision) = &wad_collision {
                transform.translation = wad_collision.resolve_position(transform.translation);
            }

            if transform.translation.y <= rest_y {
                transform.translation.y = rest_y;
                movement_state.vertical_velocity = 0.0;
            }
        }
    }
}

pub fn mouse_look(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    accumulated_mouse_motion: Res<AccumulatedMouseMotion>,
    ergo: Res<crate::config::HumanErgoConfig>,
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

    rig.yaw -= delta.x * ergo.camera.sensitivity_x;
    rig.pitch = (rig.pitch - delta.y * ergo.camera.sensitivity_y)
        .clamp(-ergo.camera.pitch_limit, ergo.camera.pitch_limit);

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

    rig_transform.translation = cube_transform.translation + Vec3::Y * (crate::player::CUBE_REST_Y * cube_transform.scale.y);
}
