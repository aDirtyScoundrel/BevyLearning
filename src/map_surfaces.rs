//! Bounded floor and ceiling meshes for native map importers.
use bevy::{asset::RenderAssetUsages, prelude::*, render::render_resource::PrimitiveTopology};
use std::collections::BTreeMap;

const MAX_EDGES: usize = 8192;
const MAX_SCAN_WORK: usize = 4_000_000;
const MAX_TRIANGLES: usize = 65_536;

/// Fill unordered closed boundaries with the even-odd rule.
/// Horizontal strips preserve concave outlines, holes and separate islands.
pub fn triangulate(edges: &[(Vec2, Vec2)]) -> Result<Vec<[Vec2; 3]>, String> {
    if edges.len() > MAX_EDGES {
        return Err("Sector exceeds the edge limit".into());
    }
    let mut levels = Vec::with_capacity(edges.len() * 2);
    let mut degree = BTreeMap::<(u32, u32), usize>::new();
    for &(a, b) in edges {
        if !a.is_finite() || !b.is_finite() {
            return Err("Sector contains a non-finite endpoint".into());
        }
        for p in [a, b] {
            levels.push(p.y);
            let bits = |v: f32| if v == 0.0 { 0 } else { v.to_bits() };
            *degree.entry((bits(p.x), bits(p.y))).or_default() += 1;
        }
    }
    if degree.values().any(|count| count % 2 != 0) {
        return Err("Sector boundary is open".into());
    }
    levels.sort_by(f32::total_cmp);
    levels.dedup();
    if edges
        .len()
        .checked_mul(levels.len())
        .is_none_or(|work| work > MAX_SCAN_WORK)
    {
        return Err("Sector exceeds the scan limit".into());
    }
    let mut triangles = Vec::new();
    for strip in levels.windows(2) {
        let (low, high) = (f64::from(strip[0]), f64::from(strip[1]));
        let mid = (low + high) * 0.5;
        let mut crossings = Vec::new();
        for &(a, b) in edges {
            let (ay, by) = (f64::from(a.y), f64::from(b.y));
            if (ay < mid && mid < by) || (by < mid && mid < ay) {
                crossings.push((x_at(a, b, mid), a, b));
            }
        }
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        if crossings.len() % 2 != 0 {
            return Err("Sector boundary is open".into());
        }
        // A crossing between separate filled spans also invalidates this strip.
        for pair in crossings.windows(2) {
            for y in [low, high] {
                let left = x_at(pair[0].1, pair[0].2, y);
                let right = x_at(pair[1].1, pair[1].2, y);
                let tolerance = left.abs().max(right.abs()).max(1.0) * 1e-12;
                if left > right + tolerance {
                    return Err("Sector boundaries cross inside a strip".into());
                }
            }
        }
        for pair in crossings.chunks_exact(2) {
            let (_, la, lb) = pair[0];
            let (_, ra, rb) = pair[1];
            let (ll, lh) = (x_at(la, lb, low), x_at(la, lb, high));
            let (rl, rh) = (x_at(ra, rb, low), x_at(ra, rb, high));
            let corners = [
                Vec2::new(ll as f32, strip[0]),
                Vec2::new(rl as f32, strip[0]),
                Vec2::new(rh as f32, strip[1]),
                Vec2::new(lh as f32, strip[1]),
            ];
            for mut triangle in [
                [corners[0], corners[1], corners[2]],
                [corners[0], corners[2], corners[3]],
            ] {
                let area = signed_area(triangle[0], triangle[1], triangle[2]);
                if area == 0.0 {
                    continue;
                }
                if area < 0.0 {
                    triangle.swap(1, 2);
                }
                if triangles.len() == MAX_TRIANGLES {
                    return Err("Sector exceeds the triangle limit".into());
                }
                triangles.push(triangle);
            }
        }
    }
    Ok(triangles)
}

