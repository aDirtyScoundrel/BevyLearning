use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
use bevy::input_focus::{AutoFocus, FocusCause, InputFocus};
use bevy::picking::hover::Hovered;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy::text::{EditableText, EditableTextFilter, TextCursorStyle};
use bevy::ui_widgets::{observe, slider_self_update, Slider, SliderDragState, SliderRange, SliderThumb, SliderValue, TrackClick};

use crate::controls::{ControlAction, ControlBindings};
use crate::RotatingCube;
use crate::scene::{DEFAULT_CUBE_COLOR, DEFAULT_CUBE_METALLIC, DEFAULT_CUBE_ROUGHNESS};

const PANEL_WIDTH: f32 = 280.0;
const PANEL_HEIGHT: f32 = 288.0;

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