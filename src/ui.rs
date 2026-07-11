use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
use bevy::input_focus::{AutoFocus, FocusCause, InputFocus};
use bevy::picking::hover::Hovered;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy::text::{EditableText, EditableTextFilter, TextCursorStyle};
use bevy::ui_widgets::{observe, slider_self_update, Slider, SliderDragState, SliderRange, SliderThumb, SliderValue, TrackClick};
use std::fs;

use crate::controls::{ControlAction, ControlBindings};
use crate::RotatingCube;
use crate::player::{DEFAULT_CUBE_COLOR, DEFAULT_CUBE_METALLIC, DEFAULT_CUBE_ROUGHNESS};

const PANEL_WIDTH: f32 = 280.0;
const PANEL_HEIGHT: f32 = 288.0;
const ERGO_PANEL_WIDTH: f32 = 340.0;
const ERGO_PRESET_PATH: &str = "human_ergo_preset.cfg";
const ERGO_TOGGLE_KEY: KeyCode = KeyCode::F8;

#[derive(Resource)]
pub struct HudState {
    hex_code: String,
    selected_color: Color,
    metallic: f32,
    roughness: f32,
}

impl HudState {
    pub fn selected_color(&self) -> Color {
        self.selected_color
    }
}

#[derive(Component)]
pub struct HudRoot;

#[derive(Component)]
pub struct HexInputField;

#[derive(Component)]
pub struct HexInputText;

#[derive(Component)]
pub struct HexSwatch;

#[derive(Component)]
pub struct PlayerNameStub;

#[derive(Component)]
pub struct ConnectedUsersStub;

#[derive(Component)]
pub struct SteamBrowserRoot;

#[derive(Component)]
pub struct SteamBrowserStatusText;

#[derive(Component)]
pub struct SteamBrowserRowsText;

#[derive(Component)]
pub struct ErgoPanelRoot;

#[derive(Component)]
pub struct ErgoStatusText;

#[derive(Component)]
pub struct ErgoSaveButton {
    slot: ErgoPresetSlot,
}

#[derive(Component)]
pub struct ErgoLoadButton {
    slot: ErgoPresetSlot,
}

#[derive(Component, Clone, Copy)]
pub struct ErgoSlider {
    setting: ErgoSetting,
}

#[derive(Component, Clone, Copy)]
pub struct ErgoValueText {
    setting: ErgoSetting,
}

#[derive(Component)]
pub struct ErgoSliderThumb;

#[derive(Resource)]
pub struct ErgoPanelState {
    status: String,
    is_visible: bool,
}

#[derive(Clone, Copy)]
pub enum ErgoPresetSlot {
    Arcade,
    Grounded,
    Floaty,
}

impl ErgoPresetSlot {
    fn id(self) -> &'static str {
        match self {
            ErgoPresetSlot::Arcade => "arcade",
            ErgoPresetSlot::Grounded => "grounded",
            ErgoPresetSlot::Floaty => "floaty",
        }
    }

    fn label(self) -> &'static str {
        match self {
            ErgoPresetSlot::Arcade => "Arcade",
            ErgoPresetSlot::Grounded => "Grounded",
            ErgoPresetSlot::Floaty => "Floaty",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        match id.trim().to_ascii_lowercase().as_str() {
            "arcade" => Some(Self::Arcade),
            "grounded" => Some(Self::Grounded),
            "floaty" => Some(Self::Floaty),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub enum ErgoSetting {
    MoveSpeed,
    JumpVelocity,
    Gravity,
    PlaneLimit,
    CameraSensitivityX,
    CameraSensitivityY,
    CameraPitchLimit,
    WingFlapDuration,
    WingFlapAngle,
    WalkCycleRate,
    WalkCycleSwing,
    WalkCycleLift,
}

impl ErgoSetting {
    fn label(self) -> &'static str {
        match self {
            ErgoSetting::MoveSpeed => "Move speed",
            ErgoSetting::JumpVelocity => "Jump velocity",
            ErgoSetting::Gravity => "Gravity",
            ErgoSetting::PlaneLimit => "Plane limit",
            ErgoSetting::CameraSensitivityX => "Sensitivity X",
            ErgoSetting::CameraSensitivityY => "Sensitivity Y",
            ErgoSetting::CameraPitchLimit => "Pitch limit",
            ErgoSetting::WingFlapDuration => "Flap duration",
            ErgoSetting::WingFlapAngle => "Flap angle",
            ErgoSetting::WalkCycleRate => "Cycle rate",
            ErgoSetting::WalkCycleSwing => "Max swing",
            ErgoSetting::WalkCycleLift => "Lift amount",
        }
    }

    fn range(self) -> (f32, f32) {
        match self {
            ErgoSetting::MoveSpeed => (1.0, 12.0),
            ErgoSetting::JumpVelocity => (1.0, 15.0),
            ErgoSetting::Gravity => (1.0, 30.0),
            ErgoSetting::PlaneLimit => (3.0, 50.0),
            ErgoSetting::CameraSensitivityX => (0.0005, 0.02),
            ErgoSetting::CameraSensitivityY => (0.0005, 0.02),
            ErgoSetting::CameraPitchLimit => (0.3, 1.55),
            ErgoSetting::WingFlapDuration => (0.05, 1.2),
            ErgoSetting::WingFlapAngle => (0.1, 2.2),
            ErgoSetting::WalkCycleRate => (1.0, 20.0),
            ErgoSetting::WalkCycleSwing => (0.05, 1.8),
            ErgoSetting::WalkCycleLift => (0.0, 0.25),
        }
    }

    fn value(self, ergo: &crate::config::HumanErgoConfig) -> f32 {
        match self {
            ErgoSetting::MoveSpeed => ergo.movement.move_speed,
            ErgoSetting::JumpVelocity => ergo.movement.jump_velocity,
            ErgoSetting::Gravity => ergo.movement.gravity,
            ErgoSetting::PlaneLimit => ergo.movement.plane_limit,
            ErgoSetting::CameraSensitivityX => ergo.camera.sensitivity_x,
            ErgoSetting::CameraSensitivityY => ergo.camera.sensitivity_y,
            ErgoSetting::CameraPitchLimit => ergo.camera.pitch_limit,
            ErgoSetting::WingFlapDuration => ergo.wing_flap.duration_secs,
            ErgoSetting::WingFlapAngle => ergo.wing_flap.angle_radians,
            ErgoSetting::WalkCycleRate => ergo.walk_cycle.cycle_rate,
            ErgoSetting::WalkCycleSwing => ergo.walk_cycle.max_swing_radians,
            ErgoSetting::WalkCycleLift => ergo.walk_cycle.lift_amount,
        }
    }

    fn set_value(self, ergo: &mut crate::config::HumanErgoConfig, raw_value: f32) {
        let (min, max) = self.range();
        let value = raw_value.clamp(min, max);

        match self {
            ErgoSetting::MoveSpeed => ergo.movement.move_speed = value,
            ErgoSetting::JumpVelocity => ergo.movement.jump_velocity = value,
            ErgoSetting::Gravity => ergo.movement.gravity = value,
            ErgoSetting::PlaneLimit => ergo.movement.plane_limit = value,
            ErgoSetting::CameraSensitivityX => ergo.camera.sensitivity_x = value,
            ErgoSetting::CameraSensitivityY => ergo.camera.sensitivity_y = value,
            ErgoSetting::CameraPitchLimit => ergo.camera.pitch_limit = value,
            ErgoSetting::WingFlapDuration => ergo.wing_flap.duration_secs = value,
            ErgoSetting::WingFlapAngle => ergo.wing_flap.angle_radians = value,
            ErgoSetting::WalkCycleRate => ergo.walk_cycle.cycle_rate = value,
            ErgoSetting::WalkCycleSwing => ergo.walk_cycle.max_swing_radians = value,
            ErgoSetting::WalkCycleLift => ergo.walk_cycle.lift_amount = value,
        }
    }
}

#[derive(Component, Clone, Copy)]
pub struct MaterialSlider {
    property: MaterialProperty,
}

#[derive(Component, Clone, Copy)]
pub struct MaterialValueText {
    property: MaterialProperty,
}

#[derive(Component)]
pub struct MaterialSliderThumb;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MaterialProperty {
    Reflectiveness,
    Roughness,
}

impl MaterialProperty {
    fn label(self) -> &'static str {
        match self {
            MaterialProperty::Reflectiveness => "Reflectiveness",
            MaterialProperty::Roughness => "Roughness",
        }
    }
}

