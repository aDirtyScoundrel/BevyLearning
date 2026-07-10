use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::Skybox;
use bevy::math::Affine2;
use bevy::math::primitives::{Cone, Cuboid, Plane3d};
use bevy::mesh::Mesh3d;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::controls::CameraOrbitRig;
use crate::skybox::create_skybox_image;

use crate::RotatingCube;

const CUBE_EDGE: f32 = 1.5;
const FLOOR_HALF_EXTENT: f32 = 12.0;
const FLOOR_TEXTURE_SIZE: u32 = 512;
const FLOOR_TEXTURE_REPEAT: f32 = 10.0;
const AIM_CONE_RADIUS: f32 = 0.24;
const AIM_CONE_HEIGHT: f32 = 1.0;
const AIM_CONE_OFFSET_Y: f32 = 1.8;
const CONE_PROJECTILE_SPEED: f32 = 42.0;
const CONE_PROJECTILE_LIFETIME_SECS: f32 = 1.0;
const CONE_PROJECTILE_SIZE: f32 = 0.14;
const PROJECTILE_HIT_RADIUS_SQ: f32 = 1.0;
pub const CUBE_REST_Y: f32 = CUBE_EDGE * 0.5;
pub const DEFAULT_CUBE_COLOR: Color = Color::srgb(0.8, 0.3, 0.3);
pub const DEFAULT_CUBE_METALLIC: f32 = 0.2;
pub const DEFAULT_CUBE_ROUGHNESS: f32 = 0.6;

const CAMERA_PIVOT_HEIGHT: f32 = 1.4;
const CAMERA_DISTANCE: f32 = 5.5;
const CAMERA_HEIGHT: f32 = 1.6;

#[derive(Component)]
pub struct CameraAimCone;

#[derive(Component)]
pub struct ConeProjectile {
    velocity: Vec3,
    lifetime: Timer,
}

fn blend_color(base: Color, overlay: Color, strength: f32) -> Color {
    let base = base.to_srgba();
    let overlay = overlay.to_srgba();

    Color::srgba(
        base.red + (overlay.red - base.red) * strength,
        base.green + (overlay.green - base.green) * strength,
        base.blue + (overlay.blue - base.blue) * strength,
        1.0,
    )
}

fn floor_texture_color(x: u32, y: u32) -> Color {
    let minor_step = FLOOR_TEXTURE_SIZE / 48;
    let major_step = FLOOR_TEXTURE_SIZE / 12;

    let minor_grid = x % minor_step <= 1 || y % minor_step <= 1;
    let major_grid = x % major_step <= 1 || y % major_step <= 1;

    let tile_step = FLOOR_TEXTURE_SIZE / 6;
    let local_x = (x % tile_step) as f32 - tile_step as f32 * 0.5;
    let local_y = (y % tile_step) as f32 - tile_step as f32 * 0.5;
    let local_distance = (local_x * local_x + local_y * local_y).sqrt();

    let ring_spacing = tile_step as f32 / 3.0;
    let ring_width = tile_step as f32 / 18.0;
    let ring_phase = local_distance % ring_spacing;
    let ring = ring_phase <= ring_width || ring_phase >= ring_spacing - ring_width;

    let diagonal = ((local_x - local_y).abs() <= 1.2) || ((local_x + local_y).abs() <= 1.2);
    let checker_cell = FLOOR_TEXTURE_SIZE / 16;
    let checker_is_light = ((x / checker_cell) + (y / checker_cell)).is_multiple_of(2);

    let mut color = if checker_is_light {
        Color::srgb(0.11, 0.14, 0.18)
    } else {
        Color::srgb(0.08, 0.10, 0.14)
    };

    if minor_grid {
        color = blend_color(color, Color::srgb(0.20, 0.30, 0.34), 0.55);
    }

    if major_grid {
        color = blend_color(color, Color::srgb(0.68, 0.80, 0.84), 0.9);
    }

    if ring {
        color = blend_color(color, Color::srgb(0.94, 0.54, 0.22), 0.85);
    }

    if diagonal {
        color = blend_color(color, Color::srgb(0.30, 0.72, 0.82), 0.5);
    }

    color
}

