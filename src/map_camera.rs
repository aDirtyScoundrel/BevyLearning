//! Camera-only collision against the current Doom collision world.
use crate::doom_wad::{DOOR_COLLISION_OPEN_THRESHOLD, WadCollisionWorld};
use bevy::math::DVec2;
use bevy::prelude::*;

/// Preserve the unblocked offset so the camera can extend again.
#[derive(Component, Debug, Clone, Copy)]
pub struct CameraBoom {
    pub rest_offset: Vec3,
}

pub fn near_plane_radius(projection: &Projection) -> f32 {
    match projection {
        Projection::Perspective(p) => {
            let half_height = p.near * (p.fov * 0.5).tan();
            let half_width = half_height * p.aspect_ratio;
            safe_radius(Vec3::new(half_width, half_height, p.near).length() + 0.02)
        }
        _ => 0.15,
    }
}

const MAX_EDGES: usize = 262_144;
const MAX_CUTS: usize = 4096;

fn safe_radius(radius: f32) -> f32 {
    if radius.is_finite() {
        radius.clamp(0.01, 1.0)
    } else {
        0.15
    }
}

/// Keep the orbit pivot between the floor and ceiling.
pub fn camera_anchor(world: &WadCollisionWorld, pivot: Vec3, radius: f32) -> Vec3 {
    if !pivot.is_finite() {
        return Vec3::ZERO;
    }
    let Some((floor, ceiling)) = world.vertical_bounds(pivot) else {
        return pivot;
    };
    if !floor.is_finite() || !ceiling.is_finite() {
        return pivot;
    }
    let radius = safe_radius(radius);
    let mut result = pivot;
    result.y = if ceiling - floor < 2.0 * radius {
        ((f64::from(floor) + f64::from(ceiling)) * 0.5) as f32
    } else {
        pivot.y.clamp(floor + radius, ceiling - radius)
    };
    result
}

/// Shorten the camera offset without changing its direction.
pub fn clip_boom(world: &WadCollisionWorld, start: Vec3, desired: Vec3, radius: f32) -> Vec3 {
    if !start.is_finite() {
        return Vec3::ZERO;
    }
    if !desired.is_finite() {
        return start;
    }
    let delta = desired - start;
    let length = delta.length();
    if !length.is_finite() || length > 32.0 || length == 0.0 {
        return start;
    }
    let count = world
        .sectors
        .iter()
        .try_fold(world.segments.len(), |n, s| n.checked_add(s.edges.len()))
        .and_then(|n| n.checked_add(world.doors.len()));
    if count.is_none_or(|n| n > MAX_EDGES) || world.sectors.len() > MAX_EDGES {
        return start;
    }
    let radius = f64::from(safe_radius(radius));
    let s = DVec2::new(f64::from(start.x), f64::from(start.z));
    let d = DVec2::new(f64::from(delta.x), f64::from(delta.z));
    let mut limit = 1.0_f64;
    for wall in &world.segments {
        limit = limit.min(capsule_hit(
            s,
            d,
            wall.start.as_dvec2(),
            wall.end.as_dvec2(),
            radius,
        ));
    }
    for door in &world.doors {
        if !door.open_amount.is_finite() || door.open_amount < DOOR_COLLISION_OPEN_THRESHOLD {
            limit = limit.min(capsule_hit(
                s,
                d,
                door.start.as_dvec2(),
                door.end.as_dvec2(),
                radius,
            ));
        }
    }
    if !world.sectors.is_empty() {
        limit = limit.min(sector_limit(world, start, delta, radius));
    }
    if limit >= 1.0 {
        return desired;
    }
    let safe_t = (limit - 0.0001 / f64::from(length)).max(0.0) as f32;
    start + delta * safe_t
}

/// Sweep a point against a segment with rounded ends.
fn capsule_hit(s: DVec2, d: DVec2, a: DVec2, b: DVec2, radius: f64) -> f64 {
    if !a.is_finite() || !b.is_finite() {
        return 0.0;
    }
    let ab = b - a;
    let length_sq = ab.length_squared();
    let closest = if length_sq == 0.0 {
        a
    } else {
        a + ab * ((s - a).dot(ab) / length_sq).clamp(0.0, 1.0)
    };
    if (s - closest).length_squared() <= radius * radius {
        return 0.0;
    }
    let mut limit = 1.0_f64;
    if length_sq > 0.0 {
        let length = length_sq.sqrt();
        let tangent = ab / length;
        let normal = DVec2::new(-tangent.y, tangent.x);
        let speed = d.dot(normal);
        if speed != 0.0 {
            for side in [-radius, radius] {
                let t = (side - (s - a).dot(normal)) / speed;
                let projection = (s + d * t - a).dot(tangent);
                if (0.0..=1.0).contains(&t) && (0.0..=length).contains(&projection) {
                    limit = limit.min(t);
                }
            }
        }
    }
    let aa = d.length_squared();
    if aa > 0.0 {
        for endpoint in [a, b] {
            let offset = s - endpoint;
            let bb = offset.dot(d);
            let cc = offset.length_squared() - radius * radius;
            let discriminant = bb * bb - aa * cc;
            if discriminant >= 0.0 {
                let t = (-bb - discriminant.sqrt()) / aa;
                if (0.0..=1.0).contains(&t) {
                    limit = limit.min(t);
                }
            }
        }
    }
    limit
}

