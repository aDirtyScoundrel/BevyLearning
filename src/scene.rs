use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::Skybox;
use bevy::math::Affine2;
use bevy::math::primitives::{Cone, Plane3d};
use bevy::mesh::Mesh3d;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::controls::CameraOrbitRig;
use crate::skybox::create_skybox_image;

use crate::player::{
    ChickenHead, HeadTurnDelayTimer, Beak,
    CUBE_REST_Y, spawn_player_chicken,
};
use crate::RotatingCube;

#[derive(Component)]
pub struct CameraAimCone;

const FLOOR_HALF_EXTENT: f32 = 12.0;
const FLOOR_TEXTURE_SIZE: u32 = 512;
const FLOOR_TEXTURE_REPEAT: f32 = 10.0;
const AIM_CONE_RADIUS: f32 = 0.24;
const AIM_CONE_HEIGHT: f32 = 1.0;
const AIM_CONE_OFFSET_Y: f32 = 1.8;
const SEED_PROJECTILE_SPEED: f32 = 42.0;
const SEED_PROJECTILE_LIFETIME_SECS: f32 = 1.0;
const SEED_WIDTH: f32 = 0.06;
const SEED_LENGTH: f32 = 0.12;
const PROJECTILE_HIT_RADIUS_SQ: f32 = 1.0;

const CAMERA_PIVOT_HEIGHT: f32 = 1.4;
const CAMERA_DISTANCE: f32 = 5.5;
const CAMERA_HEIGHT: f32 = 1.6;

#[derive(Component)]
pub struct ConeProjectile {
    velocity: Vec3,
    lifetime: Timer,
}

#[derive(Component)]
pub struct ReplicatedProjectileVisual;

#[derive(Resource, Default)]
pub struct ProjectileSequence {
    next_id: u32,
}