#[derive(Resource, Default)]
pub struct EscapeMenuState {
    pub is_open: bool,
    awaiting_action: Option<ControlAction>,
}

#[derive(Component)]
pub struct EscapeMenuRoot;

#[derive(Component)]
pub struct EscapeExitButton;

#[derive(Component)]
pub struct EscapeDefaultsButton;

#[derive(Component, Clone, Copy)]
pub struct RebindButton {
    action: ControlAction,
}

#[derive(Component, Clone, Copy)]
pub struct RebindLabel {
    action: ControlAction,
}

#[derive(Component)]
pub struct EscapeMenuStatusText;

pub fn setup_hud(mut commands: Commands) {
    let initial_color = parse_hex_color("CC4D4D").unwrap_or(DEFAULT_CUBE_COLOR);

    commands.insert_resource(HudState {
        hex_code: "CC4D4D".to_string(),
        selected_color: initial_color,
        metallic: DEFAULT_CUBE_METALLIC,
        roughness: DEFAULT_CUBE_ROUGHNESS,
    });

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(20.0),
                bottom: Val::Px(20.0),
                width: Val::Px(PANEL_WIDTH),
                height: Val::Px(PANEL_HEIGHT),
                padding: UiRect::all(Val::Px(14.0)),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexStart,
                align_items: AlignItems::Stretch,
                row_gap: Val::Px(10.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(16.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.07, 0.09, 0.88)),
            BorderColor::all(Color::srgba(0.7, 0.8, 1.0, 0.16)),
            HudRoot,
            TabGroup::default(),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Appearance"),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgba(0.96, 0.97, 0.99, 0.92)),
            ));

            parent.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(62.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::FlexStart,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(10.0),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.12, 0.13, 0.17, 0.96)),
                BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.06)),
                HexInputField,
            ))
            .with_children(|row| {
                row.spawn((
                    Node {
                        width: Val::Px(34.0),
                        height: Val::Px(34.0),
                        border_radius: BorderRadius::all(Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(initial_color),
                    HexSwatch,
                ));

                row.spawn((
                    Node {
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::FlexStart,
                        row_gap: Val::Px(2.0),
                        ..default()
                    },
                ))
                .with_children(|column| {
                    column.spawn((
                        Text::new("Hex color"),
                        TextFont::from_font_size(11.0),
                        TextColor(Color::srgba(0.8, 0.83, 0.88, 0.78)),
                    ));
                    column.spawn((
                        Node {
                            width: Val::Px(176.0),
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(10.0)),
                            padding: UiRect::horizontal(Val::Px(8.0)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        EditableText {
                            max_characters: Some(6),
                            ..EditableText::new("D94A4A")
                        },
                        EditableTextFilter::new(|c| c.is_ascii_hexdigit()),
                        TextCursorStyle::default(),
                        TextLayout::no_wrap(),
                        TextFont::from_font_size(16.0),
                        TextColor(Color::WHITE),
                        TabIndex(0),
                        AutoFocus,
                        BackgroundColor(Color::srgba(0.12, 0.13, 0.17, 0.96)),
                        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.06)),
                        HexInputText,
                    ))
                    .observe(|mut click: On<Pointer<Click>>, mut focus: ResMut<InputFocus>| {
                        focus.set(click.entity, FocusCause::Pressed);
                        click.propagate(false);
                    });
                });

                row.spawn((
                    Text::new("Click and type"),
                    TextFont::from_font_size(10.0),
                    TextColor(Color::srgba(0.78, 0.82, 0.9, 0.6)),
                ));
            });

            parent.spawn((
                Text::new("Material"),
                TextFont::from_font_size(12.0),
                TextColor(Color::srgba(0.9, 0.92, 0.95, 0.86)),
            ));

            parent.spawn(material_slider_row(
                MaterialProperty::Reflectiveness,
                DEFAULT_CUBE_METALLIC,
            ));

            parent.spawn(material_slider_row(
                MaterialProperty::Roughness,
                DEFAULT_CUBE_ROUGHNESS,
            ));

            parent.spawn((
                Text::new("Player name"),
                TextFont::from_font_size(12.0),
                TextColor(Color::srgba(0.9, 0.92, 0.95, 0.86)),
                PlayerNameStub,
            ));

            parent.spawn((
                Text::new("Connected users"),
                TextFont::from_font_size(12.0),
                TextColor(Color::srgba(0.9, 0.92, 0.95, 0.86)),
                ConnectedUsersStub,
            ));
        });
}

