use bevy::math::primitives::{Cone, Cuboid, Cylinder, Sphere};
use bevy::mesh::Mesh3d;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;

use crate::RotatingCube;

// Components
#[derive(Component)]
pub struct ChickenHead;

#[derive(Component)]
pub struct HeadTurnDelayTimer {
    pub elapsed: f32,
    pub delay_secs: f32,
}

#[derive(Component)]
pub struct Wings {
    pub flap_timer: f32,
    pub is_flapping: bool,
}

#[derive(Component)]
pub struct ChickenBody;

#[derive(Component)]
pub struct Beak;

// Constants
pub const CHICKEN_BODY_RADIUS: f32 = 0.4;
const CHICKEN_HEAD_RADIUS: f32 = 0.2;
const CHICKEN_LEG_HALF_HEIGHT: f32 = 0.175;
const CHICKEN_CREAM: Color = Color::srgb(0.96, 0.94, 0.88);
const CHICKEN_ORANGE: Color = Color::srgb(1.0, 0.62, 0.08);
const CHICKEN_RED: Color = Color::srgb(0.88, 0.08, 0.06);
pub const CUBE_REST_Y: f32 = 0.75;
pub const DEFAULT_CUBE_COLOR: Color = Color::srgb(0.96, 0.94, 0.88);
pub const DEFAULT_CUBE_METALLIC: f32 = 0.0;
pub const DEFAULT_CUBE_ROUGHNESS: f32 = 0.85;

/// Spawns all chicken body parts as children of the given parent
pub fn spawn_chicken_parts(
    parent: &mut ChildSpawnerCommands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    // Head (with beak and comb as children)
    parent
        .spawn((
            Mesh3d(meshes.add(Sphere::new(CHICKEN_HEAD_RADIUS).mesh().uv(24, 16))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: CHICKEN_CREAM,
                metallic: 0.0,
                perceptual_roughness: 0.85,
                ..default()
            })),
            Transform::from_xyz(0.0, 0.48, -0.2),
            GlobalTransform::default(),
            Visibility::default(),
            ChickenHead,
        ))
        .with_children(|head| {
            // Beak (relative to head)
            head.spawn((
                Mesh3d(meshes.add(Cone::new(0.05, 0.16).mesh().build())),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: CHICKEN_ORANGE,
                    metallic: 0.0,
                    perceptual_roughness: 0.5,
                    ..default()
                })),
                Transform::from_xyz(0.0, 0.0, -0.34)
                    .with_rotation(Quat::from_rotation_arc(Vec3::Y, -Vec3::Z)),
                GlobalTransform::default(),
                Visibility::default(),
                Beak,
            ));

            // Comb (relative to head)
            head.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.055, 0.18, 0.07).mesh().build())),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: CHICKEN_RED,
                    metallic: 0.0,
                    perceptual_roughness: 0.7,
                    ..default()
                })),
                Transform::from_xyz(0.0, 0.2, 0.0),
                GlobalTransform::default(),
                Visibility::default(),
            ));
        });

    // Left leg
    let leg_mat = materials.add(StandardMaterial {
        base_color: CHICKEN_ORANGE,
        metallic: 0.0,
        perceptual_roughness: 0.5,
        ..default()
    });
    parent.spawn((
        Mesh3d(meshes.add(
            Cylinder::new(0.045, CHICKEN_LEG_HALF_HEIGHT).mesh().build(),
        )),
        MeshMaterial3d(leg_mat.clone()),
        Transform::from_xyz(0.12, -0.575, 0.06),
        GlobalTransform::default(),
        Visibility::default(),
    ));

    // Right leg
    parent.spawn((
        Mesh3d(meshes.add(
            Cylinder::new(0.045, CHICKEN_LEG_HALF_HEIGHT).mesh().build(),
        )),
        MeshMaterial3d(leg_mat),
        Transform::from_xyz(-0.12, -0.575, 0.06),
        GlobalTransform::default(),
        Visibility::default(),
    ));

    // Left wing
    parent.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.08, 0.16, 0.28).mesh().build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: CHICKEN_CREAM,
            metallic: 0.0,
            perceptual_roughness: 0.85,
            ..default()
        })),
        Transform::from_xyz(0.42, 0.1, 0.0).with_rotation(Quat::from_rotation_z(0.3)),
        GlobalTransform::default(),
        Visibility::default(),
        Wings {
            flap_timer: 0.0,
            is_flapping: false,
        },
    ));

    // Right wing
    parent.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.08, 0.16, 0.28).mesh().build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: CHICKEN_CREAM,
            metallic: 0.0,
            perceptual_roughness: 0.85,
            ..default()
        })),
        Transform::from_xyz(-0.42, 0.1, 0.0).with_rotation(Quat::from_rotation_z(-0.3)),
        GlobalTransform::default(),
        Visibility::default(),
        Wings {
            flap_timer: 0.0,
            is_flapping: false,
        },
    ));

    // Tail feathers
    parent.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.09, 0.32, 0.09).mesh().build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: CHICKEN_CREAM,
            metallic: 0.0,
            perceptual_roughness: 0.85,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.12, 0.44).with_rotation(Quat::from_rotation_x(-0.6)),
        GlobalTransform::default(),
        Visibility::default(),
    ));
}

