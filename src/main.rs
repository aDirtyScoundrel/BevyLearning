mod controls;
mod multiplayer;
mod scene;
mod ui;
mod skybox;
mod steam_mp;
mod player;
mod sync_codec;
mod remote_runtime;

use bevy::dev_tools::fps_overlay::{FpsOverlayConfig, FpsOverlayPlugin};
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::input_focus::tab_navigation::TabNavigationPlugin;
use bevy::prelude::*;
use bevy::ui::widget::TextShadow;
use bevy::window::{PresentMode, Window, WindowPlugin};
use std::time::Duration;
use std::fs;

#[derive(Component)]
struct RotatingCube;


#[derive(Resource, Default)]
struct ExitRequested(bool);

#[cfg(feature = "steamworks")]
fn steam_init_guard() {
    let app_id_env = std::env::var("SteamAppId").ok();
    let game_id_env = std::env::var("SteamGameId").ok();
    let app_id_file = fs::read_to_string("steam_appid.txt")
        .ok()
        .map(|s| s.trim().to_string());

    println!(
        "[steam] config SteamAppId={:?} SteamGameId={:?} steam_appid.txt={:?}",
        app_id_env,
        game_id_env,
        app_id_file
    );

    println!("[steam] runtime init delegated to steam_mp::setup_steam_sync");
}

#[cfg(not(feature = "steamworks"))]
fn steam_init_guard() {
    let app_id_env = std::env::var("SteamAppId").ok();
    let game_id_env = std::env::var("SteamGameId").ok();
    let app_id_file = fs::read_to_string("steam_appid.txt")
        .ok()
        .map(|s| s.trim().to_string());

    println!(
        "[steam] steamworks feature disabled; Steam init skipped. SteamAppId={:?} SteamGameId={:?} steam_appid.txt={:?}",
        app_id_env,
        game_id_env,
        app_id_file
    );
}

fn style_fps_overlay_shadow(
    mut commands: Commands,
    query: Query<(Entity, &Text), Without<TextShadow>>,
) {
    for (entity, text) in &query {
        // The FPS overlay label starts as "FPS: " in Bevy's built-in plugin.
        if text.0 == "FPS: " {
            commands.entity(entity).insert(TextShadow {
                offset: Vec2::new(1.0, -1.0),
                color: Color::srgba(0.0, 0.0, 0.0, 0.85),
            });
        }
    }
}

fn capture_app_exit(
    mut exit_events: MessageReader<AppExit>,
    mut exit_requested: ResMut<ExitRequested>,
) {
    if exit_events.read().next().is_some() {
        exit_requested.0 = true;
    }
}

fn add_player_controller_systems(app: &mut App) {
    app.add_systems(
        Update,
        (
            controls::mouse_look,
            controls::move_cube,
        ),
    )
    .add_systems(
        Update,
        controls::follow_cube_camera
            .after(controls::move_cube),
    )
    .add_systems(
        Update,
        controls::tick_movement_freeze.before(controls::move_cube),
    )
    .add_systems(
        Update,
        multiplayer::apply_local_freeze
            .after(multiplayer::receive_remote_states)
            .before(controls::move_cube),
    )
    .add_systems(
        Update,
        steam_mp::apply_local_freeze
            .after(steam_mp::receive_remote_states)
            .before(controls::move_cube),
    );
}

fn main() {
    steam_init_guard();
    let local_player_id = multiplayer::generate_local_player_id();

    let mut app = App::new();

    app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                present_mode: PresentMode::AutoNoVsync,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(TabNavigationPlugin)
        .add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_plugins(FpsOverlayPlugin {
            config: FpsOverlayConfig {
                text_config: TextFont::from_font_size(12.0),
                text_color: Color::WHITE,
                refresh_interval: Duration::from_millis(100),
                frame_time_graph_config: bevy::dev_tools::fps_overlay::FrameTimeGraphConfig {
                    enabled: false,
                    ..default()
                },
                ..default()
            },
        })
        .insert_resource(controls::ControlBindings::default())
        .insert_resource(controls::MovementState::default())
        .insert_resource(controls::MovementFreeze::default())
        .insert_resource(scene::ProjectileSequence::default())
        .insert_resource(ExitRequested::default())
        .insert_resource(local_player_id)
        .add_systems(
            Startup,
            (
                multiplayer::setup_network,
                steam_mp::setup_steam_sync,
                scene::setup,
                ui::setup_hud,
                ui::setup_escape_menu,
            ),
        )
        .add_systems(
            Update,
            (capture_app_exit, steam_mp::send_local_leave, multiplayer::send_local_leave).chain(),
        )
        .add_systems(
            Update,
            (
                ui::update_escape_menu,
                ui::update_hex_color_picker,
                ui::update_material_slider_visuals,
                ui::update_material_sliders,
                ui::update_player_name_stub,
                ui::update_connected_users_stub,
                steam_mp::process_callbacks,
                steam_mp::announce_local_presence,
                steam_mp::receive_remote_states,
                steam_mp::sync_remote_cubes,
                steam_mp::sync_remote_projectiles,
                steam_mp::send_local_state,
                multiplayer::announce_local_presence,
                multiplayer::receive_remote_states,
                multiplayer::sync_remote_cubes,
                multiplayer::sync_remote_projectiles,
                multiplayer::send_local_state,
                style_fps_overlay_shadow,
            ),
        );

    add_player_controller_systems(&mut app);

    app
        .add_systems(
            Update,
            (
                player::flap_wings_on_jump,
                player::animate_wing_flap,
            ),
        )
        .add_systems(
            Update,
            scene::update_camera_aim_cone.after(controls::follow_cube_camera),
        )
        .add_systems(
            Update,
            (
                scene::spawn_cone_projectile.after(scene::update_camera_aim_cone),
                scene::update_cone_projectiles,
                scene::resolve_projectile_collisions.after(scene::update_cone_projectiles),
            ),
        );

    app.run();
}