pub fn setup_escape_menu(mut commands: Commands, bindings: Res<ControlBindings>) {
    commands.insert_resource(EscapeMenuState::default());

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.72)),
            EscapeMenuRoot,
            TabGroup::default(),
        ))
        .with_children(|overlay| {
            overlay
                .spawn((
                    Node {
                        width: Val::Px(520.0),
                        max_width: Val::Percent(92.0),
                        padding: UiRect::all(Val::Px(20.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(16.0)),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(12.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.08, 0.1, 0.14, 0.95)),
                    BorderColor::all(Color::srgba(0.66, 0.82, 1.0, 0.26)),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new("Escape Menu"),
                        TextFont::from_font_size(24.0),
                        TextColor(Color::WHITE),
                    ));

                    panel.spawn((
                        Text::new("Press Escape to close. Click a control button, then press a key to rebind."),
                        TextFont::from_font_size(12.0),
                        TextColor(Color::srgba(0.8, 0.86, 0.94, 0.9)),
                    ));

                    panel.spawn((
                        Text::new("Ready"),
                        TextFont::from_font_size(12.0),
                        TextColor(Color::srgba(0.7, 0.92, 0.78, 0.95)),
                        EscapeMenuStatusText,
                    ));

                    panel
                        .spawn((
                            Node {
                                width: Val::Percent(100.0),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(8.0),
                                ..default()
                            },
                        ))
                        .with_children(|list| {
                            for action in ControlAction::ALL {
                                list.spawn((
                                    Node {
                                        width: Val::Percent(100.0),
                                        justify_content: JustifyContent::SpaceBetween,
                                        align_items: AlignItems::Center,
                                        ..default()
                                    },
                                    children![
                                        (
                                            Text::new(action.label()),
                                            TextFont::from_font_size(14.0),
                                            TextColor(Color::srgba(0.94, 0.96, 0.99, 0.92)),
                                        ),
                                        (
                                            Button,
                                            Node {
                                                width: Val::Px(156.0),
                                                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                                                border: UiRect::all(Val::Px(1.0)),
                                                justify_content: JustifyContent::Center,
                                                align_items: AlignItems::Center,
                                                border_radius: BorderRadius::all(Val::Px(10.0)),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(0.14, 0.16, 0.23, 0.96)),
                                            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.1)),
                                            RebindButton { action },
                                            children![
                                                (
                                                    Text::new(key_label(bindings.key_for(action))),
                                                    TextFont::from_font_size(13.0),
                                                    TextColor(Color::WHITE),
                                                    RebindLabel { action },
                                                )
                                            ],
                                        )
                                    ],
                                ));
                            }
                        });

                    panel.spawn((
                        Button,
                        Node {
                            width: Val::Percent(100.0),
                            margin: UiRect::top(Val::Px(4.0)),
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(10.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(Val::Px(12.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.18, 0.26, 0.16, 0.96)),
                        BorderColor::all(Color::srgba(0.76, 0.96, 0.74, 0.4)),
                        EscapeDefaultsButton,
                        children![
                            (
                                Text::new("Reset Controls To Defaults"),
                                TextFont::from_font_size(14.0),
                                TextColor(Color::WHITE),
                            )
                        ],
                    ));

                    panel.spawn((
                        Button,
                        Node {
                            width: Val::Percent(100.0),
                            margin: UiRect::top(Val::Px(8.0)),
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(10.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(Val::Px(12.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.55, 0.15, 0.14, 0.96)),
                        BorderColor::all(Color::srgba(1.0, 0.78, 0.76, 0.4)),
                        EscapeExitButton,
                        children![
                            (
                                Text::new("Exit Game"),
                                TextFont::from_font_size(15.0),
                                TextColor(Color::WHITE),
                            )
                        ],
                    ));
                });
        });
}

