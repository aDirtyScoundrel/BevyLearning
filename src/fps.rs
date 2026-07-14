//! FPS diagnostic text update system.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use crate::FpsText;

pub fn update_fps_text(diagnostics: Res<DiagnosticsStore>, mut query: Query<&mut Text, With<FpsText>>) {
    if let Some(measurement) = diagnostics.get_measurement(&FrameTimeDiagnosticsPlugin::FPS) {
        for mut text in &mut query {
            text.0 = format!("FPS: {:.1}", measurement.value);
        }
    }
}
