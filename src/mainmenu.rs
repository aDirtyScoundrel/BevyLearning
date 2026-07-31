//! Top-level application state (title screen vs. in-game) and the title
//! screen UI itself.
//!
//! `Startup` systems that assume the game world exists (network setup,
//! scene spawn, HUD, escape menu, ...) are gated behind `OnEnter(AppState::InGame)`
//! rather than running unconditionally at process start — see `main.rs`.

use bevy::prelude::*;

/// Top-level app state. Everything gameplay/network-related is scoped to
/// `InGame`; `MainMenu` is a lightweight title screen with no game world.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppState {
    #[default]
    MainMenu,
    InGame,
}

#[derive(Component)]
pub struct MainMenuRoot;

#[derive(Component)]
pub struct SinglePlayerButton;

#[derive(Component)]
pub struct QuitButton;

pub fn setup_main_menu(mut commands: Commands) {
    commands.spawn((Camera3d::default(), MainMenuRoot));

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(18.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.06, 0.09, 1.0)),
            MainMenuRoot,
        ))
        .with_children(|root| {
            root.spawn((
                Text::new("Learning"),
                TextFont::from_font_size(42.0),
                TextColor(Color::WHITE),
            ));

            root.spawn((
                Text::new("A chicken multiplayer experiment"),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgba(0.8, 0.86, 0.94, 0.85)),
            ));

            root.spawn((
                Button,
                Node {
                    width: Val::Px(240.0),
                    margin: UiRect::top(Val::Px(24.0)),
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(12.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.16, 0.24, 0.4, 0.96)),
                BorderColor::all(Color::srgba(0.66, 0.82, 1.0, 0.4)),
                SinglePlayerButton,
                children![(
                    Text::new("Single Player"),
                    TextFont::from_font_size(16.0),
                    TextColor(Color::WHITE),
                )],
            ));

            root.spawn((
                Button,
                Node {
                    width: Val::Px(240.0),
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(12.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.18, 0.14, 0.14, 0.96)),
                BorderColor::all(Color::srgba(1.0, 0.7, 0.68, 0.35)),
                QuitButton,
                children![(
                    Text::new("Quit"),
                    TextFont::from_font_size(16.0),
                    TextColor(Color::WHITE),
                )],
            ));
        });
}

pub fn update_main_menu(
    mut commands: Commands,
    mut next_state: ResMut<NextState<AppState>>,
    mut app_exit: MessageWriter<AppExit>,
    mut interactions: Query<
        (
            &Interaction,
            Option<&SinglePlayerButton>,
            Option<&QuitButton>,
            &mut BackgroundColor,
        ),
        (Changed<Interaction>, With<Button>),
    >,
) {
    for (interaction, single_player, quit, mut bg) in &mut interactions {
        match *interaction {
            Interaction::Pressed => {
                if single_player.is_some() {
                    match crate::singleplayer::spawn_embedded_server(&mut commands) {
                        Ok(()) => next_state.set(AppState::InGame),
                        Err(err) => {
                            eprintln!("[singleplayer] failed to spawn embedded server: {}", err);
                        }
                    }
                }
                if quit.is_some() {
                    app_exit.write(AppExit::Success);
                }
            }
            Interaction::Hovered => {
                *bg = if single_player.is_some() {
                    BackgroundColor(Color::srgba(0.22, 0.32, 0.5, 0.98))
                } else {
                    BackgroundColor(Color::srgba(0.26, 0.18, 0.18, 0.98))
                };
            }
            Interaction::None => {
                *bg = if single_player.is_some() {
                    BackgroundColor(Color::srgba(0.16, 0.24, 0.4, 0.96))
                } else {
                    BackgroundColor(Color::srgba(0.18, 0.14, 0.14, 0.96))
                };
            }
        }
    }
}

/// Removes the title screen's camera and UI when leaving `MainMenu` — the
/// in-game scene sets up its own camera in `scene::setup`.
pub fn teardown_main_menu(mut commands: Commands, query: Query<Entity, With<MainMenuRoot>>) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
}