pub fn setup_steam_server_browser(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Px(20.0),
                width: Val::Px(420.0),
                max_height: Val::Percent(40.0),
                overflow: Overflow::scroll_y(),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.07, 0.1, 0.13, 0.88)),
            BorderColor::all(Color::srgba(0.66, 0.82, 1.0, 0.24)),
            SteamBrowserRoot,
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Steam Server Browser"),
                TextFont::from_font_size(14.0),
                TextColor(Color::WHITE),
            ));
            panel.spawn((
                Text::new("F6 refresh, Up/Down select, F7 join"),
                TextFont::from_font_size(11.0),
                TextColor(Color::srgba(0.82, 0.88, 0.96, 0.9)),
                SteamBrowserStatusText,
            ));
            panel.spawn((
                Text::new("No servers yet."),
                TextFont::from_font_size(11.0),
                TextColor(Color::srgba(0.9, 0.93, 0.97, 0.95)),
                SteamBrowserRowsText,
            ));
        });
}

pub fn setup_ergo_panel(mut commands: Commands, mut ergo: ResMut<crate::config::HumanErgoConfig>) {
    let startup_slot = ErgoPresetSlot::from_id(&ergo.autoload_preset_slot)
        .unwrap_or(ErgoPresetSlot::Grounded);

    let startup_status = match load_ergo_preset(startup_slot) {
        Ok(loaded) => {
            *ergo = loaded;
            format!(
                "Auto-loaded {} preset from {}",
                startup_slot.label(),
                ERGO_PRESET_PATH
            )
        }
        Err(_) => format!("Ready (toggle with {:?})", ERGO_TOGGLE_KEY),
    };

    commands.insert_resource(ErgoPanelState {
        status: startup_status,
        is_visible: true,
    });

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                bottom: Val::Px(20.0),
                width: Val::Px(ERGO_PANEL_WIDTH),
                max_height: Val::Percent(92.0),
                overflow: Overflow::scroll_y(),
                padding: UiRect::all(Val::Px(14.0)),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                row_gap: Val::Px(8.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(14.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.08, 0.1, 0.12, 0.9)),
            BorderColor::all(Color::srgba(0.6, 0.8, 0.96, 0.22)),
            ErgoPanelRoot,
            TabGroup::default(),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Human Ergo Tuning"),
                TextFont::from_font_size(16.0),
                TextColor(Color::WHITE),
            ));

            panel.spawn((
                Text::new("Movement"),
                TextFont::from_font_size(12.0),
                TextColor(Color::srgba(0.88, 0.91, 0.95, 0.9)),
            ));

            panel.spawn(ergo_slider_row(ErgoSetting::MoveSpeed, ergo.movement.move_speed));
            panel.spawn(ergo_slider_row(ErgoSetting::JumpVelocity, ergo.movement.jump_velocity));
            panel.spawn(ergo_slider_row(ErgoSetting::Gravity, ergo.movement.gravity));
            panel.spawn(ergo_slider_row(ErgoSetting::PlaneLimit, ergo.movement.plane_limit));

            panel.spawn((
                Text::new("Camera"),
                TextFont::from_font_size(12.0),
                TextColor(Color::srgba(0.88, 0.91, 0.95, 0.9)),
            ));

            panel.spawn(ergo_slider_row(
                ErgoSetting::CameraSensitivityX,
                ergo.camera.sensitivity_x,
            ));
            panel.spawn(ergo_slider_row(
                ErgoSetting::CameraSensitivityY,
                ergo.camera.sensitivity_y,
            ));
            panel.spawn(ergo_slider_row(
                ErgoSetting::CameraPitchLimit,
                ergo.camera.pitch_limit,
            ));

            panel.spawn((
                Text::new("Animation"),
                TextFont::from_font_size(12.0),
                TextColor(Color::srgba(0.88, 0.91, 0.95, 0.9)),
            ));

            panel.spawn(ergo_slider_row(
                ErgoSetting::WingFlapDuration,
                ergo.wing_flap.duration_secs,
            ));
            panel.spawn(ergo_slider_row(
                ErgoSetting::WingFlapAngle,
                ergo.wing_flap.angle_radians,
            ));
            panel.spawn(ergo_slider_row(
                ErgoSetting::WalkCycleRate,
                ergo.walk_cycle.cycle_rate,
            ));
            panel.spawn(ergo_slider_row(
                ErgoSetting::WalkCycleSwing,
                ergo.walk_cycle.max_swing_radians,
            ));
            panel.spawn(ergo_slider_row(
                ErgoSetting::WalkCycleLift,
                ergo.walk_cycle.lift_amount,
            ));

            panel.spawn((
                Text::new("Presets"),
                TextFont::from_font_size(12.0),
                TextColor(Color::srgba(0.88, 0.91, 0.95, 0.9)),
            ));

            panel.spawn(preset_slot_row(ErgoPresetSlot::Arcade));
            panel.spawn(preset_slot_row(ErgoPresetSlot::Grounded));
            panel.spawn(preset_slot_row(ErgoPresetSlot::Floaty));

            panel.spawn((
                Text::new(""),
                TextFont::from_font_size(11.0),
                TextColor(Color::srgba(0.78, 0.84, 0.9, 0.9)),
                ErgoStatusText,
            ));
        });
}