impl ProjectileSequence {
    pub fn next_id(&mut self) -> u32 {
        let projectile_id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        projectile_id
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProjectileSpawnData {
    pub projectile_id: u32,
    pub position: Vec3,
    pub velocity: Vec3,
    pub lifetime_secs: f32,
}

pub fn spawn_projectile_entity(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    position: Vec3,
    velocity: Vec3,
    lifetime_secs: f32,
) -> Entity {
    // Seed is elongated along Z axis, rotate to point in direction of travel
    let seed_rotation = if velocity.length_squared() > 0.0001 {
        Quat::from_rotation_arc(Vec3::Z, velocity.normalize())
    } else {
        Quat::IDENTITY
    };
    
    commands
        .spawn((
            Mesh3d(
                meshes.add(
                    Cuboid::new(
                        SEED_WIDTH,
                        SEED_WIDTH,
                        SEED_LENGTH,
                    )
                    .mesh()
                    .build(),
                ),
            ),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.4, 0.35, 0.25),
                emissive: LinearRgba::rgb(0.15, 0.12, 0.08),
                metallic: 0.0,
                perceptual_roughness: 0.7,
                ..default()
            })),
            Transform::from_translation(position).with_rotation(seed_rotation),
            GlobalTransform::default(),
            Visibility::default(),
            ConeProjectile {
                velocity,
                lifetime: Timer::from_seconds(lifetime_secs, TimerMode::Once),
            },
        ))
        .id()
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

    spawn_player_chicken(&mut commands, &mut meshes, &mut materials);

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
    mut queries: ParamSet<(
        Query<(&mut Transform, &mut HeadTurnDelayTimer), With<RotatingCube>>,
        Query<&mut Transform, With<CameraAimCone>>,
        Query<&mut Transform, (With<ChickenHead>, Without<CameraAimCone>, Without<RotatingCube>)>,
    )>,
    camera_query: Query<&GlobalTransform, (With<Camera3d>, Without<CameraAimCone>)>,
    body_global_query: Query<&GlobalTransform, With<RotatingCube>>,
    head_global_query: Query<&GlobalTransform, With<ChickenHead>>,
    time: Res<Time>,
) {
    let Ok(camera_transform) = camera_query.single() else {
        return;
    };

    // Get body transform
    let (body_translation, body_rotation, should_rotate) = {
        let mut body_query = queries.p0();
        let Ok((body_transform, mut timer)) = body_query.single_mut() else {
            return;
        };
        
        // Update timer - reset if angle is small, otherwise accumulate
        let body_fwd_xz = Vec3::new(body_transform.forward().x, 0.0, body_transform.forward().z).normalize_or_zero();
        let camera_fwd_xz = Vec3::new(-camera_transform.forward().x, 0.0, -camera_transform.forward().z).normalize_or_zero();
        let dot = body_fwd_xz.dot(camera_fwd_xz).clamp(-1.0, 1.0);
        let angle = dot.acos();
        
        const BODY_FOLLOW_THRESHOLD: f32 = 1.396; // 80 degrees in radians
        
        if angle > BODY_FOLLOW_THRESHOLD {
            timer.elapsed += time.delta_secs();
        } else {
            timer.elapsed = 0.0;
        }
        
        let should_rotate = timer.elapsed >= timer.delay_secs;
        (body_transform.translation, body_transform.rotation, should_rotate)
    };

    let body_global_fwd = {
        let Ok(body_global) = body_global_query.single() else {
            return;
        };
        body_global.forward()
    };

    let anchor = body_translation + Vec3::Y * AIM_CONE_OFFSET_Y;
    let camera_pos = camera_transform.translation();
    let camera_forward = camera_transform.forward();
    let plane_y = body_translation.y;

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
        let normalized_direction = direction.normalize();
        
        // Update cone
        {
            let mut cone_query = queries.p1();
            if let Ok(mut cone_transform) = cone_query.single_mut() {
                cone_transform.rotation = Quat::from_rotation_arc(Vec3::Y, normalized_direction);
                cone_transform.translation = anchor;
            }
        }
        
        // Make chicken body align with head direction (after delay)
        if should_rotate {
            let Ok(head_global) = head_global_query.single() else {
                // If no head found, fall back to camera direction
                let camera_fwd_xz = Vec3::new(-camera_forward.x, 0.0, -camera_forward.z).normalize_or_zero();
                let body_fwd_xz = Vec3::new(body_global_fwd.x, 0.0, body_global_fwd.z).normalize_or_zero();
                
                let dot = body_fwd_xz.dot(camera_fwd_xz).clamp(-1.0, 1.0);
                let angle = dot.acos();
                
                const TURN_SPEED: f32 = 3.0; // radians per second
                
                if angle > 0.01 {
                    let cross = body_fwd_xz.cross(camera_fwd_xz);
                    let rotation_direction = if cross.y > 0.0 { 1.0 } else { -1.0 };
                    let rotation_amount = angle.min(TURN_SPEED * time.delta_secs()) * rotation_direction;
                    let mut body_query = queries.p0();
                    if let Ok((mut body_transform, _)) = body_query.single_mut() {
                        body_transform.rotate_y(rotation_amount);
                    }
                }
                return;
            };
            
            // Rotate body toward head's direction
            let head_fwd_xz = Vec3::new(head_global.forward().x, 0.0, head_global.forward().z).normalize_or_zero();
            let body_fwd_xz = Vec3::new(body_global_fwd.x, 0.0, body_global_fwd.z).normalize_or_zero();
            
            // Calculate angle and rotation needed
            let dot = body_fwd_xz.dot(head_fwd_xz).clamp(-1.0, 1.0);
            let angle = dot.acos();
            
            const TURN_SPEED: f32 = 3.0; // radians per second
            
            if angle > 0.01 {
                // Determine rotation direction
                let cross = body_fwd_xz.cross(head_fwd_xz);
                let rotation_direction = if cross.y > 0.0 { 1.0 } else { -1.0 };
                
                // Rotate body toward head direction
                let rotation_amount = angle.min(TURN_SPEED * time.delta_secs()) * rotation_direction;
                let mut body_query = queries.p0();
                if let Ok((mut body_transform, _)) = body_query.single_mut() {
                    body_transform.rotate_y(rotation_amount);
                }
            }
        }
        
        // Update head to point in the same direction as the camera
        {
            let mut head_query = queries.p2();
            if let Ok(mut head_transform) = head_query.single_mut() {
                // Convert camera forward direction to body-local space (negated to face correctly)
                let local_camera_fwd = body_rotation.inverse() * (-camera_forward.as_vec3());
                // Only use horizontal component (parallel to floor) - ignore vertical look
                let local_camera_fwd_horizontal = Vec3::new(local_camera_fwd.x, 0.0, local_camera_fwd.z).normalize_or_zero();
                if local_camera_fwd_horizontal.length() > 0.0 {
                    // Head points in local +Z direction, smoothly rotate around Y axis only
                    let target_rotation = Quat::from_rotation_arc(Vec3::Z, local_camera_fwd_horizontal);
                    head_transform.rotation = head_transform.rotation.slerp(target_rotation, 0.1);
                }
            }
        }
    }
}