fn create_floor_reference_image() -> Image {
    let mut pixel_data = Vec::with_capacity((FLOOR_TEXTURE_SIZE * FLOOR_TEXTURE_SIZE * 4) as usize);

    for y in 0..FLOOR_TEXTURE_SIZE {
        for x in 0..FLOOR_TEXTURE_SIZE {
            let rgba = floor_texture_color(x, y).to_srgba();
            pixel_data.extend_from_slice(&[
                (rgba.red * 255.0).clamp(0.0, 255.0) as u8,
                (rgba.green * 255.0).clamp(0.0, 255.0) as u8,
                (rgba.blue * 255.0).clamp(0.0, 255.0) as u8,
                (rgba.alpha * 255.0).clamp(0.0, 255.0) as u8,
            ]);
        }
    }

    let mut image = Image::new(
        Extent3d {
            width: FLOOR_TEXTURE_SIZE,
            height: FLOOR_TEXTURE_SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixel_data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    let mut sampler = ImageSamplerDescriptor::nearest();
    sampler.set_address_mode(ImageAddressMode::Repeat);
    image.sampler = ImageSampler::Descriptor(sampler);
    image
}

pub fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let skybox_handle = images.add(create_skybox_image());
    let floor_texture = images.add(create_floor_reference_image());

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
        Mesh3d(meshes.add(Cuboid::new(CUBE_EDGE, CUBE_EDGE, CUBE_EDGE).mesh().build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: DEFAULT_CUBE_COLOR,
            metallic: DEFAULT_CUBE_METALLIC,
            perceptual_roughness: DEFAULT_CUBE_ROUGHNESS,
            ..default()
        })),
        Transform::from_xyz(0.0, CUBE_REST_Y, 0.0)
            .with_rotation(Quat::from_euler(EulerRot::XYZ, 0.4, 0.7, 0.2)),
        GlobalTransform::default(),
        Visibility::default(),
        RotatingCube,
    ));

    commands.spawn((
        Mesh3d(meshes.add(Cone::new(AIM_CONE_RADIUS, AIM_CONE_HEIGHT).mesh().build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.95, 0.84, 0.24),
            emissive: LinearRgba::rgb(0.30, 0.24, 0.06),
            metallic: 0.05,
            perceptual_roughness: 0.45,
            ..default()
        })),
        Transform::from_xyz(0.0, CUBE_REST_Y + AIM_CONE_OFFSET_Y, 0.0),
        GlobalTransform::default(),
        Visibility::default(),
        CameraAimCone,
    ));

    commands
        .spawn((
            Transform::from_xyz(0.0, CUBE_REST_Y + CAMERA_PIVOT_HEIGHT, 0.0)
                .with_rotation(Quat::from_euler(EulerRot::YXZ, 0.0, -0.2, 0.0)),
            GlobalTransform::default(),
            Visibility::default(),
            CameraOrbitRig {
                yaw: 0.0,
                pitch: -0.2,
            },
        ))
        .with_children(|pivot| {
            pivot.spawn((
                Camera3d::default(),
                Transform::from_xyz(0.0, CAMERA_HEIGHT, CAMERA_DISTANCE)
                    .looking_at(Vec3::ZERO, Vec3::Y),
                GlobalTransform::default(),
                Skybox {
                    image: Some(skybox_handle),
                    brightness: 1.0,
                    ..default()
                },
            ));
        });

    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(FLOOR_HALF_EXTENT)).mesh().build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(floor_texture),
            perceptual_roughness: 0.9,
            uv_transform: Affine2::from_scale(Vec2::splat(FLOOR_TEXTURE_REPEAT)),
            ..default()
        })),
        Transform::default(),
        GlobalTransform::default(),
    ));
}

pub fn update_camera_aim_cone(
    cube_query: Query<&Transform, (With<RotatingCube>, Without<CameraAimCone>)>,
    camera_query: Query<&GlobalTransform, (With<Camera3d>, Without<CameraAimCone>)>,
    mut cone_query: Query<&mut Transform, With<CameraAimCone>>,
) {
    let Ok(cube_transform) = cube_query.single() else {
        return;
    };
    let Ok(camera_transform) = camera_query.single() else {
        return;
    };
    let Ok(mut cone_transform) = cone_query.single_mut() else {
        return;
    };

    let anchor = cube_transform.translation + Vec3::Y * AIM_CONE_OFFSET_Y;
    let camera_pos = camera_transform.translation();
    let camera_forward = camera_transform.forward();
    let plane_y = cube_transform.translation.y;

    let intersect_t = if camera_forward.y.abs() > 0.0001 {
        (plane_y - camera_pos.y) / camera_forward.y
    } else {
        -1.0
    };

    let look_point = if intersect_t > 0.0 {
        camera_pos + camera_forward * intersect_t
    } else {
        camera_pos + camera_forward * 25.0
    };

    let direction = look_point - anchor;
    if direction.length_squared() > 0.00001 {
        cone_transform.rotation = Quat::from_rotation_arc(Vec3::Y, direction.normalize());
    }
    cone_transform.translation = anchor;
}