#[allow(clippy::type_complexity)]
pub fn update_hex_color_picker(
    input_focus: Res<InputFocus>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut hud: ResMut<HudState>,
    mut ui_parts: ParamSet<(
        Query<(Entity, &mut BackgroundColor, &mut BorderColor), With<HexInputField>>,
        Query<&mut BackgroundColor, With<HexSwatch>>,
    )>,
    mut text_inputs: Query<&mut EditableText, With<HexInputText>>,
    mut cube_materials: ResMut<Assets<StandardMaterial>>,
    local_cube_material: Query<&MeshMaterial3d<StandardMaterial>, With<RotatingCube>>,
) {
    if let Ok((field_entity, mut field_bg, mut field_border)) = ui_parts.p0().single_mut() {
        let is_focused = input_focus.get() == Some(field_entity);

        *field_border = BorderColor::all(if is_focused {
            Color::srgba(0.45, 0.74, 1.0, 0.95)
        } else {
            Color::srgba(1.0, 1.0, 1.0, 0.06)
        });

        *field_bg = BackgroundColor(if is_focused {
            Color::srgba(0.16, 0.18, 0.23, 0.98)
        } else {
            Color::srgba(0.12, 0.13, 0.17, 0.96)
        });
    }

    if !keyboard_input.just_pressed(KeyCode::Enter) {
        return;
    }

    let Some(focused_entity) = input_focus.get() else {
        return;
    };

    let Ok(editable_text) = text_inputs.get_mut(focused_entity) else {
        return;
    };

    let typed_hex = editable_text
        .value()
        .to_string()
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .take(6)
        .map(|character| character.to_ascii_uppercase())
        .collect::<String>();

    if typed_hex.is_empty() {
        return;
    }

    let mut normalized_hex = typed_hex;
    while normalized_hex.len() < 6 {
        normalized_hex.push('0');
    }

    hud.hex_code = normalized_hex.clone();

    if let Some(color) = parse_hex_color(&normalized_hex) {
        hud.selected_color = color;
        apply_local_material(&hud, &mut ui_parts, &mut cube_materials, &local_cube_material);
    }
}