pub fn spawn_cone_projectile(
    mut commands: Commands,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    menu_state: Option<Res<crate::ui::EscapeMenuState>>,
    local_player: Res<crate::multiplayer::LocalPlayerId>,
    beak_query: Query<&GlobalTransform, With<Beak>>,
    head_query: Query<&GlobalTransform, With<ChickenHead>>,
    mut projectile_sequence: ResMut<ProjectileSequence>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut lan_network: Option<ResMut<crate::multiplayer::NetworkSync>>,
    mut steam_sync: Option<ResMut<crate::steam_mp::SteamSync>>,
) {
    if let Some(menu_state) = menu_state && menu_state.is_open {
        return;
    }
    if !mouse_buttons.just_pressed(MouseButton::Left) {
        return;
    }

    let Ok(beak_global) = beak_query.single() else {
        return;
    };
    let Ok(head_global) = head_query.single() else {
        return;
    };
    
    // Fire seed from beak position in direction head is facing
    let beak_transform = beak_global.compute_transform();
    let head_forward = head_global.forward();
    let spawn_position = beak_transform.translation + head_forward * 0.15;
    let spawn_data = ProjectileSpawnData {
        projectile_id: projectile_sequence.next_id(),
        position: spawn_position,
        velocity: head_forward * SEED_PROJECTILE_SPEED,
        lifetime_secs: SEED_PROJECTILE_LIFETIME_SECS,
    };
    
    spawn_projectile_entity(
        &mut commands,
        &mut meshes,
        &mut materials,
        spawn_data.position,
        spawn_data.velocity,
        spawn_data.lifetime_secs,
    );

    crate::remote_runtime::broadcast_projectile_spawn(
        local_player.value,
        &spawn_data,
        lan_network.as_deref_mut(),
        steam_sync.as_deref_mut(),
    );
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

pub fn resolve_projectile_collisions(
    mut commands: Commands,
    local_player: Res<crate::multiplayer::LocalPlayerId>,
    mut lan_network: Option<ResMut<crate::multiplayer::NetworkSync>>,
    mut steam_sync: Option<ResMut<crate::steam_mp::SteamSync>>,
    projectiles: Query<(Entity, &Transform), (With<ConeProjectile>, Without<ReplicatedProjectileVisual>)>,
    lan_remote_cubes: Query<(&Transform, &crate::multiplayer::RemoteCube)>,
    steam_remote_cubes: Query<(&Transform, &crate::steam_mp::SteamRemoteCube)>,
) {
    let mut hit_targets = std::collections::HashSet::new();

    for (projectile_entity, projectile_transform) in &projectiles {
        let projectile_pos = projectile_transform.translation;
        let hit_target = crate::remote_runtime::find_remote_hit_target(
            projectile_pos,
            PROJECTILE_HIT_RADIUS_SQ,
            &lan_remote_cubes,
            &steam_remote_cubes,
        );

        if let Some(target_id) = hit_target {
            commands.entity(projectile_entity).despawn();
            hit_targets.insert(target_id);
        }
    }

    for target_id in hit_targets {
        crate::remote_runtime::broadcast_freeze_target(
            local_player.value,
            target_id,
            lan_network.as_deref_mut(),
            steam_sync.as_deref_mut(),
        );
    }
}