fn x_at(a: Vec2, b: Vec2, y: f64) -> f64 {
    let t = (y - f64::from(a.y)) / (f64::from(b.y) - f64::from(a.y));
    f64::from(a.x) + t * (f64::from(b.x) - f64::from(a.x))
}

fn signed_area(a: Vec2, b: Vec2, c: Vec2) -> f64 {
    let ab = b.as_dvec2() - a.as_dvec2();
    let ac = c.as_dvec2() - a.as_dvec2();
    (ab.x * ac.y - ac.x * ab.y) * 0.5
}

/// Make inward-facing triangles with world-aligned texture coordinates.
pub fn horizontal_mesh(
    edges: &[(Vec2, Vec2)],
    height: f32,
    ceiling: bool,
    uv_origin: Vec2,
    uv_world_size: f32,
) -> Result<Mesh, String> {
    if !height.is_finite()
        || !uv_origin.is_finite()
        || !uv_world_size.is_finite()
        || uv_world_size <= 0.0
    {
        return Err("Surface height or texture scale is invalid".into());
    }
    let triangles = triangulate(edges)?;
    let mut positions = Vec::with_capacity(triangles.len() * 3);
    let mut normals = Vec::with_capacity(triangles.len() * 3);
    let mut uvs = Vec::with_capacity(triangles.len() * 3);
    let normal = if ceiling {
        [0.0, -1.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    for mut triangle in triangles {
        // Positive X/Z area has a negative Y normal in Bevy.
        if !ceiling {
            triangle.swap(1, 2);
        }
        for p in triangle {
            let uv = [
                (p.x + uv_origin.x) / uv_world_size,
                (p.y + uv_origin.y) / uv_world_size,
            ];
            if !p.is_finite() || !uv.iter().all(|v| v.is_finite()) {
                return Err("Surface contains a non-finite vertex or texture coordinate".into());
            }
            positions.push([p.x, height, p.y]);
            normals.push(normal);
            uvs.push(uv);
        }
    }
    Ok(Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unordered_rectangle_area() {
        let edges = vec![
            (Vec2::new(2.0, 0.0), Vec2::new(0.0, 0.0)),
            (Vec2::new(2.0, 0.0), Vec2::new(2.0, 2.0)),
            (Vec2::new(2.0, 2.0), Vec2::new(0.0, 2.0)),
            (Vec2::new(0.0, 2.0), Vec2::new(0.0, 0.0)),
        ];
        let tris = triangulate(&edges).unwrap();
        let total_area: f64 = tris
            .iter()
            .map(|t| signed_area(t[0], t[1], t[2]).abs())
            .sum();
        assert!(
            (total_area - 4.0).abs() < 1e-6,
            "Expected area 4.0, got {}",
            total_area
        );
    }

    #[test]
    fn test_concave_l_shape() {
        let edges = vec![
            (Vec2::new(0.0, 0.0), Vec2::new(3.0, 0.0)),
            (Vec2::new(3.0, 0.0), Vec2::new(3.0, 1.0)),
            (Vec2::new(3.0, 1.0), Vec2::new(1.0, 1.0)),
            (Vec2::new(1.0, 1.0), Vec2::new(1.0, 3.0)),
            (Vec2::new(1.0, 3.0), Vec2::new(0.0, 3.0)),
            (Vec2::new(0.0, 3.0), Vec2::new(0.0, 0.0)),
        ];
        let tris = triangulate(&edges).unwrap();
        let total_area: f64 = tris
            .iter()
            .map(|t| signed_area(t[0], t[1], t[2]).abs())
            .sum();
        assert!(
            (total_area - 5.0).abs() < 1e-6,
            "Expected area 5.0, got {}",
            total_area
        );

        for tri in &tris {
            let cx = (tri[0].x + tri[1].x + tri[2].x) / 3.0;
            let cy = (tri[0].y + tri[1].y + tri[2].y) / 3.0;
            let inside = (cx >= 0.0 && cx <= 3.0 && cy >= 0.0 && cy <= 1.0)
                || (cx >= 0.0 && cx <= 1.0 && cy >= 0.0 && cy <= 3.0);
            assert!(inside, "Centroid ({}, {}) outside L-shape", cx, cy);
        }
    }

    #[test]
    fn test_rectangular_hole() {
        let edges = vec![
            (Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0)),
            (Vec2::new(4.0, 0.0), Vec2::new(4.0, 4.0)),
            (Vec2::new(4.0, 4.0), Vec2::new(0.0, 4.0)),
            (Vec2::new(0.0, 4.0), Vec2::new(0.0, 0.0)),
            (Vec2::new(1.0, 1.0), Vec2::new(3.0, 1.0)),
            (Vec2::new(3.0, 1.0), Vec2::new(3.0, 3.0)),
            (Vec2::new(3.0, 3.0), Vec2::new(1.0, 3.0)),
            (Vec2::new(1.0, 3.0), Vec2::new(1.0, 1.0)),
        ];
        let tris = triangulate(&edges).unwrap();
        let total_area: f64 = tris
            .iter()
            .map(|t| signed_area(t[0], t[1], t[2]).abs())
            .sum();
        assert!(
            (total_area - 12.0).abs() < 1e-6,
            "Expected area 12.0, got {}",
            total_area
        );

        for tri in &tris {
            let cx = (tri[0].x + tri[1].x + tri[2].x) / 3.0;
            let cy = (tri[0].y + tri[1].y + tri[2].y) / 3.0;
            let in_hole = cx > 1.0 && cx < 3.0 && cy > 1.0 && cy < 3.0;
            assert!(!in_hole, "Centroid ({}, {}) inside hole", cx, cy);
        }
    }

    #[test]
    fn test_disconnected_islands() {
        let edges = vec![
            (Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0)),
            (Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0)),
            (Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)),
            (Vec2::new(0.0, 1.0), Vec2::new(0.0, 0.0)),
            (Vec2::new(2.0, 0.0), Vec2::new(3.0, 0.0)),
            (Vec2::new(3.0, 0.0), Vec2::new(3.0, 1.0)),
            (Vec2::new(3.0, 1.0), Vec2::new(2.0, 1.0)),
            (Vec2::new(2.0, 1.0), Vec2::new(2.0, 0.0)),
        ];
        let tris = triangulate(&edges).unwrap();
        let total_area: f64 = tris
            .iter()
            .map(|t| signed_area(t[0], t[1], t[2]).abs())
            .sum();
        assert!(
            (total_area - 2.0).abs() < 1e-6,
            "Expected area 2.0, got {}",
            total_area
        );
    }

    #[test]
    fn test_sloped_triangle_boundary() {
        let edges = vec![
            (Vec2::new(0.0, 0.0), Vec2::new(2.0, 0.0)),
            (Vec2::new(2.0, 0.0), Vec2::new(1.0, 2.0)),
            (Vec2::new(1.0, 2.0), Vec2::new(0.0, 0.0)),
        ];
        let tris = triangulate(&edges).unwrap();
        let total_area: f64 = tris
            .iter()
            .map(|t| signed_area(t[0], t[1], t[2]).abs())
            .sum();
        assert!(
            (total_area - 2.0).abs() < 1e-6,
            "Expected area 2.0, got {}",
            total_area
        );
    }

    fn square() -> Vec<(Vec2, Vec2)> {
        let p = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y];
        vec![(p[2], p[3]), (p[1], p[0]), (p[0], p[3]), (p[2], p[1])]
    }

    #[test]
    fn mesh_faces_heights_normals_and_uvs_agree() {
        use bevy::mesh::VertexAttributeValues;
        for ceiling in [false, true] {
            let mesh = horizontal_mesh(&square(), 5.0, ceiling, Vec2::new(1.0, 2.0), 2.0).unwrap();
            let VertexAttributeValues::Float32x3(positions) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
            else {
                panic!("Positions have the wrong type")
            };
            let VertexAttributeValues::Float32x3(normals) =
                mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap()
            else {
                panic!("Normals have the wrong type")
            };
            let VertexAttributeValues::Float32x2(uvs) =
                mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap()
            else {
                panic!("UVs have the wrong type")
            };
            assert_eq!(positions.len(), 6);
            assert_eq!(normals.len(), positions.len());
            assert_eq!(uvs.len(), positions.len());
            let expected = if ceiling { -Vec3::Y } else { Vec3::Y };
            for face in positions.chunks_exact(3) {
                let [a, b, c] = [
                    Vec3::from(face[0]),
                    Vec3::from(face[1]),
                    Vec3::from(face[2]),
                ];
                assert!((b - a).cross(c - a).dot(expected) > 0.0);
            }
            for ((position, normal), uv) in positions.iter().zip(normals).zip(uvs) {
                assert_eq!(position[1], 5.0);
                assert_eq!(*normal, expected.to_array());
                assert_eq!(*uv, [(position[0] + 1.0) / 2.0, (position[2] + 2.0) / 2.0]);
            }
        }
    }

    #[test]
    fn invalid_and_empty_inputs() {
        assert!(triangulate(&[]).unwrap().is_empty());
        let mut edges = square();
        edges.remove(1);
        assert!(triangulate(&edges).unwrap_err().contains("open"));
        assert!(triangulate(&[(Vec2::NAN, Vec2::ZERO)]).is_err());
        assert!(horizontal_mesh(&square(), f32::NAN, false, Vec2::ZERO, 1.0).is_err());
        assert!(horizontal_mesh(&square(), 0.0, false, Vec2::ZERO, 0.0).is_err());
        assert!(horizontal_mesh(&square(), 0.0, false, Vec2::NAN, 1.0).is_err());
    }

    #[test]
    fn resource_limits_reject_bounded_fixtures() {
        assert!(
            triangulate(&vec![(Vec2::ZERO, Vec2::ONE); MAX_EDGES + 1])
                .unwrap_err()
                .contains("edge limit")
        );
        let mut edges = Vec::new();
        for i in 0..1500 {
            let p = Vec2::new(0.0, i as f32);
            edges.extend([(p, p + Vec2::ONE), (p + Vec2::ONE, p)]);
        }
        assert!(triangulate(&edges).unwrap_err().contains("scan limit"));
    }

    #[test]
    fn intersecting_boundaries_are_rejected() {
        let a = [
            Vec2::ZERO,
            Vec2::X,
            Vec2::new(3.0, 2.0),
            Vec2::new(0.0, 2.0),
        ];
        let b = [
            Vec2::new(2.0, 0.0),
            Vec2::new(4.0, 0.0),
            Vec2::new(4.0, 2.0),
            Vec2::new(2.0, 2.0),
        ];
        let mut edges = Vec::new();
        for p in [a, b] {
            edges.extend([(p[0], p[1]), (p[1], p[2]), (p[2], p[3]), (p[3], p[0])]);
        }
        assert!(triangulate(&edges).is_err());
    }

    #[test]
    fn adjacent_float_levels_remain_distinct() {
        let low = 1.0_f32;
        let high = f32::from_bits(low.to_bits() + 1);
        let p = [
            Vec2::new(0.0, low),
            Vec2::new(2.0, low),
            Vec2::new(2.0, high),
            Vec2::new(0.0, high),
        ];
        let edges = [(p[0], p[1]), (p[1], p[2]), (p[2], p[3]), (p[3], p[0])];
        let triangles = triangulate(&edges).unwrap();
        assert_eq!(triangles.len(), 2);
        let area: f64 = triangles
            .iter()
            .map(|t| signed_area(t[0], t[1], t[2]))
            .sum();
        assert_eq!(area, 2.0 * f64::from(high - low));
    }
}