#[allow(clippy::type_complexity)]
pub fn update_material_sliders(
    mut hud: ResMut<HudState>,
    changed_sliders: Query<(&SliderValue, &MaterialSlider), Changed<SliderValue>>,
    mut value_text: Query<(&mut Text, &MaterialValueText)>,
    mut ui_parts: ParamSet<(
        Query<(Entity, &mut BackgroundColor, &mut BorderColor), With<HexInputField>>,
        Query<&mut BackgroundColor, With<HexSwatch>>,
    )>,
    mut cube_materials: ResMut<Assets<StandardMaterial>>,
    local_cube_material: Query<&MeshMaterial3d<StandardMaterial>, With<RotatingCube>>,
) {
    let mut changed = false;

    for (slider_value, slider) in &changed_sliders {
        let value = slider_value.0.clamp(0.0, 1.0);
        match slider.property {
            MaterialProperty::Reflectiveness => hud.metallic = value,
            MaterialProperty::Roughness => hud.roughness = value,
        }
        changed = true;
    }

    for (mut text, label) in &mut value_text {
        let value = match label.property {
            MaterialProperty::Reflectiveness => hud.metallic,
            MaterialProperty::Roughness => hud.roughness,
        };
        text.0 = format!("{value:.2}");
    }

    if changed {
        apply_local_material(&hud, &mut ui_parts, &mut cube_materials, &local_cube_material);
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_escape_menu(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut menu_state: ResMut<EscapeMenuState>,
    mut menu_root: Query<&mut Node, With<EscapeMenuRoot>>,
    mut button_interactions: Query<
        (
            &Interaction,
            Option<&EscapeExitButton>,
            Option<&EscapeDefaultsButton>,
            Option<&RebindButton>,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        (Changed<Interaction>, With<Button>),
    >,
    mut status_text: Query<&mut Text, (With<EscapeMenuStatusText>, Without<RebindLabel>)>,
    mut key_labels: Query<(&mut Text, &RebindLabel), Without<EscapeMenuStatusText>>,
    mut bindings: ResMut<ControlBindings>,
    mut app_exit: MessageWriter<AppExit>,
) {
    if keyboard_input.just_pressed(KeyCode::Escape) {
        if menu_state.awaiting_action.is_some() {
            menu_state.awaiting_action = None;
        } else {
            menu_state.is_open = !menu_state.is_open;
        }
    }

    if let Ok(mut root_node) = menu_root.single_mut() {
        root_node.display = if menu_state.is_open {
            Display::Flex
        } else {
            Display::None
        };
    }

    if menu_state.is_open {
        for (interaction, exit_button, defaults_button, rebind_button, mut bg, mut border) in &mut button_interactions {
            match *interaction {
                Interaction::Pressed => {
                    if exit_button.is_some() {
                        app_exit.write(AppExit::Success);
                    }

                    if defaults_button.is_some() {
                        *bindings = ControlBindings::default();
                        menu_state.awaiting_action = None;
                    }

                    if let Some(button) = rebind_button {
                        menu_state.awaiting_action = Some(button.action);
                    }

                    *bg = BackgroundColor(Color::srgba(0.24, 0.32, 0.45, 0.98));
                    *border = BorderColor::all(Color::srgba(0.74, 0.86, 1.0, 0.95));
                }
                Interaction::Hovered => {
                    *bg = BackgroundColor(Color::srgba(0.18, 0.22, 0.31, 0.98));
                    *border = BorderColor::all(Color::srgba(0.76, 0.86, 0.98, 0.65));
                }
                Interaction::None => {
                    *bg = BackgroundColor(Color::srgba(0.14, 0.16, 0.23, 0.96));
                    *border = BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.1));
                }
            }
        }

        if let Some(action) = menu_state.awaiting_action
            && let Some(&pressed_key) = keyboard_input.get_just_pressed().next()
        {
            if pressed_key == KeyCode::Escape {
                menu_state.awaiting_action = None;
            } else {
                bindings.set_key(action, pressed_key);
                menu_state.awaiting_action = None;
            }
        }
    }

    if let Ok(mut status) = status_text.single_mut() {
        status.0 = if let Some(action) = menu_state.awaiting_action {
            format!("Press a key for {}", action.label())
        } else if menu_state.is_open {
            "Ready (Defaults button uses ControlBindings::default())".to_string()
        } else {
            "".to_string()
        };
    }

    for (mut text, label) in &mut key_labels {
        text.0 = key_label(bindings.key_for(label.action));
    }
}

pub fn update_player_name_stub() {}

pub fn update_connected_users_stub() {}

pub fn update_steam_server_browser_ui(
    browser: Option<Res<crate::steam_mp::SteamBrowserView>>,
    mut status_text: Query<&mut Text, With<SteamBrowserStatusText>>,
    mut rows_text: Query<&mut Text, With<SteamBrowserRowsText>>,
) {
    let Some(browser) = browser else {
        return;
    };

    if let Ok(mut status) = status_text.single_mut() {
        status.0 = if let Some(selected) = browser.selected_index {
            format!("{} | selected: {}", browser.status, selected + 1)
        } else {
            browser.status.clone()
        };
    }

    if let Ok(mut rows) = rows_text.single_mut() {
        rows.0 = if browser.rows.is_empty() {
            "No servers found. Press F6 to refresh.".to_string()
        } else {
            browser.rows.join("\n")
        };
    }
}

#[allow(clippy::type_complexity)]
pub fn update_ergo_panel(
    mut commands: Commands,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut ergo: ResMut<crate::config::HumanErgoConfig>,
    mut panel_state: ResMut<ErgoPanelState>,
    mut panel_root: Query<&mut Node, With<ErgoPanelRoot>>,
    changed_sliders: Query<(&SliderValue, &ErgoSlider), Changed<SliderValue>>,
    slider_values: Query<(Entity, &SliderValue, &ErgoSlider)>,
    mut value_texts: Query<(&mut Text, &ErgoValueText)>,
    mut status_text: Query<&mut Text, With<ErgoStatusText>>,
    mut buttons: Query<
        (
            &Interaction,
            Option<&ErgoSaveButton>,
            Option<&ErgoLoadButton>,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        (Changed<Interaction>, With<Button>),
    >,
) {
    for (slider_value, slider) in &changed_sliders {
        slider.setting.set_value(&mut ergo, slider_value.0);
    }

    for (entity, slider_value, slider) in &slider_values {
        let desired = slider.setting.value(&ergo);
        if (slider_value.0 - desired).abs() > f32::EPSILON {
            commands.entity(entity).insert(SliderValue(desired));
        }
    }

    for (mut text, label) in &mut value_texts {
        text.0 = format!("{:.3}", label.setting.value(&ergo));
    }

    for (interaction, save_button, load_button, mut bg, mut border) in &mut buttons {
        if save_button.is_none() && load_button.is_none() {
            continue;
        }

        match *interaction {
            Interaction::Pressed => {
                if let Some(save_button) = save_button {
                    panel_state.status = match save_ergo_preset(save_button.slot, &ergo) {
                        Ok(()) => format!("Saved {} preset", save_button.slot.label()),
                        Err(error) => format!("Save failed: {error}"),
                    };
                }

                if let Some(load_button) = load_button {
                    panel_state.status = match load_ergo_preset(load_button.slot) {
                        Ok(loaded) => {
                            *ergo = loaded;
                            format!("Loaded {} preset", load_button.slot.label())
                        }
                        Err(error) => format!("Load failed: {error}"),
                    };
                }

                *bg = BackgroundColor(Color::srgba(0.24, 0.32, 0.45, 0.98));
                *border = BorderColor::all(Color::srgba(0.74, 0.86, 1.0, 0.95));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.18, 0.22, 0.31, 0.98));
                *border = BorderColor::all(Color::srgba(0.76, 0.86, 0.98, 0.65));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.14, 0.16, 0.23, 0.96));
                *border = BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.1));
            }
        }
    }

    if keyboard_input.just_pressed(ERGO_TOGGLE_KEY) {
        panel_state.is_visible = !panel_state.is_visible;
    }

    if let Ok(mut root_node) = panel_root.single_mut() {
        root_node.display = if panel_state.is_visible {
            Display::Flex
        } else {
            Display::None
        };
    }

    if let Ok(mut status) = status_text.single_mut() {
        status.0 = format!("{} | Toggle: {:?}", panel_state.status, ERGO_TOGGLE_KEY);
    }
}

fn parse_hex_color(hex: &str) -> Option<Color> {
    if hex.len() != 6 {
        return None;
    }

    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;

    Some(Color::srgb(
        red as f32 / 255.0,
        green as f32 / 255.0,
        blue as f32 / 255.0,
    ))
}

fn key_label(key: KeyCode) -> String {
    format!("{:?}", key)
}

