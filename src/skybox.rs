use bevy::asset::RenderAssetUsages;
use bevy::math::UVec3;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};

const SKYBOX_SIZE: u32 = 128;
const SKYBOX_FACE_COLORS: [Color; 6] = [
    Color::srgb(0.3, 0.6, 0.95),
    Color::srgb(0.95, 0.45, 0.25),
    Color::srgb(0.35, 0.9, 0.5),
    Color::srgb(0.9, 0.9, 0.35),
    Color::srgb(0.55, 0.3, 0.9),
    Color::srgb(0.15, 0.65, 0.9),
];

pub fn create_skybox_image() -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: SKYBOX_SIZE,
            height: SKYBOX_SIZE * 6,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );

    for face in 0..6 {
        let rgba = SKYBOX_FACE_COLORS[face as usize].to_srgba();
        let bytes = [
            (rgba.red * 255.0).clamp(0.0, 255.0) as u8,
            (rgba.green * 255.0).clamp(0.0, 255.0) as u8,
            (rgba.blue * 255.0).clamp(0.0, 255.0) as u8,
            (rgba.alpha * 255.0).clamp(0.0, 255.0) as u8,
        ];

        let face_y_offset = face * SKYBOX_SIZE;
        for y in 0..SKYBOX_SIZE {
            for x in 0..SKYBOX_SIZE {
                if let Ok(pixel_bytes) = image.pixel_bytes_mut(UVec3::new(x, face_y_offset + y, 0)) {
                    pixel_bytes.copy_from_slice(&bytes);
                }
            }
        }
    }

    image
        .reinterpret_stacked_2d_as_array(6)
        .expect("Skybox image should be a stacked 2D texture with 6 layers");
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image
}