fn cross(a: DVec2, b: DVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

fn sector_limit(world: &WadCollisionWorld, start: Vec3, delta: Vec3, radius: f64) -> f64 {
    let s = DVec2::new(f64::from(start.x), f64::from(start.z));
    let d = DVec2::new(f64::from(delta.x), f64::from(delta.z));
    // Each cut retains the capsule contact before its boundary crossing.
    let mut cuts = vec![(0.0_f64, 0.0_f64), (1.0, 1.0)];
    for sector in &world.sectors {
        for edge in &sector.edges {
            let (a, b) = (edge.start.as_dvec2(), edge.end.as_dvec2());
            if !a.is_finite() || !b.is_finite() {
                return 0.0;
            }
            let ab = b - a;
            let denominator = cross(d, ab);
            if denominator == 0.0 {
                continue;
            }
            let t = cross(a - s, ab) / denominator;
            let u = cross(a - s, d) / denominator;
            if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                if cuts.len() == MAX_CUTS {
                    return 0.0;
                }
                cuts.push((t, capsule_hit(s, d, a, b, radius).min(t)));
            }
        }
    }
    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut unique: Vec<(f64, f64)> = Vec::with_capacity(cuts.len());
    for cut in cuts {
        if let Some(last) = unique.last_mut()
            && last.0 == cut.0
        {
            last.1 = last.1.min(cut.1);
        } else {
            unique.push(cut);
        }
    }
    let query_edges: usize = world
        .sectors
        .iter()
        .map(|sector| sector.edges.len().max(1))
        .sum();
    if unique.len().saturating_mul(query_edges) > 4_000_000 {
        return 0.0;
    }
    for interval in unique.windows(2) {
        let ((entry, safe_entry), (exit, _)) = (interval[0], interval[1]);
        let point = start + delta * ((entry + exit) * 0.5) as f32;
        let Some((floor, ceiling)) = world.vertical_bounds(point) else {
            return safe_entry;
        };
        if !floor.is_finite() || !ceiling.is_finite() {
            return 0.0;
        }
        let low = f64::from(floor) + radius;
        let high = f64::from(ceiling) - radius;
        let initial_y = f64::from(start.y);
        let dy = f64::from(delta.y);
        let y_entry = initial_y + dy * entry;
        if low > high || y_entry < low || y_entry > high {
            return safe_entry;
        }
        let y_exit = initial_y + dy * exit;
        if y_exit > high && dy > 0.0 {
            return ((high - initial_y) / dy).clamp(entry, exit);
        }
        if y_exit < low && dy < 0.0 {
            return ((low - initial_y) / dy).clamp(entry, exit);
        }
    }
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doom_wad::{WadDoor, WadSectorRegion, WallCollisionSegment};

    pub fn room(x0: f32, x1: f32, floor: f32, ceiling: f32) -> WadSectorRegion {
        let p = [
            Vec2::new(x0, -10.0),
            Vec2::new(x1, -10.0),
            Vec2::new(x1, 10.0),
            Vec2::new(x0, 10.0),
        ];
        WadSectorRegion {
            floor,
            ceiling,
            edges: [(0, 1), (1, 2), (2, 3), (3, 0)]
                .map(|(a, b)| WallCollisionSegment {
                    start: p[a],
                    end: p[b],
                })
                .to_vec(),
        }
    }

    #[test]
    fn empty_world_and_invalid_inputs() {
        let world = WadCollisionWorld::default();
        let start = Vec3::new(1.0, 2.0, 3.0);
        let end = Vec3::new(8.0, 5.0, 2.0);
        assert_eq!(clip_boom(&world, start, end, 0.15), end);
        assert_eq!(clip_boom(&world, start, Vec3::NAN, 0.15), start);
        assert_eq!(
            clip_boom(&world, start, start + Vec3::X * 33.0, 0.15),
            start
        );
        assert_eq!(clip_boom(&world, Vec3::NAN, end, 0.15), Vec3::ZERO);
        assert_eq!(camera_anchor(&world, Vec3::NAN, 0.15), Vec3::ZERO);
    }

    #[test]
    fn ceiling_and_raised_floor_stop_vertical_booms() {
        let world = WadCollisionWorld {
            sectors: vec![room(-10.0, 10.0, 3.0, 5.0)],
            ..default()
        };
        let start = Vec3::new(0.0, 4.0, 0.0);
        let up = clip_boom(&world, start, Vec3::new(2.0, 8.0, 0.0), 0.15);
        assert!((up.y - 4.85).abs() < 0.001);
        let down = clip_boom(&world, start, Vec3::new(2.0, 0.0, 0.0), 0.15);
        assert!((down.y - 3.15).abs() < 0.001);
        assert!((camera_anchor(&world, Vec3::new(0.0, 8.0, 0.0), 0.15).y - 4.85).abs() < 0.001);
        let tiny = WadCollisionWorld {
            sectors: vec![room(-1.0, 1.0, 3.0, 3.1)],
            ..default()
        };
        assert!((camera_anchor(&tiny, Vec3::ZERO, 0.15).y - 3.05).abs() < 0.001);
    }

    #[test]
    fn thin_and_oblique_walls_preserve_clearance() {
        let wall = WallCollisionSegment {
            start: Vec2::new(0.0, -4.0),
            end: Vec2::new(0.0, 4.0),
        };
        let world = WadCollisionWorld {
            segments: vec![wall],
            ..default()
        };
        let start = Vec3::new(-3.0, 1.0, 0.0);
        for end in [Vec3::new(4.0, 1.0, 0.0), Vec3::new(4.0, 2.0, 2.0)] {
            let result = clip_boom(&world, start, end, 0.2);
            assert!((result.x + 0.2).abs() < 0.001);
            assert!((result - start).cross(end - start).length() < 0.001);
        }
    }

    #[test]
    fn endpoint_capsules_and_parallel_motion() {
        for end in [Vec2::ZERO, Vec2::new(0.0, 1.0)] {
            let world = WadCollisionWorld {
                segments: vec![WallCollisionSegment {
                    start: Vec2::ZERO,
                    end,
                }],
                ..default()
            };
            let start = Vec3::new(-2.0, 1.0, -0.1);
            let result = clip_boom(&world, start, Vec3::new(2.0, 1.0, -0.1), 0.2);
            assert!(result.x < -0.17 && result.x > -0.18);
            assert_eq!(
                clip_boom(
                    &world,
                    Vec3::new(0.05, 1.0, 0.0),
                    Vec3::new(2.0, 1.0, 0.0),
                    0.2
                ),
                Vec3::new(0.05, 1.0, 0.0)
            );
        }
        let world = WadCollisionWorld {
            segments: vec![WallCollisionSegment {
                start: Vec2::ZERO,
                end: Vec2::Y,
            }],
            ..default()
        };
        let desired = Vec3::new(-1.0, 1.0, 3.0);
        assert_eq!(
            clip_boom(&world, Vec3::new(-1.0, 1.0, -3.0), desired, 0.2),
            desired
        );
    }

    #[test]
    fn portals_allow_motion_but_thin_low_sectors_block() {
        let start = Vec3::new(-2.0, 1.0, 0.0);
        let end = Vec3::new(3.0, 1.0, 0.0);
        let world = WadCollisionWorld {
            sectors: vec![room(-5.0, 0.0, 0.0, 5.0), room(0.0, 5.0, 0.0, 5.0)],
            ..default()
        };
        assert_eq!(clip_boom(&world, start, end, 0.15), end);
        let world = WadCollisionWorld {
            sectors: vec![
                room(-5.0, 0.0, 0.0, 5.0),
                room(0.0, 0.01, 0.0, 0.5),
                room(0.01, 5.0, 0.0, 5.0),
            ],
            ..default()
        };
        let result = clip_boom(&world, start, end, 0.15);
        assert!((result.x + 0.15).abs() < 0.001);
    }

    #[test]
    fn door_state_controls_camera_collision() {
        let mut world = WadCollisionWorld {
            doors: vec![WadDoor {
                entity: Entity::PLACEHOLDER,
                debug_entity: Entity::PLACEHOLDER,
                start: Vec2::new(0.0, -2.0),
                end: Vec2::new(0.0, 2.0),
                bottom: 0.0,
                closed_height: 3.0,
                open_amount: 0.0,
                hold_timer_secs: 0.0,
                required_key: None,
            }],
            ..default()
        };
        let start = Vec3::new(-2.0, 1.0, 0.0);
        let end = Vec3::new(3.0, 1.0, 0.0);
        assert!((clip_boom(&world, start, end, 0.15).x + 0.15).abs() < 0.001);
        world.doors[0].open_amount = 1.0;
        assert_eq!(clip_boom(&world, start, end, 0.15), end);
    }

    #[test]
    fn sector_query_budget_fails_closed() {
        let world = WadCollisionWorld {
            sectors: (0..1200)
                .map(|i| room(i as f32 * 0.02, (i + 1) as f32 * 0.02, 0.0, 4.0))
                .collect(),
            ..default()
        };
        let start = Vec3::new(0.001, 1.0, 0.0);
        assert_eq!(
            clip_boom(&world, start, Vec3::new(23.999, 1.0, 0.0), 0.15),
            start
        );
    }

    #[test]
    fn edge_budget_fails_closed() {
        let world = WadCollisionWorld {
            segments: vec![
                WallCollisionSegment {
                    start: Vec2::ZERO,
                    end: Vec2::ONE
                };
                MAX_EDGES + 1
            ],
            ..default()
        };
        let start = Vec3::new(-2.0, 1.0, 0.0);
        assert_eq!(clip_boom(&world, start, Vec3::ONE, 0.15), start);
    }
}