fn material_slider_row(property: MaterialProperty, initial_value: f32) -> impl Bundle {
    (
        Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            ..default()
        },
        children![
            (
                Node {
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    ..default()
                },
                children![
                    (
                        Text::new(property.label()),
                        TextFont::from_font_size(11.0),
                        TextColor(Color::srgba(0.82, 0.86, 0.9, 0.84)),
                    ),
                    (
                        Text::new(format!("{initial_value:.2}")),
                        TextFont::from_font_size(11.0),
                        TextColor(Color::WHITE),
                        MaterialValueText { property },
                    )
                ],
            ),
            (
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Stretch,
                    height: Val::Px(12.0),
                    width: Val::Percent(100.0),
                    ..default()
                },
                MaterialSlider { property },
                Hovered::default(),
                Slider {
                    track_click: TrackClick::Snap,
                    ..Default::default()
                },
                SliderValue(initial_value),
                SliderRange::new(0.0, 1.0),
                observe(slider_self_update),
                children![
                    (
                        Node {
                            height: Val::Px(6.0),
                            border_radius: BorderRadius::all(Val::Px(3.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.2, 0.22, 0.28, 0.96)),
                    ),
                    (
                        Node {
                            display: Display::Flex,
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            right: Val::Px(12.0),
                            top: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            ..default()
                        },
                        children![
                            (
                                SliderThumb,
                                MaterialSliderThumb,
                                Node {
                                    display: Display::Flex,
                                    width: Val::Px(12.0),
                                    height: Val::Px(12.0),
                                    position_type: PositionType::Absolute,
                                    left: Val::Percent(0.0),
                                    border_radius: BorderRadius::MAX,
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.72, 0.88, 1.0, 1.0)),
                            )
                        ],
                    )
                ],
            )
        ],
    )
}

#[allow(clippy::type_complexity)]
pub fn update_material_slider_visuals(
    sliders: Query<
        (Entity, &SliderValue, &SliderRange, &Hovered, &SliderDragState),
        (
            Or<(
                Changed<SliderValue>,
                Changed<Hovered>,
                Changed<SliderDragState>,
            )>,
            With<MaterialSlider>,
        ),
    >,
    children: Query<&Children>,
    mut thumbs: Query<(&mut Node, &mut BackgroundColor, Has<MaterialSliderThumb>), Without<MaterialSlider>>,
) {
    for (slider_entity, value, range, hovered, drag_state) in &sliders {
        let position = range.thumb_position(value.0) * 100.0;

        for child in children.iter_descendants(slider_entity) {
            if let Ok((mut thumb_node, mut thumb_bg, is_thumb)) = thumbs.get_mut(child)
                && is_thumb
            {
                thumb_node.left = Val::Percent(position);
                thumb_bg.0 = if hovered.0 || drag_state.dragging {
                    Color::srgba(0.88, 0.97, 1.0, 1.0)
                } else {
                    Color::srgba(0.72, 0.88, 1.0, 1.0)
                };
            }
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn update_ergo_slider_visuals(
    sliders: Query<
        (Entity, &SliderValue, &SliderRange, &Hovered, &SliderDragState),
        (
            Or<(
                Changed<SliderValue>,
                Changed<Hovered>,
                Changed<SliderDragState>,
            )>,
            With<ErgoSlider>,
        ),
    >,
    children: Query<&Children>,
    mut thumbs: Query<(&mut Node, &mut BackgroundColor, Has<ErgoSliderThumb>), Without<ErgoSlider>>,
) {
    for (slider_entity, value, range, hovered, drag_state) in &sliders {
        let position = range.thumb_position(value.0) * 100.0;

        for child in children.iter_descendants(slider_entity) {
            if let Ok((mut thumb_node, mut thumb_bg, is_thumb)) = thumbs.get_mut(child)
                && is_thumb
            {
                thumb_node.left = Val::Percent(position);
                thumb_bg.0 = if hovered.0 || drag_state.dragging {
                    Color::srgba(0.93, 0.99, 1.0, 1.0)
                } else {
                    Color::srgba(0.8, 0.9, 1.0, 1.0)
                };
            }
        }
    }
}

fn ergo_slider_row(setting: ErgoSetting, initial_value: f32) -> impl Bundle {
    let (min, max) = setting.range();

    (
        Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(5.0),
            ..default()
        },
        children![
            (
                Node {
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    ..default()
                },
                children![
                    (
                        Text::new(setting.label()),
                        TextFont::from_font_size(11.0),
                        TextColor(Color::srgba(0.84, 0.88, 0.92, 0.88)),
                    ),
                    (
                        Text::new(format!("{initial_value:.3}")),
                        TextFont::from_font_size(11.0),
                        TextColor(Color::WHITE),
                        ErgoValueText { setting },
                    )
                ],
            ),
            (
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Stretch,
                    height: Val::Px(12.0),
                    width: Val::Percent(100.0),
                    ..default()
                },
                ErgoSlider { setting },
                Hovered::default(),
                Slider {
                    track_click: TrackClick::Snap,
                    ..Default::default()
                },
                SliderValue(initial_value),
                SliderRange::new(min, max),
                observe(slider_self_update),
                children![
                    (
                        Node {
                            height: Val::Px(6.0),
                            border_radius: BorderRadius::all(Val::Px(3.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.2, 0.22, 0.28, 0.96)),
                    ),
                    (
                        Node {
                            display: Display::Flex,
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            right: Val::Px(12.0),
                            top: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            ..default()
                        },
                        children![(
                            SliderThumb,
                            ErgoSliderThumb,
                            Node {
                                display: Display::Flex,
                                width: Val::Px(12.0),
                                height: Val::Px(12.0),
                                position_type: PositionType::Absolute,
                                left: Val::Percent(0.0),
                                border_radius: BorderRadius::MAX,
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.8, 0.9, 1.0, 1.0)),
                        )],
                    )
                ],
            )
        ],
    )
}

fn preset_slot_row(slot: ErgoPresetSlot) -> impl Bundle {
    (
        Node {
            width: Val::Percent(100.0),
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.0),
            ..default()
        },
        children![
            (
                Text::new(slot.label()),
                TextFont::from_font_size(11.0),
                TextColor(Color::srgba(0.84, 0.88, 0.92, 0.88)),
            ),
            (
                Node {
                    column_gap: Val::Px(8.0),
                    ..default()
                },
                children![
                    (
                        Button,
                        Node {
                            width: Val::Px(96.0),
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(Val::Px(10.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.14, 0.2, 0.16, 0.96)),
                        BorderColor::all(Color::srgba(0.76, 0.96, 0.74, 0.34)),
                        ErgoSaveButton { slot },
                        children![(
                            Text::new("Save"),
                            TextFont::from_font_size(11.0),
                            TextColor(Color::WHITE),
                        )],
                    ),
                    (
                        Button,
                        Node {
                            width: Val::Px(96.0),
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(Val::Px(10.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.15, 0.18, 0.24, 0.96)),
                        BorderColor::all(Color::srgba(0.78, 0.84, 1.0, 0.34)),
                        ErgoLoadButton { slot },
                        children![(
                            Text::new("Load"),
                            TextFont::from_font_size(11.0),
                            TextColor(Color::WHITE),
                        )],
                    )
                ],
            )
        ],
    )
}

fn save_ergo_preset(slot: ErgoPresetSlot, ergo: &crate::config::HumanErgoConfig) -> Result<(), String> {
    let slot_key = slot.id();
    let mut existing = fs::read_to_string(ERGO_PRESET_PATH).unwrap_or_default();

    let mut lines: Vec<String> = existing
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with(&format!("{slot_key}."))
        })
        .map(ToString::to_string)
        .collect();

    lines.push(format!("{slot_key}.move_speed={}", ergo.movement.move_speed));
    lines.push(format!("{slot_key}.jump_velocity={}", ergo.movement.jump_velocity));
    lines.push(format!("{slot_key}.gravity={}", ergo.movement.gravity));
    lines.push(format!("{slot_key}.plane_limit={}", ergo.movement.plane_limit));
    lines.push(format!("{slot_key}.camera_sensitivity_x={}", ergo.camera.sensitivity_x));
    lines.push(format!("{slot_key}.camera_sensitivity_y={}", ergo.camera.sensitivity_y));
    lines.push(format!("{slot_key}.camera_pitch_limit={}", ergo.camera.pitch_limit));
    lines.push(format!("{slot_key}.wing_flap_duration={}", ergo.wing_flap.duration_secs));
    lines.push(format!("{slot_key}.wing_flap_angle={}", ergo.wing_flap.angle_radians));
    lines.push(format!("{slot_key}.walk_cycle_rate={}", ergo.walk_cycle.cycle_rate));
    lines.push(format!("{slot_key}.walk_cycle_swing={}", ergo.walk_cycle.max_swing_radians));
    lines.push(format!("{slot_key}.walk_cycle_lift={}", ergo.walk_cycle.lift_amount));

    existing = lines.join("\n");
    fs::write(ERGO_PRESET_PATH, existing).map_err(|error| error.to_string())
}