pub fn spawn_cone_projectile(
    mut commands: Commands,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    menu_state: Option<Res<crate::ui::EscapeMenuState>>,
    cone_query: Query<&GlobalTransform, With<CameraAimCone>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if let Some(menu_state) = menu_state && menu_state.is_open {
        return;
    }
    if !mouse_buttons.just_pressed(MouseButton::Left) {
        return;
    }

    let Ok(cone_global) = cone_query.single() else {
        return;
    };
    let cone_transform = cone_global.compute_transform();
    let direction = cone_transform.rotation * Vec3::Y;
    let spawn_position = cone_transform.translation + direction * (AIM_CONE_HEIGHT * 0.5 + 0.06);

    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(
            CONE_PROJECTILE_SIZE,
            CONE_PROJECTILE_SIZE,
            CONE_PROJECTILE_SIZE,
        ).mesh().build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.78, 0.20),
            emissive: LinearRgba::rgb(0.55, 0.34, 0.08),
            metallic: 0.0,
            perceptual_roughness: 0.25,
            ..default()
        })),
        Transform::from_translation(spawn_position),
        GlobalTransform::default(),
        Visibility::default(),
        ConeProjectile {
            velocity: direction * CONE_PROJECTILE_SPEED,
            lifetime: Timer::from_seconds(CONE_PROJECTILE_LIFETIME_SECS, TimerMode::Once),
        },
    ));
}

pub fn update_cone_projectiles(
    mut commands: Commands,
    time: Res<Time>,
    mut projectiles: Query<(Entity, &mut Transform, &mut ConeProjectile)>,
) {
    for (entity, mut transform, mut projectile) in &mut projectiles {
        transform.translation += projectile.velocity * time.delta_secs();
        projectile.lifetime.tick(time.delta());

        if projectile.lifetime.is_finished() {
            commands.entity(entity).despawn();
        }
    }
}

fn projectile_hits_cube(projectile_pos: Vec3, cube_pos: Vec3) -> bool {
    projectile_pos.distance_squared(cube_pos) <= PROJECTILE_HIT_RADIUS_SQ
}

pub fn resolve_projectile_collisions(
    mut commands: Commands,
    local_player: Res<crate::multiplayer::LocalPlayerId>,
    mut lan_network: Option<ResMut<crate::multiplayer::NetworkSync>>,
    mut steam_sync: Option<ResMut<crate::steam_mp::SteamSync>>,
    projectiles: Query<(Entity, &Transform), With<ConeProjectile>>,
    lan_remote_cubes: Query<(&Transform, &crate::multiplayer::RemoteCube)>,
    steam_remote_cubes: Query<(&Transform, &crate::steam_mp::SteamRemoteCube)>,
) {
    let mut hit_targets = std::collections::HashSet::new();

    for (projectile_entity, projectile_transform) in &projectiles {
        let projectile_pos = projectile_transform.translation;
        let mut hit_target = None;

        for (remote_transform, remote_cube) in &lan_remote_cubes {
            if projectile_hits_cube(projectile_pos, remote_transform.translation) {
                hit_target = Some(remote_cube.player_id);
                break;
            }
        }

        if hit_target.is_none() {
            for (remote_transform, remote_cube) in &steam_remote_cubes {
                if projectile_hits_cube(projectile_pos, remote_transform.translation) {
                    hit_target = Some(remote_cube.player_id);
                    break;
                }
            }
        }

        if let Some(target_id) = hit_target {
            commands.entity(projectile_entity).despawn();
            hit_targets.insert(target_id);
        }
    }

    for target_id in hit_targets {
        if let Some(network) = lan_network.as_deref_mut() {
            crate::multiplayer::send_freeze_target(network, local_player.value, target_id);
        }

        if let Some(steam) = steam_sync.as_deref_mut() {
            crate::steam_mp::send_freeze_target(steam, local_player.value, target_id);
        }
    }
}