/// Spawns the local player chicken with all animations
pub fn spawn_player_chicken(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    commands
        .spawn((
            Mesh3d(meshes.add(Sphere::new(CHICKEN_BODY_RADIUS).mesh().uv(32, 18))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: DEFAULT_CUBE_COLOR,
                metallic: DEFAULT_CUBE_METALLIC,
                perceptual_roughness: DEFAULT_CUBE_ROUGHNESS,
                ..default()
            })),
            Transform::from_xyz(0.0, CUBE_REST_Y, 0.0),
            GlobalTransform::default(),
            Visibility::default(),
            RotatingCube,
            ChickenBody,
            HeadTurnDelayTimer {
                elapsed: 0.0,
                delay_secs: 0.5,
            },
        ))
        .with_children(|chicken| {
            spawn_chicken_parts(chicken, meshes, materials);
        });
}

/// Detects jump input and triggers wing flapping for all chickens
pub fn flap_wings_on_jump(mut wing_query: Query<&mut Wings>, keyboard: Res<ButtonInput<KeyCode>>) {
    if keyboard.just_pressed(KeyCode::Space) {
        for mut wing in wing_query.iter_mut() {
            wing.is_flapping = true;
            wing.flap_timer = 0.0;
        }
    }
}

/// Animates wing flapping with smooth motion
pub fn animate_wing_flap(mut wing_query: Query<(&mut Transform, &mut Wings)>, time: Res<Time>) {
    const FLAP_DURATION: f32 = 0.3; // seconds for one complete flap
    const FLAP_ANGLE: f32 = 0.8; // radians

    for (mut transform, mut wing) in wing_query.iter_mut() {
        if !wing.is_flapping {
            continue;
        }

        wing.flap_timer += time.delta_secs();

        if wing.flap_timer >= FLAP_DURATION {
            wing.is_flapping = false;
            wing.flap_timer = 0.0;
            // Reset to original rotation based on wing side
            let original_angle = if transform.translation.x > 0.0 { 0.3 } else { -0.3 };
            transform.rotation = Quat::from_rotation_z(original_angle);
            continue;
        }

        // Calculate flap animation (sine wave for smooth motion)
        let progress = wing.flap_timer / FLAP_DURATION;
        let flap_rotation = (progress * std::f32::consts::PI).sin() * FLAP_ANGLE;

        // Determine wing direction
        let base_angle = if transform.translation.x > 0.0 { 0.3 } else { -0.3 };
        let side_multiplier = if transform.translation.x > 0.0 { 1.0 } else { -1.0 };

        // Apply flap on top of base rotation
        let new_angle = base_angle + (flap_rotation * side_multiplier);
        transform.rotation = Quat::from_rotation_z(new_angle);
    }
}
