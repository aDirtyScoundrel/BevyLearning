mod controls;
mod multiplayer;
mod scene;
mod skybox;
mod steam_mp;

use bevy::dev_tools::fps_overlay::{FpsOverlayConfig, FpsOverlayPlugin};
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::prelude::*;
use bevy::ui::widget::TextShadow;
use bevy::window::{PresentMode, Window, WindowPlugin};
use std::time::Duration;
use std::fs;

#[derive(Component)]
struct RotatingCube;

#[derive(Resource)]
struct RotationControl {
    speed: f32,
    vertical_speed: f32,
    paused: bool,
}

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

fn main() {
    steam_init_guard();
    let local_player_id = multiplayer::generate_local_player_id();

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                present_mode: PresentMode::AutoNoVsync,
                ..default()
            }),
            ..default()
        }))
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
        .insert_resource(RotationControl {
            speed: 1.2,
            vertical_speed: 0.0,
            paused: false,
        })
        .insert_resource(ExitRequested::default())
        .insert_resource(local_player_id)
        .add_systems(
            Startup,
            (multiplayer::setup_network, steam_mp::setup_steam_sync, scene::setup),
        )
        .add_systems(
            Update,
            (capture_app_exit, steam_mp::send_local_leave, multiplayer::send_local_leave).chain(),
        )
        .add_systems(
            Update,
            (
                controls::rotation_input,
                controls::spin_cube,
                steam_mp::process_callbacks,
                steam_mp::announce_local_presence,
                steam_mp::receive_remote_states,
                steam_mp::sync_remote_cubes,
                steam_mp::send_local_state,
                multiplayer::announce_local_presence,
                multiplayer::receive_remote_states,
                multiplayer::sync_remote_cubes,
                multiplayer::send_local_state,
                style_fps_overlay_shadow,
            ),
        )
        .run();
}


