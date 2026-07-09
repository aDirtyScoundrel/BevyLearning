use bevy::light::Skybox;
use bevy::mesh::Mesh3d;
use bevy::math::primitives::{Cuboid, Plane3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;

use crate::skybox::create_skybox_image;

use crate::RotatingCube;

pub fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let skybox_handle = images.add(create_skybox_image());

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 1.8, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
        GlobalTransform::default(),
        Skybox {
            image: Some(skybox_handle),
            brightness: 1.0,
            ..default()
        },
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 10_000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.8, 0.6, 0.0)),
        GlobalTransform::default(),
    ));

    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(1.5, 1.5, 1.5).mesh().build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.8, 0.3, 0.3),
            metallic: 0.2,
            perceptual_roughness: 0.6,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.75, 0.0)
            .with_rotation(Quat::from_euler(EulerRot::XYZ, 0.4, 0.7, 0.2)),
        GlobalTransform::default(),
        RotatingCube,
    ));

    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(10.0)).mesh().build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.15, 0.15, 0.2),
            perceptual_roughness: 0.9,
            ..default()
        })),
        Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        GlobalTransform::default(),
    ));
}
