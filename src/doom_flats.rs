//! Decode static Doom flats. Wall patches and palette effects are separate.
use std::collections::BTreeMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

pub const MAX_LUMP_NAMES: usize = 65_536;
const FLAT_PIXELS: usize = 4096;
const PALETTE_BYTES: usize = 768;

pub fn flat_image(indices: &[u8], playpal: &[u8]) -> Result<Image, String> {
    if indices.len() != FLAT_PIXELS {
        return Err("A classic flat must contain 4096 pixels".into());
    }
    let palette = playpal
        .get(..PALETTE_BYTES)
        .ok_or_else(|| "PLAYPAL must contain at least 768 bytes".to_owned())?;
    let mut pixels = Vec::with_capacity(FLAT_PIXELS * 4);
    for &index in indices {
        let start = usize::from(index) * 3;
        let rgb = palette
            .get(start..start + 3)
            .ok_or_else(|| "Palette index is outside PLAYPAL".to_owned())?;
        pixels.extend_from_slice(rgb);
        pixels.push(255);
    }
    let mut image = Image::new(
        Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    let mut sampler = ImageSamplerDescriptor::nearest();
    sampler.set_address_mode(ImageAddressMode::Repeat);
    image.sampler = ImageSampler::Descriptor(sampler);
    Ok(image)
}

/// Select directory indices inside flat namespaces. Later definitions win.
pub fn flat_indices(names: &[&str]) -> Result<BTreeMap<String, usize>, String> {
    if names.len() > MAX_LUMP_NAMES {
        return Err("WAD exceeds the flat directory scan limit".into());
    }
    let mut indices = BTreeMap::new();
    let mut depth = 0usize;
    for (index, name) in names.iter().enumerate() {
        // Directory names arrive through lossy UTF-8 decoding, so a WAD with
        // high-bit lump names yields non-ASCII or overlong strings. Such a
        // name can never be a valid flat; skip it instead of rejecting the
        // whole map.
        if name.len() > 8 || !name.is_ascii() {
            continue;
        }
        let name = name.to_ascii_uppercase();
        match name.as_str() {
            "F_START" | "FF_START" => {
                depth += 1;
                if depth > 16 {
                    return Err("Flat namespaces exceed the nesting limit".into());
                }
            }
            "F_END" | "FF_END" => depth = depth.saturating_sub(1),
            "F_SKY1" | "" => {}
            _ if depth > 0 => {
                // Numbered markers group resources within the outer namespace.
                let bytes = name.as_bytes();
                let numbered = bytes.first() == Some(&b'F')
                    && bytes.get(1).is_some_and(|b| (b'1'..=b'9').contains(b))
                    && matches!(bytes.get(2..), Some(b"_START" | b"_END"));
                if !numbered {
                    indices.insert(name, index);
                }
            }
            _ => {}
        }
    }
    // An unclosed F_START degrades: keep everything collected so far. Real
    // community WADs occasionally omit the closing marker, and rejecting the
    // whole map contradicts the load-maps-as-is goal. The nesting cap above
    // still guards against pathological directories.
    Ok(indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::image::ImageFilterMode;

    #[test]
    fn pixels_preserve_palette_indices_row_order_and_opacity() {
        let mut palette = Vec::new();
        for n in 0..=255u8 {
            palette.extend_from_slice(&[n, 255 - n, n / 2]);
        }
        palette.extend_from_slice(&vec![17; 768]);
        let mut indices = vec![0; 4096];
        indices[1] = 255;
        indices[64] = 73;
        indices[4095] = 8;
        let image = flat_image(&indices, &palette).unwrap();
        let data = image.data.as_ref().unwrap();
        assert_eq!(data.len(), 16384);
        for (i, &index) in indices.iter().enumerate() {
            assert_eq!(
                &data[i * 4..i * 4 + 4],
                &[index, 255 - index, index / 2, 255]
            );
        }
        assert_eq!(
            image.texture_descriptor.size,
            Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1
            }
        );
        assert_eq!(
            image.texture_descriptor.format,
            TextureFormat::Rgba8UnormSrgb
        );
        let ImageSampler::Descriptor(sampler) = image.sampler else {
            panic!("Missing sampler")
        };
        assert_eq!(sampler.address_mode_u, ImageAddressMode::Repeat);
        assert_eq!(sampler.address_mode_v, ImageAddressMode::Repeat);
        assert_eq!(sampler.mag_filter, ImageFilterMode::Nearest);
        assert_eq!(sampler.min_filter, ImageFilterMode::Nearest);
    }

    #[test]
    fn invalid_image_sizes_fail_without_partial_output() {
        for size in [0, 4095, 4097] {
            assert!(flat_image(&vec![0; size], &[0; 768]).is_err());
        }
        assert!(flat_image(&[0; 4096], &[0; 767]).is_err());
    }

    #[test]
    fn namespaces_preserve_directory_indices_and_last_definition() {
        let names = [
            "OUTSIDE", "F_START", "F1_START", "flata", "F1_END", "FLATB", "F_END", "FLATA",
            "FF_START", "FLATA", "F_SKY1", "FF_END",
        ];
        let found = flat_indices(&names).unwrap();
        assert_eq!(
            found,
            BTreeMap::from([("FLATA".into(), 9), ("FLATB".into(), 5)])
        );
        assert!(flat_indices(&["FLATA"]).unwrap().is_empty());
    }

    #[test]
    fn nested_outer_markers_keep_the_remaining_flats() {
        let names = ["f_start", "ff_start", "A", "ff_end", "B", "f_end"];
        assert_eq!(
            flat_indices(&names).unwrap(),
            BTreeMap::from([("A".into(), 2), ("B".into(), 4)])
        );
    }

    #[test]
    fn namespace_caps_still_fail() {
        assert!(flat_indices(&["F_START"; 17]).is_err());
        assert!(flat_indices(&vec!["A"; MAX_LUMP_NAMES + 1]).is_err());
    }

    #[test]
    fn invalid_names_and_unclosed_markers_degrade() {
        // Names produced by lossy decoding of high-bit lump names can never
        // be valid flats; they are skipped instead of rejecting the map.
        assert!(flat_indices(&["LONG_NAME"]).unwrap().is_empty());
        assert!(flat_indices(&["é"]).unwrap().is_empty());
        // An unclosed namespace keeps the flats collected so far.
        assert_eq!(
            flat_indices(&["F_START", "A"]).unwrap(),
            BTreeMap::from([("A".into(), 1)])
        );
        assert!(flat_indices(&["F_END"]).unwrap().is_empty());
    }
}