fn load_ergo_preset(slot: ErgoPresetSlot) -> Result<crate::config::HumanErgoConfig, String> {
    let raw = fs::read_to_string(ERGO_PRESET_PATH).map_err(|error| error.to_string())?;
    let slot_key = slot.id();
    let mut loaded = crate::config::HumanErgoConfig::default();
    let mut any_loaded = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Some((key, value_raw)) = trimmed.split_once('=') else {
            continue;
        };

        let Ok(value) = value_raw.trim().parse::<f32>() else {
            continue;
        };

        let key = key.trim();
        let key = if let Some((prefix, scoped_key)) = key.split_once('.') {
            if prefix != slot_key {
                continue;
            }
            scoped_key
        } else {
            // Backward compatibility with older single-preset files.
            if !matches!(slot, ErgoPresetSlot::Grounded) {
                continue;
            }
            key
        };

        match key {
            "move_speed" => loaded.movement.move_speed = value,
            "jump_velocity" => loaded.movement.jump_velocity = value,
            "gravity" => loaded.movement.gravity = value,
            "plane_limit" => loaded.movement.plane_limit = value,
            "camera_sensitivity_x" => loaded.camera.sensitivity_x = value,
            "camera_sensitivity_y" => loaded.camera.sensitivity_y = value,
            "camera_pitch_limit" => loaded.camera.pitch_limit = value,
            "wing_flap_duration" => loaded.wing_flap.duration_secs = value,
            "wing_flap_angle" => loaded.wing_flap.angle_radians = value,
            "walk_cycle_rate" => loaded.walk_cycle.cycle_rate = value,
            "walk_cycle_swing" => loaded.walk_cycle.max_swing_radians = value,
            "walk_cycle_lift" => loaded.walk_cycle.lift_amount = value,
            _ => {}
        }
        any_loaded = true;
    }

    if !any_loaded {
        return Err(format!("Preset '{}' not found in {}", slot.label(), ERGO_PRESET_PATH));
    }

    Ok(loaded)
}

#[allow(clippy::type_complexity)]
fn apply_local_material(
    hud: &HudState,
    ui_parts: &mut ParamSet<(
        Query<(Entity, &mut BackgroundColor, &mut BorderColor), With<HexInputField>>,
        Query<&mut BackgroundColor, With<HexSwatch>>,
    )>,
    cube_materials: &mut Assets<StandardMaterial>,
    local_cube_material: &Query<&MeshMaterial3d<StandardMaterial>, With<RotatingCube>>,
) {
    if let Ok(mut swatch_color) = ui_parts.p1().single_mut() {
        swatch_color.0 = hud.selected_color;
    }

    if let Ok(material_handle) = local_cube_material.single()
        && let Some(mut material) = cube_materials.get_mut(&material_handle.0)
    {
        material.base_color = hud.selected_color;
        material.metallic = hud.metallic;
        material.perceptual_roughness = hud.roughness;
    }
}