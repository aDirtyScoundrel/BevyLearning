use std::io::{Cursor, Read};
use std::path::Path;

use bevy::math::primitives::Cuboid;
use bevy::mesh::Mesh3d;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;

const DOOM_TO_WORLD_SCALE: f32 = 0.02;
const WALL_THICKNESS: f32 = 0.22;
const FLOOR_PADDING: f32 = 2.0;
const MIN_FLOOR_HALF_EXTENT: f32 = 12.0;
const COLLISION_PADDING: f32 = 0.26;
const WALL_MIN_HEIGHT: f32 = 0.05;
const NO_SIDEDEF: u16 = 0xFFFF;
const COLLISION_DEBUG_HEIGHT: f32 = 0.03;
const COLLISION_DEBUG_THICKNESS: f32 = 0.04;
const DOOR_OPEN_SPEED: f32 = 1.8;
const DOOR_HOLD_SECS: f32 = 2.0;
const DOOR_TRIGGER_DISTANCE: f32 = 1.7;
pub(crate) const DOOR_COLLISION_OPEN_THRESHOLD: f32 = 0.88;
const MIN_DOOR_SCALE_Y: f32 = 0.04;
const MAX_TRAVERSABLE_STEP_HEIGHT: f32 = 0.50;

/// Maximum bytes we'll read from a single archive entry (embedded WAD,
/// nested PK3/zip, or UDMF TEXTMAP) before giving up. WAD/PK3 files are
/// untrusted user-supplied content; without this cap a crafted archive
/// with a small compressed size but a huge decompressed entry could
/// exhaust memory well before the existing MAX_NESTED_DEPTH guard (which
/// only limits *recursive* zip nesting, not single-entry bloat) applies.
const MAX_ARCHIVE_ENTRY_BYTES: u64 = 64 * 1024 * 1024; // 64 MiB

/// Reads at most `MAX_ARCHIVE_ENTRY_BYTES` from `entry` into a `Vec<u8>`,
/// erroring out if the entry is larger than that.
fn read_entry_bytes_capped<R: Read>(entry: &mut R, entry_name: &str) -> Result<Vec<u8>, String> {
    let mut limited = entry.take(MAX_ARCHIVE_ENTRY_BYTES + 1);
    let mut buf = Vec::new();
    limited
        .read_to_end(&mut buf)
        .map_err(|error| format!("unable to read archive entry {entry_name}: {error}"))?;
    if buf.len() as u64 > MAX_ARCHIVE_ENTRY_BYTES {
        return Err(format!(
            "archive entry {entry_name} exceeds the {MAX_ARCHIVE_ENTRY_BYTES}-byte size cap"
        ));
    }
    Ok(buf)
}

/// Reads at most `MAX_ARCHIVE_ENTRY_BYTES` from `entry` into a `String`,
/// erroring out if the entry is larger than that or not valid UTF-8.
fn read_entry_string_capped<R: Read>(entry: &mut R, entry_name: &str) -> Result<String, String> {
    let bytes = read_entry_bytes_capped(entry, entry_name)?;
    String::from_utf8(bytes)
        .map_err(|error| format!("archive entry {entry_name} is not valid UTF-8: {error}"))
}

#[derive(Debug, Clone, Copy)]
pub struct LevelSpawnInfo {
    pub player_spawn: Vec3,
    pub player_yaw: f32,
    pub suggested_plane_limit: f32,
}

#[derive(Debug, Clone)]
pub struct LoadedLevel {
    pub info: LevelSpawnInfo,
    pub map_name: String,
    pub source_label: String,
}

#[derive(Debug, Clone)]
struct MapSource {
    map_data: MapSourceData,
    resolved_map_name: String,
    source_label: String,
}

#[derive(Debug, Clone)]
enum MapSourceData {
    ClassicWad(Vec<u8>),
    Udmf(ParsedMap),
}

#[derive(Debug, Clone)]
struct LumpEntry {
    offset: usize,
    size: usize,
    name: String,
}

#[derive(Debug, Clone, Copy)]
struct Vertex {
    x: i16,
    y: i16,
}

#[derive(Debug, Clone, Copy)]
struct LineDef {
    start_vertex: u16,
    end_vertex: u16,
    special: u16,
    tag: u16,
    right_sidedef: u16,
    left_sidedef: u16,
}

/// Doom keycard/skull-key colors used by locked-door specials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyColor {
    Red,
    Blue,
    Yellow,
}

/// Broad category of effect a linedef special triggers. Only `Door` is
/// dispatched into real gameplay behavior today — the others exist so the
/// decode table has a home for specials as they're implemented (see
/// docs/CLASSIC_MAP_SUPPORT.md items 2/5/6), instead of silently treating
/// every non-door special as "not a special."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Only Door is dispatched today; rest scaffolded for docs/CLASSIC_MAP_SUPPORT.md items 2/5/6.
enum SpecialCategory {
    Door,
    Floor,
    Ceiling,
    Lift,
    Stair,
    Teleport,
    Exit,
    LightEffect,
    Scroller,
}

/// How a linedef special is triggered in vanilla Doom terms. Activation is
/// not yet enforced (everything still opens on proximity — see
/// docs/CLASSIC_MAP_SUPPORT.md item 2) but is decoded now so the dispatch
/// table doesn't need reshaping when real walk/switch/shoot activation
/// lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Walk/Gun activation isn't dispatched yet — docs/CLASSIC_MAP_SUPPORT.md item 2.
enum ActivationKind {
    /// Player walks across the line.
    Walk,
    /// Player uses/pushes the line (a switch or a door pushed open by hand).
    Manual,
    /// Line is triggered by a hitscan or projectile impact.
    Gun,
}

/// Decoded metadata for a single linedef `special` ID. This is a real
/// dispatch table over Doom's numeric special IDs (see the DoomWiki
/// "Linedef type" reference), not a geometry-shape heuristic — replaces
/// the old `is_door_linedef`/`is_door_geometry_candidate` pair for
/// recognizing doors, and gives locked doors a real key requirement
/// instead of treating every door special as unlocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinedefSpecial {
    category: SpecialCategory,
    activation: ActivationKind,
    repeatable: bool,
    required_key: Option<KeyColor>,
}

/// Decodes a linedef's numeric `special` field into structured metadata.
/// Currently only recognizes the vanilla door specials (needed for door
/// spawning + future keycard gating); other categories return `None` today
/// and fall through to the existing geometry-based door-candidate fallback
/// so already-working maps don't regress while the dispatch table grows.
/// Extend this match arm-by-arm as docs/CLASSIC_MAP_SUPPORT.md items 2, 5,
/// and 6 land — do not replace it with a heuristic again.
fn decode_linedef_special(special: u16) -> Option<LinedefSpecial> {
    use ActivationKind::*;
    use SpecialCategory::*;

    let (category, activation, repeatable, required_key) = match special {
        // Vanilla Doom door specials (manual activation)
        1 => (Door, Manual, true, None),
        26 => (Door, Manual, true, Some(KeyColor::Blue)),
        27 => (Door, Manual, true, Some(KeyColor::Yellow)),
        28 => (Door, Manual, true, Some(KeyColor::Red)),
        31 => (Door, Manual, false, None),
        32 => (Door, Manual, false, Some(KeyColor::Blue)),
        33 => (Door, Manual, false, Some(KeyColor::Red)),
        34 => (Door, Manual, false, Some(KeyColor::Yellow)),
        117 => (Door, Manual, true, None),
        118 => (Door, Manual, false, None),

        // Vanilla Doom floor specials (walk-over activation)
        // Lower floor to ground level (fastest)
        19 => (Floor, Walk, true, None),
        18 => (Floor, Walk, true, None),
        // Lower floor to ground + stay (one-way)
        36 => (Floor, Walk, false, None),
        // Raise floor to nearest ceiling
        64 => (Floor, Walk, true, None),
        // Lower floor - 8 units quickly
        60 => (Floor, Walk, false, None),
        // Two-way floor (up/down toggle)
        56 => (Floor, Walk, true, None),

        // Vanilla Doom lifting platform specials
        // Raise platform to next floor
        62 => (Lift, Walk, true, None),
        // Lower platform to ground
        63 => (Lift, Walk, true, None),
        // Raise lift to ceiling level
        66 => (Lift, Walk, true, None),

        // Vanilla Doom ceiling specials (walk-over activation)
        // Lower ceiling to floor + 4
        40 => (Ceiling, Walk, true, None),
        // Lower ceiling to starting height
        41 => (Ceiling, Walk, true, None),
        // Raise ceiling to nearest height
        42 => (Ceiling, Walk, true, None),

        // Vanilla Doom stair specials (walk-over activation)
        // Build stairs up (8 steps, 8 units each)
        8 => (Stair, Walk, false, None),
        // Build stairs up (16 steps, 16 units each)
        7 => (Stair, Walk, false, None),

        // Vanilla Doom teleporter special (walk-over activation)
        // Teleport to TAG line (dest must have matching sector tag)
        39 => (Teleport, Walk, true, None),

        // Vanilla Doom exit special (walk-over activation)
        // Exit to next map
        11 => (Exit, Walk, false, None),
        // Exit to secret map
        51 => (Exit, Walk, false, None),

        // Vanilla Doom light effect specials (walk-over activation)
        // Flashing light effect
        2 => (LightEffect, Walk, true, None),
        // Strobe light effect
        3 => (LightEffect, Walk, true, None),
        // Light flicker effect
        4 => (LightEffect, Walk, true, None),

        _ => return None,
    };

    Some(LinedefSpecial {
        category,
        activation,
        repeatable,
        required_key,
    })
}

#[derive(Debug, Clone)]
struct SideDef {
    upper_texture: String,
    lower_texture: String,
    middle_texture: String,
    sector: u16,
}

#[derive(Debug, Clone)]
struct Sector {
    floor_height: i16,
    ceiling_height: i16,
    floor_texture: String,
    ceiling_texture: String,
}

#[derive(Debug, Clone, Copy)]
struct Thing {
    x: i16,
    y: i16,
    angle_deg: i16,
    kind: u16,
}

#[derive(Debug, Clone)]
struct ParsedMap {
    vertices: Vec<Vertex>,
    linedefs: Vec<LineDef>,
    sidedefs: Vec<SideDef>,
    sectors: Vec<Sector>,
    things: Vec<Thing>,
}

#[derive(Debug, Clone, Copy)]
pub struct WallCollisionSegment {
    pub start: Vec2,
    pub end: Vec2,
}

#[derive(Debug, Clone, Copy)]
pub struct WadDoor {
    pub entity: Entity,
    pub debug_entity: Entity,
    pub start: Vec2,
    pub end: Vec2,
    pub bottom: f32,
    pub closed_height: f32,
    pub open_amount: f32,
    pub hold_timer_secs: f32,
    /// Keycard/skull-key color required to operate this door, decoded from
    /// its linedef special. `None` for unlocked doors. Not yet enforced —
    /// gating on player inventory is docs/CLASSIC_MAP_SUPPORT.md item 3.
    #[allow(dead_code)]
    pub required_key: Option<KeyColor>,
}

#[derive(Resource, Debug, Clone, Default)]
pub struct WadCollisionWorld {
    pub segments: Vec<WallCollisionSegment>,
    pub doors: Vec<WadDoor>,
    pub push_radius: f32,
    pub sectors: Vec<WadSectorRegion>,
    pub player_spawns: Vec<Transform>,
}


#[derive(Debug, Clone)]
pub struct WadSectorRegion {
    pub edges: Vec<WallCollisionSegment>,
    pub floor: f32,
    pub ceiling: f32,
}

impl WadSectorRegion {
    pub fn contains(&self, point: Vec2) -> bool {
        if !point.is_finite() {
            return false;
        }

        let mut inside = false;
        for edge in &self.edges {
            let p1 = edge.start;
            let p2 = edge.end;

            if !p1.is_finite() || !p2.is_finite() {
                continue;
            }

            if (p1.y > point.y) != (p2.y > point.y) {
                let x_intersect = p1.x + (point.y - p1.y) * (p2.x - p1.x) / (p2.y - p1.y);
                if point.x < x_intersect {
                    inside = !inside;
                }
            }
        }
        inside
    }
}

fn sector_regions(map: &ParsedMap, center: Vec2) -> Vec<WadSectorRegion> {
    let mut regions: Vec<WadSectorRegion> = map.sectors.iter().map(|s| WadSectorRegion {
        edges: Vec::new(),
        floor: s.floor_height as f32 * DOOM_TO_WORLD_SCALE,
        ceiling: s.ceiling_height as f32 * DOOM_TO_WORLD_SCALE,
    }).collect();

    for linedef in &map.linedefs {
        let v1_idx = linedef.start_vertex as usize;
        let v2_idx = linedef.end_vertex as usize;

        let v1 = match map.vertices.get(v1_idx) {
            Some(v) => v,
            None => continue,
        };
        let v2 = match map.vertices.get(v2_idx) {
            Some(v) => v,
            None => continue,
        };

        let right_sector_idx = if linedef.right_sidedef != NO_SIDEDEF {
            match map.sidedefs.get(linedef.right_sidedef as usize) {
                Some(sd) => Some(sd.sector as usize),
                None => None,
            }
        } else {
            None
        };

        let left_sector_idx = if linedef.left_sidedef != NO_SIDEDEF {
            match map.sidedefs.get(linedef.left_sidedef as usize) {
                Some(sd) => Some(sd.sector as usize),
                None => None,
            }
        } else {
            None
        };

        if let (Some(r), Some(l)) = (right_sector_idx, left_sector_idx) {
            if r == l {
                continue;
            }
        }

        let p1 = doom_to_world(Vec2::new(v1.x as f32, v1.y as f32)) - center;
        let p2 = doom_to_world(Vec2::new(v2.x as f32, v2.y as f32)) - center;
        let segment = WallCollisionSegment { start: p1, end: p2 };

        if let Some(idx) = right_sector_idx {
            if idx < regions.len() {
                regions[idx].edges.push(segment);
            }
        }

        if let Some(idx) = left_sector_idx {
            if idx < regions.len() {
                regions[idx].edges.push(segment);
            }
        }
    }

    regions
}
#[cfg(test)]
mod sector_region_tests {
    use super::*;

    #[test]
    fn test_contains_unordered_rectangle() {
        let edges = vec![
            WallCollisionSegment { start: Vec2::new(0.0, 0.0), end: Vec2::new(10.0, 0.0) },
            WallCollisionSegment { start: Vec2::new(10.0, 0.0), end: Vec2::new(10.0, 10.0) },
            WallCollisionSegment { start: Vec2::new(10.0, 10.0), end: Vec2::new(0.0, 10.0) },
            WallCollisionSegment { start: Vec2::new(0.0, 10.0), end: Vec2::new(0.0, 0.0) },
        ];
        let mut edges = edges;
        edges.swap(0, 2);
        let region = WadSectorRegion { edges, floor: 0.0, ceiling: 10.0 };
        assert!(region.contains(Vec2::new(5.0, 5.0)));
        assert!(!region.contains(Vec2::new(-1.0, 5.0)));
    }

    #[test]
    fn test_contains_concave_outline() {

        let edges = vec![
            WallCollisionSegment { start: Vec2::new(0.0, 0.0), end: Vec2::new(10.0, 0.0) },
            WallCollisionSegment { start: Vec2::new(10.0, 0.0), end: Vec2::new(10.0, 5.0) },
            WallCollisionSegment { start: Vec2::new(10.0, 5.0), end: Vec2::new(5.0, 5.0) },
            WallCollisionSegment { start: Vec2::new(5.0, 5.0), end: Vec2::new(5.0, 10.0) },
            WallCollisionSegment { start: Vec2::new(5.0, 10.0), end: Vec2::new(0.0, 10.0) },
            WallCollisionSegment { start: Vec2::new(0.0, 10.0), end: Vec2::new(0.0, 0.0) },
        ];
        let mut edges = edges;
        edges.swap(0, 2);
        let region = WadSectorRegion { edges, floor: 0.0, ceiling: 10.0 };
        assert!(region.contains(Vec2::new(2.0, 2.0)));
        assert!(!region.contains(Vec2::new(7.0, 7.0)));
    }

    #[test]
    fn test_contains_inner_hole() {

        let edges = vec![

            WallCollisionSegment { start: Vec2::new(0.0, 0.0), end: Vec2::new(10.0, 0.0) },
            WallCollisionSegment { start: Vec2::new(10.0, 0.0), end: Vec2::new(10.0, 10.0) },
            WallCollisionSegment { start: Vec2::new(10.0, 10.0), end: Vec2::new(0.0, 10.0) },
            WallCollisionSegment { start: Vec2::new(0.0, 10.0), end: Vec2::new(0.0, 0.0) },

            WallCollisionSegment { start: Vec2::new(3.0, 3.0), end: Vec2::new(7.0, 3.0) },
            WallCollisionSegment { start: Vec2::new(7.0, 3.0), end: Vec2::new(7.0, 7.0) },
            WallCollisionSegment { start: Vec2::new(7.0, 7.0), end: Vec2::new(3.0, 7.0) },
            WallCollisionSegment { start: Vec2::new(3.0, 7.0), end: Vec2::new(3.0, 3.0) },
        ];
        let mut edges = edges;
        edges.swap(0, 2);
        let region = WadSectorRegion { edges, floor: 0.0, ceiling: 10.0 };
        assert!(region.contains(Vec2::new(1.0, 1.0)));
        assert!(!region.contains(Vec2::new(5.0, 5.0)));
    }

    #[test]
    fn test_contains_non_finite() {
        let edges = vec![
            WallCollisionSegment { start: Vec2::new(0.0, 0.0), end: Vec2::new(10.0, 0.0) },
            WallCollisionSegment { start: Vec2::new(10.0, 0.0), end: Vec2::new(10.0, 10.0) },
            WallCollisionSegment { start: Vec2::new(10.0, 10.0), end: Vec2::new(0.0, 10.0) },
            WallCollisionSegment { start: Vec2::new(0.0, 10.0), end: Vec2::new(0.0, 0.0) },
        ];
        let mut edges = edges;
        edges.swap(0, 2);
        let region = WadSectorRegion { edges, floor: 0.0, ceiling: 10.0 };
        assert!(!region.contains(Vec2::new(f32::NAN, 5.0)));
        assert!(!region.contains(Vec2::new(5.0, f32::INFINITY)));
    }


    #[test]
    fn sector_boundaries_preserve_ownership_and_ignore_bad_references() {
        let mut map = ParsedMap {
            vertices: vec![Vertex { x: 0, y: 0 }, Vertex { x: 128, y: 0 }],
            linedefs: vec![LineDef { start_vertex: 0, end_vertex: 1, special: 0, tag: 0, right_sidedef: 0, left_sidedef: 1 }],
            sidedefs: (0..2).map(|_| SideDef { upper_texture: String::new(), lower_texture: String::new(), middle_texture: String::new(), sector: 0 }).collect(),
            sectors: (0..2).map(|_| Sector { floor_height: 64, ceiling_height: 192, floor_texture: String::new(), ceiling_texture: String::new() }).collect(),
            things: Vec::new(),
        };
        assert!(sector_regions(&map, Vec2::ZERO).iter().all(|s| s.edges.is_empty()));
        map.sidedefs[1].sector = 1;
        let regions = sector_regions(&map, Vec2::new(1.0,2.0));
        assert_eq!(regions.len(), 2);
        for region in &regions {
            assert_eq!(region.edges.len(), 1);
            assert_eq!(region.floor, 64.0 * DOOM_TO_WORLD_SCALE);
            assert_eq!(region.ceiling, 192.0 * DOOM_TO_WORLD_SCALE);
            assert_eq!(region.edges[0].start, Vec2::new(-1.0,-2.0));
        }
        map.linedefs[0].end_vertex = u16::MAX;
        assert!(sector_regions(&map, Vec2::ZERO).iter().all(|s| s.edges.is_empty()));
        map.linedefs[0].end_vertex = 1;
        map.linedefs[0].left_sidedef = u16::MAX;
        let regions = sector_regions(&map, Vec2::ZERO);
        assert_eq!(regions[0].edges.len(), 1);
        assert!(regions[1].edges.is_empty());
        map.linedefs[0].right_sidedef = 999;
        assert!(sector_regions(&map, Vec2::ZERO).iter().all(|s| s.edges.is_empty()));
    }
}

impl WadCollisionWorld {
    pub fn vertical_bounds(&self, position: Vec3) -> Option<(f32, f32)> {
        let point = Vec2::new(position.x, position.z);
        self.sectors.iter().find(|sector| sector.contains(point)).map(|sector| (sector.floor, sector.ceiling))
    }

    pub fn floor_height_at(&self, position: Vec3) -> f32 {
        self.vertical_bounds(position).map_or(0.0, |bounds| bounds.0)
    }

    pub fn spawn_for_slot(&self, slot: usize) -> Option<Transform> {
        self.player_spawns.get(slot % self.player_spawns.len().max(1)).copied()
    }

pub fn resolve_motion(&self, start: Vec3, end: Vec3) -> Vec3 {
    if !start.is_finite() {
        return Vec3::ZERO;
    }
    if !end.is_finite() {
        return start;
    }

    let displacement = end - start;
    if displacement.x == 0.0 && displacement.y == 0.0 && displacement.z == 0.0 {
        return start;
    }

    let distance_sq = displacement.length_squared();
    if !distance_sq.is_finite() {
        return start;
    }

    let distance = distance_sq.sqrt();
    if !distance.is_finite() {
        return start;
    }

    let max_step = self.push_radius.max(0.01);
    let substep_length = max_step * 0.5;

    if distance > 256.0 * substep_length {
        return start;
    }

    let num_substeps = (distance / substep_length).ceil() as usize;
    let num_substeps = num_substeps.min(256);

    if num_substeps == 0 {
        return start;
    }

    let step_vec = displacement / num_substeps as f32;
    let mut current = start;

    for _ in 0..num_substeps {
        let requested = current + step_vec;
        current = self.resolve_position(requested);
    }

    current
}

    pub fn resolve_position(&self, position: Vec3) -> Vec3 {
        if self.segments.is_empty() && self.doors.is_empty() {
            return position;
        }

        let mut result = position;
        let mut point = Vec2::new(result.x, result.z);

        for _ in 0..4 {
            let mut changed = false;
            for segment in &self.segments {
                let closest = closest_point_on_segment(point, segment.start, segment.end);
                let delta = point - closest;
                let distance_sq = delta.length_squared();
                let radius = self.push_radius.max(0.01);

                if distance_sq < radius * radius {
                    let distance = distance_sq.sqrt();
                    let normal = if distance > 0.0001 {
                        delta / distance
                    } else {
                        let segment_dir = (segment.end - segment.start).normalize_or_zero();
                        Vec2::new(-segment_dir.y, segment_dir.x)
                    };
                    point += normal * (radius - distance + 0.0005);
                    changed = true;
                }
            }

            for door in &self.doors {
                if door.open_amount >= DOOR_COLLISION_OPEN_THRESHOLD {
                    continue;
                }

                let closest = closest_point_on_segment(point, door.start, door.end);
                let delta = point - closest;
                let distance_sq = delta.length_squared();
                let radius = self.push_radius.max(0.01);

                if distance_sq < radius * radius {
                    let distance = distance_sq.sqrt();
                    let normal = if distance > 0.0001 {
                        delta / distance
                    } else {
                        let segment_dir = (door.end - door.start).normalize_or_zero();
                        Vec2::new(-segment_dir.y, segment_dir.x)
                    };
                    point += normal * (radius - distance + 0.0005);
                    changed = true;
                }
            }

            if !changed {
                break;
            }
        }

        result.x = point.x;
        result.z = point.y;
        result
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;

    fn make_world() -> WadCollisionWorld {
        WadCollisionWorld {
            segments: vec![WallCollisionSegment {
                start: Vec2::new(0.0, 0.0),
                end: Vec2::new(0.0, 10.0),
            }],
            doors: vec![],
            push_radius: 1.0,
            ..Default::default()
        }
    }

    #[test]
    fn test_cross_wall_from_left() {
        let world = make_world();
        let start = Vec3::new(-2.0, 0.0, 5.0);
        let end = Vec3::new(2.0, 0.0, 5.0);
        let result = world.resolve_motion(start, end);
        assert!(result.x <= -world.push_radius, "Should not cross wall from left");
    }

    #[test]
    fn test_cross_wall_from_right() {
        let world = make_world();
        let start = Vec3::new(2.0, 0.0, 5.0);
        let end = Vec3::new(-2.0, 0.0, 5.0);
        let result = world.resolve_motion(start, end);
        assert!(result.x >= world.push_radius, "Should not cross wall from right");
    }

    #[test]
    fn test_move_parallel_to_wall() {
        let world = make_world();
        let start = Vec3::new(2.0, 0.0, 0.0);
        let end = Vec3::new(2.0, 0.0, 10.0);
        let result = world.resolve_motion(start, end);
        assert!((result.x - 2.0).abs() < 0.1, "Should move parallel without deviation");
        assert!((result.z - 10.0).abs() < 0.1, "Should reach end z");
    }

    #[test]
    fn test_unobstructed_movement() {
        let world = make_world();
        let start = Vec3::new(5.0, 0.0, 5.0);
        let end = Vec3::new(6.0, 0.0, 5.0);
        let result = world.resolve_motion(start, end);
        assert!((result.x - 6.0).abs() < 0.01, "Should reach end x");
        assert!((result.z - 5.0).abs() < 0.01, "Should reach end z");
    }

    #[test]
    fn test_zero_motion() {
        let world = make_world();
        let start = Vec3::new(5.0, 0.0, 5.0);
        let end = Vec3::new(5.0, 0.0, 5.0);
        let result = world.resolve_motion(start, end);
        assert_eq!(result, start);
    }

    #[test]
    fn test_excessive_motion() {
        let world = make_world();
        let start = Vec3::new(0.0, 0.0, 0.0);
        let end = Vec3::new(10000.0, 0.0, 0.0);
        let result = world.resolve_motion(start, end);
        assert_eq!(result, start, "Should reject excessive motion");
    }

    #[test]
    fn test_non_finite_input_start() {
        let world = make_world();
        let start = Vec3::new(f32::NAN, 0.0, 0.0);
        let end = Vec3::new(1.0, 0.0, 0.0);
        let result = world.resolve_motion(start, end);
        assert_eq!(result, Vec3::ZERO, "Should return ZERO for non-finite start");
    }

    #[test]
    fn test_non_finite_input_end() {
        let world = make_world();
        let start = Vec3::new(0.0, 0.0, 0.0);
        let end = Vec3::new(f32::INFINITY, 0.0, 0.0);
        let result = world.resolve_motion(start, end);
        assert_eq!(result, start, "Should return start for non-finite end");
    }

    #[test]
    fn test_closed_door_blocks() {
        let mut world = make_world();
        world.segments.clear();
        world.doors.push(WadDoor {
            entity: Entity::PLACEHOLDER,
            debug_entity: Entity::PLACEHOLDER,
            start: Vec2::new(0.0, 0.0),
            end: Vec2::new(0.0, 10.0),
            bottom: 0.0,
            closed_height: 1.0,
            open_amount: 0.0,
            hold_timer_secs: 0.0,
            required_key: None,
        });
        let start = Vec3::new(-2.0, 0.0, 5.0);
        let end = Vec3::new(2.0, 0.0, 5.0);
        let result = world.resolve_motion(start, end);
        assert!(result.x <= -world.push_radius, "Closed door should block crossing");
    }

    #[test]
    fn test_open_door_permits() {
        let mut world = make_world();
        world.segments.clear();
        world.doors.push(WadDoor {
            entity: Entity::PLACEHOLDER,
            debug_entity: Entity::PLACEHOLDER,
            start: Vec2::new(0.0, 0.0),
            end: Vec2::new(0.0, 10.0),
            bottom: 0.0,
            closed_height: 1.0,
            open_amount: 1.0,
            hold_timer_secs: 0.0,
            required_key: None,
        });
        let start = Vec3::new(-2.0, 0.0, 5.0);
        let end = Vec3::new(2.0, 0.0, 5.0);
        let result = world.resolve_motion(start, end);
        assert!((result.x - 2.0).abs() < 0.1, "Open door should permit crossing");
    }
}
fn spawn_headless_level(commands: &mut Commands, path: &str, name: &str) -> Result<LoadedLevel, String> {
    let mut meshes = Assets::<Mesh>::default();
    let mut materials = Assets::<StandardMaterial>::default();
    try_spawn_level_from_selection(commands, &mut meshes, &mut materials, None, path, name)
}

pub fn setup_headless_level(
    mut commands: Commands,
    mut ergo: ResMut<crate::config::HumanErgoConfig>,
    mut exit: MessageWriter<bevy::app::AppExit>,
) {
    let Ok(path) = std::env::var("DOOM_WAD") else { return; };
    let name = std::env::var("DOOM_MAP").unwrap_or_else(|_| "MAP01".to_owned());
    match spawn_headless_level(&mut commands, &path, &name) {
        Ok(level) => ergo.movement.plane_limit = level.info.suggested_plane_limit,
        Err(error) => {
            eprintln!("[server] Cannot load the selected map: {error}");
            exit.write(bevy::app::AppExit::error());
        }
    }
}

pub fn spawn_level_from_env(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: Option<&mut Assets<Image>>,
) -> Option<LevelSpawnInfo> {
    let Some(wad_path) = std::env::var("DOOM_WAD").ok() else {
        commands.remove_resource::<WadCollisionWorld>();
        return None;
    };
    let map_name = std::env::var("DOOM_MAP").unwrap_or_else(|_| "MAP01".to_string());
    spawn_level_from_selection(commands, meshes, materials, images, &wad_path, &map_name)
}

pub fn update_wad_doors(
    time: Res<Time>,
    menu_state: Option<Res<crate::ui::EscapeMenuState>>,
    collision_debug: Option<Res<crate::scene::CollisionDebugState>>,
    world: Option<ResMut<WadCollisionWorld>>,
    player_query: Query<&Transform, With<crate::RotatingCube>>,
    mut door_transforms: Query<&mut Transform, Without<crate::RotatingCube>>,
    mut debug_visuals: Query<&mut Visibility, With<crate::scene::DoorCollisionDebugVisual>>,
) {
    if let Some(menu_state) = menu_state && menu_state.is_open {
        return;
    }

    let Some(mut world) = world else {
        return;
    };
    if world.doors.is_empty() {
        return;
    }

    let player_pos = {
        let Ok(player_transform) = player_query.single() else {
            return;
        };
        Vec2::new(player_transform.translation.x, player_transform.translation.z)
    };

    let dt = time.delta_secs().clamp(0.0, 0.1);
    let debug_visible = collision_debug.map(|state| state.visible).unwrap_or(false);

    world.doors.retain_mut(|door| {
        if door.closed_height <= WALL_MIN_HEIGHT {
            return false;
        }

        let near_door = point_segment_distance(player_pos, door.start, door.end) <= DOOR_TRIGGER_DISTANCE;

        if near_door {
            door.hold_timer_secs = DOOR_HOLD_SECS;
        } else {
            door.hold_timer_secs = (door.hold_timer_secs - dt).max(0.0);
        }

        let target_open = if door.hold_timer_secs > 0.0 { 1.0 } else { 0.0 };
        if door.open_amount < target_open {
            door.open_amount = (door.open_amount + DOOR_OPEN_SPEED * dt).min(target_open);
        } else if door.open_amount > target_open {
            door.open_amount = (door.open_amount - DOOR_OPEN_SPEED * dt).max(target_open);
        }
        door.open_amount = door.open_amount.clamp(0.0, 1.0);

        if let Ok(mut transform) = door_transforms.get_mut(door.entity) {
            let scale_y = (1.0 - door.open_amount).max(MIN_DOOR_SCALE_Y);
            transform.scale.y = scale_y;
            transform.translation.y = door.bottom
                + (door.closed_height * scale_y) * 0.5
                + door.closed_height * door.open_amount;

            if let Ok(mut visibility) = debug_visuals.get_mut(door.debug_entity) {
                *visibility = if debug_visible && door.open_amount < DOOR_COLLISION_OPEN_THRESHOLD {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                };
            }
            true
        } else {
            // Door entity was despawned by a level reload or cleanup, drop stale runtime state.
            false
        }
    });
}

pub fn spawn_level_from_selection(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: Option<&mut Assets<Image>>,
    wad_path: &str,
    map_name: &str,
) -> Option<LevelSpawnInfo> {
    match try_spawn_level_from_selection(commands, meshes, materials, images, wad_path, map_name) {
        Ok(loaded) => Some(loaded.info),
        Err(error) => {
            println!(
                "[wad] failed to load map {} from {}: {} (falling back to default floor)",
                map_name, wad_path, error
            );
            None
        }
    }
}

pub fn try_spawn_level_from_selection(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: Option<&mut Assets<Image>>,
    wad_path: &str,
    map_name: &str,
) -> Result<LoadedLevel, String> {
    match spawn_level_from_wad(commands, meshes, materials, images, wad_path, map_name) {
        Ok((info, collision_world, resolved_map_name, source_label)) => {
            commands.insert_resource(collision_world);
            println!("[wad] loaded map {} from {}", resolved_map_name, source_label);
            Ok(LoadedLevel {
                info,
                map_name: resolved_map_name,
                source_label,
            })
        }
        Err(error) => {
            commands.remove_resource::<WadCollisionWorld>();
            Err(error)
        }
    }
}

/// Prepare every sector surface before the loader creates any entities.
fn prepare_sector_surfaces(
    map: &ParsedMap, regions: &[WadSectorRegion], center: Vec2,
) -> Result<Vec<(Mesh, String)>, String> {
    const MAX_MAP_SCAN_WORK: usize = 16_000_000;
    const MAX_MAP_SURFACE_VERTICES: usize = 1_500_000;
    if map.sectors.len() != regions.len() {
        return Err("Sector geometry does not match the map".into());
    }
    let mut surfaces = Vec::new();
    let mut scan_work = 0usize;
    let mut vertex_count = 0usize;
    for (index, (sector, region)) in map.sectors.iter().zip(regions).enumerate() {
        let work = region.edges.len().checked_mul(region.edges.len()).and_then(|v| v.checked_mul(2));
        scan_work = work.and_then(|v| scan_work.checked_add(v))
            .filter(|v| *v <= MAX_MAP_SCAN_WORK)
            .ok_or_else(|| "Map exceeds the surface scan limit".to_owned())?;
        let edges: Vec<_> = region.edges.iter().map(|edge| (edge.start, edge.end)).collect();
        for (height, ceiling, texture) in [
            (region.floor, false, &sector.floor_texture),
            (region.ceiling, true, &sector.ceiling_texture),
        ] {
            // A sky flat exposes the sky rather than an opaque plane.
            if texture.eq_ignore_ascii_case("F_SKY1") { continue; }
            // One unmeshable sector (open or self-crossing boundary) loses
            // only its own surface; the map keeps loading and the sector
            // keeps its collision region.
            let mesh = match crate::map_surfaces::horizontal_mesh(
                &edges, height, ceiling, center, 64.0 * DOOM_TO_WORLD_SCALE,
            ) {
                Ok(mesh) => mesh,
                Err(error) => {
                    warn!("[wad] Cannot mesh sector {index}: {error}. The surface is skipped.");
                    continue;
                }
            };
            let count = mesh.count_vertices();
            vertex_count = vertex_count.checked_add(count)
                .filter(|v| *v <= MAX_MAP_SURFACE_VERTICES)
                .ok_or_else(|| "Map exceeds the surface vertex limit".to_owned())?;
            if count > 0 { surfaces.push((mesh, clean_texture_name(texture))); }
        }
    }
    Ok(surfaces)
}

fn prepare_flat_images(
    wad_bytes: &[u8],
    lumps: &[LumpEntry],
    requested: &std::collections::BTreeSet<String>,
) -> Result<std::collections::BTreeMap<String, Image>, String> {
    // Limit decoded RGBA pixels to 32 MiB per level.
    if requested.len() > 2048 || lumps.len() > crate::doom_flats::MAX_LUMP_NAMES {
        return Err("WAD exceeds the flat texture budget".into());
    }
    let mut decoded = std::collections::BTreeMap::new();
    let Some(palette) = lumps.iter().rev().find(|entry| entry.name == "PLAYPAL") else {
        warn!("[wad] No PLAYPAL in this WAD. Flat textures use placeholders.");
        return Ok(decoded);
    };
    let bytes = |entry: &LumpEntry| -> Result<&[u8], String> {
        entry.offset.checked_add(entry.size)
            .and_then(|end| wad_bytes.get(entry.offset..end))
            .ok_or_else(|| "Flat resource range exceeds the WAD".to_owned())
    };
    // A damaged palette degrades every flat at once, so fall back to
    // placeholders in one step instead of failing the whole map.
    let Ok(palette) = bytes(palette) else {
        warn!("[wad] PLAYPAL lies outside the WAD data. Flat textures use placeholders.");
        return Ok(decoded);
    };
    if palette.len() < 768 {
        warn!("[wad] PLAYPAL is shorter than 768 bytes. Flat textures use placeholders.");
        return Ok(decoded);
    }
    let names: Vec<_> = lumps.iter().map(|entry| entry.name.as_str()).collect();
    let flat_indices = crate::doom_flats::flat_indices(&names)?;
    for name in requested {
        let Some(entry) = flat_indices.get(name).and_then(|index| lumps.get(*index)) else {
            warn!("[wad] Flat {name} is missing from this WAD. It uses a placeholder.");
            continue;
        };
        // One damaged flat must not reject the whole map. Warn and fall back
        // to the placeholder, matching the missing-flat convention above.
        let Ok(raw) = bytes(entry) else {
            warn!("[wad] Flat {name} lies outside the WAD data. It uses a placeholder.");
            continue;
        };
        match crate::doom_flats::flat_image(raw, palette) {
            Ok(image) => {
                decoded.insert(name.clone(), image);
            }
            Err(error) => warn!("[wad] Cannot decode flat {name}: {error}. It uses a placeholder."),
        }
    }
    info!("[wad] flat textures: {}/{} decoded (static, base palette)", decoded.len(), requested.len());
    Ok(decoded)
}

fn spawn_level_from_wad(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: Option<&mut Assets<Image>>,
    wad_path: &str,
    map_name: &str,
) -> Result<(LevelSpawnInfo, WadCollisionWorld, String, String), String> {
    let map_source = load_map_source(wad_path, map_name)?;
    let mut source_lumps = Vec::new();
    let map = match &map_source.map_data {
        MapSourceData::ClassicWad(wad_bytes) => {
            source_lumps = parse_lump_directory(wad_bytes)?;
            parse_map_lumps(wad_bytes, &source_lumps, &map_source.resolved_map_name)?
        }
        MapSourceData::Udmf(map) => map.clone(),
    };

    if map.vertices.is_empty() || map.linedefs.is_empty() {
        return Err("map does not contain enough geometry".to_string());
    }

    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);

    for vertex in &map.vertices {
        let point = doom_to_world(Vec2::new(vertex.x as f32, vertex.y as f32));
        min = min.min(point);
        max = max.max(point);
    }

    let center = (min + max) * 0.5;
    let sectors = sector_regions(&map, center);
    let surfaces = prepare_sector_surfaces(&map, &sectors, center)?;
    let mut flat_materials = std::collections::BTreeMap::new();
    if let Some(images) = images {
        if let MapSourceData::ClassicWad(bytes) = &map_source.map_data {
            let requested = surfaces.iter().map(|(_, name)| name.clone()).collect();
            let decoded = prepare_flat_images(bytes, &source_lumps, &requested)?;
            for (name, image) in decoded {
                let texture = images.add(image);
                let material = materials.add(StandardMaterial {
                    base_color: Color::WHITE,
                    base_color_texture: Some(texture),
                    perceptual_roughness: 1.0,
                    // Sector COLORMAP lighting is not implemented yet.
                    unlit: true,
                    ..default()
                });
                flat_materials.insert(name, material);
            }
        } else {
            warn!("[wad] UDMF external flat resources are not supported. Flats use placeholders.");
        }
    }

    let mut collision_segments = Vec::new();
    let mut doors = Vec::new();

    for linedef in &map.linedefs {
        let Some(start_vertex) = map.vertices.get(linedef.start_vertex as usize) else {
            continue;
        };
        let Some(end_vertex) = map.vertices.get(linedef.end_vertex as usize) else {
            continue;
        };

        let start = doom_to_world(Vec2::new(start_vertex.x as f32, start_vertex.y as f32)) - center;
        let end = doom_to_world(Vec2::new(end_vertex.x as f32, end_vertex.y as f32)) - center;

        if (end - start).length_squared() <= 0.000001 {
            continue;
        }

        let right_side = map.sidedefs.get(linedef.right_sidedef as usize);
        let left_side = map
            .sidedefs
            .get(linedef.left_sidedef as usize)
            .filter(|_| linedef.left_sidedef != NO_SIDEDEF);

        let right_sector = right_side.and_then(|side| map.sectors.get(side.sector as usize));
        let left_sector = left_side.and_then(|side| map.sectors.get(side.sector as usize));

        let right_floor = right_sector
            .map(|sector| sector.floor_height as f32 * DOOM_TO_WORLD_SCALE)
            .unwrap_or(0.0);
        let right_ceil = right_sector
            .map(|sector| sector.ceiling_height as f32 * DOOM_TO_WORLD_SCALE)
            .unwrap_or(2.5);

        if let Some(left_sector) = left_sector {
            let left_floor = left_sector.floor_height as f32 * DOOM_TO_WORLD_SCALE;
            let left_ceil = left_sector.ceiling_height as f32 * DOOM_TO_WORLD_SCALE;

            let decoded_special = decode_linedef_special(linedef.special);
            let is_door = is_door_special(linedef.special);

            if is_door {
                let bottom = right_floor.min(left_floor);
                let top = right_ceil.max(left_ceil);
                let door_height = (top - bottom).max(WALL_MIN_HEIGHT);
                let door_entity = spawn_door_panel(
                    commands,
                    meshes,
                    materials,
                    start,
                    end,
                    bottom,
                    door_height,
                );
                let debug_entity = spawn_collision_debug_segment(commands, meshes, materials, start, end, true);

                let _door_tag = linedef.tag;

                doors.push(WadDoor {
                    entity: door_entity,
                    debug_entity,
                    start,
                    end,
                    bottom,
                    closed_height: door_height,
                    open_amount: 0.0,
                    hold_timer_secs: 0.0,
                    required_key: decoded_special.and_then(|special| special.required_key),
                });
                continue;
            }

            let traversable_step = is_traversable_step(right_floor, left_floor);

            let lower_bottom = right_floor.min(left_floor);
            let lower_top = right_floor.max(left_floor);
            if lower_top - lower_bottom > WALL_MIN_HEIGHT {
                let lower_texture = choose_lower_texture(right_side, left_side, right_floor, left_floor);
                spawn_wall_band(commands, meshes, materials, start, end, lower_bottom, lower_top, lower_texture);
            }

            let upper_bottom = right_ceil.min(left_ceil);
            let upper_top = right_ceil.max(left_ceil);
            if upper_top - upper_bottom > WALL_MIN_HEIGHT {
                let upper_texture = choose_upper_texture(right_side, left_side, right_ceil, left_ceil);
                spawn_wall_band(commands, meshes, materials, start, end, upper_bottom, upper_top, upper_texture);
            }

            if !traversable_step
                && ((right_floor - left_floor).abs() > WALL_MIN_HEIGHT
                    || (right_ceil - left_ceil).abs() > WALL_MIN_HEIGHT)
            {
                collision_segments.push(WallCollisionSegment { start, end });
                spawn_collision_debug_segment(commands, meshes, materials, start, end, false);
            }
        } else {
            let middle_texture = right_side
                .map(|side| clean_texture_name(&side.middle_texture))
                .unwrap_or("METAL".to_string());
            spawn_wall_band(commands, meshes, materials, start, end, right_floor, right_ceil, middle_texture);
            collision_segments.push(WallCollisionSegment { start, end });
            spawn_collision_debug_segment(commands, meshes, materials, start, end, false);
        }
    }

    let extent = (max - min) * 0.5 + Vec2::splat(FLOOR_PADDING);
    let floor_half_extent = extent
        .x
        .max(extent.y)
        .max(MIN_FLOOR_HALF_EXTENT);

    for (mesh, texture_name) in surfaces {
        let material = flat_materials.get(&texture_name).cloned()
            .unwrap_or_else(|| materials.add(texture_to_material(&texture_name)));
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material),
            Transform::default(),
            GlobalTransform::default(),
            crate::scene::LevelGeometry,
        ));
    }

    let player_spawns = find_multiplayer_spawns(&map).iter().map(|spawn| {
        let point = doom_to_world(Vec2::new(spawn.x as f32, spawn.y as f32)) - center;
        let floor = sectors.iter().find(|sector| sector.contains(point)).map_or(0.0, |sector| sector.floor);
        Transform::from_xyz(point.x, floor + crate::player::CUBE_REST_Y, point.y)
            .with_rotation(Quat::from_rotation_y(doom_angle_to_yaw(spawn.angle_deg)))
    }).collect();

    let (player_spawn, player_yaw) = find_player_spawn_for_slot(&map, 0)
        .map(|spawn| {
            let flat = doom_to_world(Vec2::new(spawn.x as f32, spawn.y as f32)) - center;
            let floor = sectors.iter().find(|sector| sector.contains(flat)).map_or(0.0, |sector| sector.floor);
            let spawn_pos = Vec3::new(flat.x, floor + crate::player::CUBE_REST_Y, flat.y);
            (spawn_pos, doom_angle_to_yaw(spawn.angle_deg))
        })
        .unwrap_or((Vec3::new(0.0, crate::player::CUBE_REST_Y, 0.0), 0.0));

    let suggested_plane_limit = floor_half_extent + FLOOR_PADDING;

    let spawn_info = LevelSpawnInfo {
        player_spawn,
        player_yaw,
        suggested_plane_limit,
    };

    let collision_world = WadCollisionWorld {
        segments: collision_segments,
        doors,
        push_radius: COLLISION_PADDING,
        sectors,
        player_spawns,
    };

    Ok((
        spawn_info,
        collision_world,
        map_source.resolved_map_name,
        map_source.source_label,
    ))
}

fn load_map_source(container_path: &str, requested_map_name: &str) -> Result<MapSource, String> {
    match detect_map_container_kind(container_path) {
        Ok(MapContainerKind::Wad) => load_map_source_from_wad_file(container_path, requested_map_name),
        Ok(MapContainerKind::ZipLike) => load_map_source_from_pk3(container_path, requested_map_name),
        Ok(MapContainerKind::Unknown) => {
            let wad_try = load_map_source_from_wad_file(container_path, requested_map_name);
            if wad_try.is_ok() {
                return wad_try;
            }

            let pk3_try = load_map_source_from_pk3(container_path, requested_map_name);
            if pk3_try.is_ok() {
                return pk3_try;
            }

            Err(format!(
                "unsupported map container: {} (expected WAD/PK3/ZIP)",
                container_path
            ))
        }
        Err(error) => Err(error),
    }
}

#[derive(Debug, Clone, Copy)]
enum MapContainerKind {
    Wad,
    ZipLike,
    Unknown,
}

fn detect_map_container_kind(path: &str) -> Result<MapContainerKind, String> {
    let mut file = std::fs::File::open(Path::new(path))
        .map_err(|error| format!("unable to read map container: {error}"))?;
    let mut header = [0u8; 4];
    let read_count = std::io::Read::read(&mut file, &mut header)
        .map_err(|error| format!("unable to inspect map container: {error}"))?;

    if read_count >= 4 && (&header == b"IWAD" || &header == b"PWAD") {
        return Ok(MapContainerKind::Wad);
    }

    if read_count >= 4 && header == [0x50, 0x4B, 0x03, 0x04] {
        return Ok(MapContainerKind::ZipLike);
    }

    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".wad") {
        return Ok(MapContainerKind::Wad);
    }
    if lower.ends_with(".pk3") || lower.ends_with(".zip") {
        return Ok(MapContainerKind::ZipLike);
    }

    Ok(MapContainerKind::Unknown)
}

fn load_map_source_from_wad_file(path: &str, requested_map_name: &str) -> Result<MapSource, String> {
    let wad_bytes = std::fs::read(Path::new(path))
        .map_err(|error| format!("unable to read WAD file: {error}"))?;
    let lumps = parse_lump_directory(&wad_bytes)?;
    let resolved_map_name = resolve_map_marker(&lumps, requested_map_name)?;
    Ok(MapSource {
        map_data: MapSourceData::ClassicWad(wad_bytes),
        resolved_map_name,
        source_label: path.to_string(),
    })
}

fn load_map_source_from_pk3(path: &str, requested_map_name: &str) -> Result<MapSource, String> {
    let file = std::fs::File::open(Path::new(path))
        .map_err(|error| format!("unable to read PK3 file: {error}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("unable to open PK3 archive: {error}"))?;

    let requested_upper = requested_map_name.trim().to_ascii_uppercase();
    let mut exact_match: Option<MapSource> = None;
    let mut fallback_match: Option<MapSource> = None;
    let mut wad_entry_count = 0usize;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("unable to read PK3 entry: {error}"))?;
        let entry_name = entry.name().to_string();
        let entry_name_lower = entry_name.to_ascii_lowercase();

        if !entry_name_lower.ends_with(".wad") {
            continue;
        }
        wad_entry_count += 1;

        let wad_bytes = read_entry_bytes_capped(&mut entry, &entry_name)?;

        let Ok(lumps) = parse_lump_directory(&wad_bytes) else {
            continue;
        };

        let resolved_map_name = match resolve_map_marker(&lumps, requested_map_name) {
            Ok(map_name) => map_name,
            Err(_) => continue,
        };

        let source_label = format!("{}::{}", path, entry_name);
        let source = MapSource {
            map_data: MapSourceData::ClassicWad(wad_bytes),
            resolved_map_name,
            source_label,
        };

        let has_exact = lumps.iter().any(|entry| entry.name == requested_upper);
        if has_exact {
            exact_match = Some(source);
            break;
        }

        if fallback_match.is_none() {
            fallback_match = Some(source);
        }
    }

    if let Some(source) = exact_match.or(fallback_match) {
        return Ok(source);
    }

    if let Some(source) = load_udmf_map_source_from_pk3(path, requested_map_name)? {
        return Ok(source);
    }

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("unable to read PK3 entry: {error}"))?;
        let entry_name = entry.name().to_string();
        let entry_name_lower = entry_name.to_ascii_lowercase();
        if !entry_name_lower.ends_with(".pk3") && !entry_name_lower.ends_with(".zip") {
            continue;
        }

        let nested_bytes = read_entry_bytes_capped(&mut entry, &entry_name)?;

        if let Some(source) = load_map_source_from_zip_blob(
            &nested_bytes,
            &format!("{}::{}", path, entry_name),
            requested_map_name,
        )? {
            return Ok(source);
        }
    }

    if wad_entry_count == 0 {
        Err(format!(
            "no embedded WAD, UDMF TEXTMAP, or nested PK3 in {} produced a loadable map for {}",
            path, requested_map_name
        ))
    } else {
        Err(format!(
            "embedded WAD entries in {} did not provide a usable map marker for {}",
            path, requested_map_name
        ))
    }
}

fn load_map_source_from_zip_blob(
    zip_bytes: &[u8],
    source_prefix: &str,
    requested_map_name: &str,
) -> Result<Option<MapSource>, String> {
    load_map_source_from_zip_blob_inner(zip_bytes, source_prefix, requested_map_name, 0)
}

fn load_map_source_from_zip_blob_inner(
    zip_bytes: &[u8],
    source_prefix: &str,
    requested_map_name: &str,
    depth: usize,
) -> Result<Option<MapSource>, String> {
    const MAX_NESTED_DEPTH: usize = 2;
    if depth > MAX_NESTED_DEPTH {
        return Ok(None);
    }

    let cursor = Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|error| format!("unable to open nested archive {}: {error}", source_prefix))?;

    let requested_upper = requested_map_name.trim().to_ascii_uppercase();
    let mut exact_match: Option<MapSource> = None;
    let mut fallback_match: Option<MapSource> = None;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("unable to read nested entry: {error}"))?;
        let entry_name = entry.name().to_string();
        let entry_name_lower = entry_name.to_ascii_lowercase();

        if !entry_name_lower.ends_with(".wad") {
            continue;
        }

        let wad_bytes = read_entry_bytes_capped(&mut entry, &entry_name)?;

        let Ok(lumps) = parse_lump_directory(&wad_bytes) else {
            continue;
        };

        let resolved_map_name = match resolve_map_marker(&lumps, requested_map_name) {
            Ok(map_name) => map_name,
            Err(_) => continue,
        };

        let source = MapSource {
            map_data: MapSourceData::ClassicWad(wad_bytes),
            resolved_map_name,
            source_label: format!("{}::{}", source_prefix, entry_name),
        };

        let has_exact = lumps.iter().any(|entry| entry.name == requested_upper);
        if has_exact {
            exact_match = Some(source);
            break;
        }

        if fallback_match.is_none() {
            fallback_match = Some(source);
        }
    }

    if let Some(source) = exact_match.or_else(|| fallback_match.clone()) {
        return Ok(Some(source));
    }

    // Fallback: search direct UDMF TEXTMAP entries in nested archives.
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("unable to read nested entry: {error}"))?;
        let entry_name = entry.name().to_string();
        let Some(map_name) = infer_map_name_from_textmap_path(&entry_name) else {
            continue;
        };

        let text = read_entry_string_capped(&mut entry, &entry_name)?;

        let parsed_map = match parse_udmf_textmap(&text) {
            Ok(map) => map,
            Err(_) => continue,
        };

        if parsed_map.vertices.is_empty() || parsed_map.linedefs.is_empty() {
            continue;
        }

        let requested_upper = requested_map_name.trim().to_ascii_uppercase();
        let source = MapSource {
            map_data: MapSourceData::Udmf(parsed_map),
            resolved_map_name: map_name.clone(),
            source_label: format!("{}::{}", source_prefix, entry_name),
        };

        if map_name == requested_upper {
            return Ok(Some(source));
        }

        if fallback_match.is_none() {
            fallback_match = Some(source);
        }
    }

    if let Some(source) = fallback_match {
        return Ok(Some(source));
    }

    // Final fallback: recurse into nested pk3/zip entries.
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("unable to read nested entry: {error}"))?;
        let entry_name = entry.name().to_string();
        let entry_name_lower = entry_name.to_ascii_lowercase();
        if !entry_name_lower.ends_with(".pk3") && !entry_name_lower.ends_with(".zip") {
            continue;
        }

        let nested_bytes = read_entry_bytes_capped(&mut entry, &entry_name)?;

        if let Some(source) = load_map_source_from_zip_blob_inner(
            &nested_bytes,
            &format!("{}::{}", source_prefix, entry_name),
            requested_map_name,
            depth + 1,
        )? {
            return Ok(Some(source));
        }
    }

    Ok(None)
}

fn load_udmf_map_source_from_pk3(path: &str, requested_map_name: &str) -> Result<Option<MapSource>, String> {
    let file = std::fs::File::open(Path::new(path))
        .map_err(|error| format!("unable to read PK3 file: {error}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("unable to open PK3 archive: {error}"))?;

    let requested_upper = requested_map_name.trim().to_ascii_uppercase();
    let mut exact_match: Option<MapSource> = None;
    let mut fallback_match: Option<MapSource> = None;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("unable to read PK3 entry: {error}"))?;
        let entry_name = entry.name().to_string();
        let Some(map_name) = infer_map_name_from_textmap_path(&entry_name) else {
            continue;
        };

        let text = read_entry_string_capped(&mut entry, &entry_name)?;

        let parsed_map = match parse_udmf_textmap(&text) {
            Ok(map) => map,
            Err(_) => continue,
        };

        if parsed_map.vertices.is_empty() || parsed_map.linedefs.is_empty() {
            continue;
        }

        let source = MapSource {
            map_data: MapSourceData::Udmf(parsed_map),
            resolved_map_name: map_name.clone(),
            source_label: format!("{}::{}", path, entry_name),
        };

        if map_name == requested_upper {
            exact_match = Some(source);
            break;
        }

        if fallback_match.is_none() {
            fallback_match = Some(source);
        }
    }

    Ok(exact_match.or(fallback_match))
}

fn resolve_map_marker(lumps: &[LumpEntry], requested_map_name: &str) -> Result<String, String> {
    let requested = requested_map_name.trim().to_ascii_uppercase();
    if lumps.iter().any(|entry| entry.name == requested) {
        return Ok(requested);
    }

    let markers = find_map_markers(lumps);

    if requested == "MAP01"
        && markers.iter().any(|name| name == "E1M1") {
            return Ok("E1M1".to_string());
        }

    if let Some(first) = markers.first() {
        println!(
            "[wad] requested map {} not found; using {}",
            requested, first
        );
        return Ok(first.clone());
    }

    Err(format!(
        "map marker {} not found in WAD and no valid map markers were detected",
        requested
    ))
}

fn find_map_markers(lumps: &[LumpEntry]) -> Vec<String> {
    let mut markers = Vec::new();

    for index in 0..lumps.len() {
        let name = &lumps[index].name;
        if !is_map_marker_name(name) {
            continue;
        }

        let tail = &lumps[index + 1..lumps.len().min(index + 12)];
        let has_vertices = tail.iter().any(|entry| entry.name == "VERTEXES");
        let has_linedefs = tail.iter().any(|entry| entry.name == "LINEDEFS");
        let has_sidedefs = tail.iter().any(|entry| entry.name == "SIDEDEFS");
        let has_sectors = tail.iter().any(|entry| entry.name == "SECTORS");

        if has_vertices && has_linedefs && has_sidedefs && has_sectors {
            markers.push(name.clone());
        }
    }

    markers
}

fn is_map_marker_name(name: &str) -> bool {
    if name.len() == 5 {
        let bytes = name.as_bytes();
        return bytes[0] == b'M'
            && bytes[1] == b'A'
            && bytes[2] == b'P'
            && bytes[3].is_ascii_digit()
            && bytes[4].is_ascii_digit();
    }

    if name.len() == 4 {
        let bytes = name.as_bytes();
        return bytes[0] == b'E'
            && bytes[1].is_ascii_digit()
            && bytes[2] == b'M'
            && bytes[3].is_ascii_digit();
    }

    false
}

fn parse_lump_directory(wad_bytes: &[u8]) -> Result<Vec<LumpEntry>, String> {
    if wad_bytes.len() < 12 {
        return Err("WAD header too small".to_string());
    }

    let signature = &wad_bytes[0..4];
    if signature != b"IWAD" && signature != b"PWAD" {
        return Err("invalid WAD signature (expected IWAD or PWAD)".to_string());
    }

    let lump_count = read_i32_le(wad_bytes, 4)?;
    let directory_offset = read_i32_le(wad_bytes, 8)?;

    if lump_count < 0 || directory_offset < 0 {
        return Err("WAD has negative lump metadata".to_string());
    }

    let lump_count = lump_count as usize;
    let directory_offset = directory_offset as usize;

    let directory_size = lump_count
        .checked_mul(16)
        .ok_or_else(|| "WAD directory size overflow".to_string())?;

    let directory_end = directory_offset
        .checked_add(directory_size)
        .ok_or_else(|| "WAD directory end overflow".to_string())?;

    if directory_end > wad_bytes.len() {
        return Err("WAD directory extends past file size".to_string());
    }

    let mut entries = Vec::with_capacity(lump_count);
    for index in 0..lump_count {
        let cursor = directory_offset + index * 16;

        let offset = read_i32_le(wad_bytes, cursor)?;
        let size = read_i32_le(wad_bytes, cursor + 4)?;

        if offset < 0 || size < 0 {
            return Err(format!("lump {} has negative offset or size", index));
        }

        let offset = offset as usize;
        let size = size as usize;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| format!("lump {} range overflow", index))?;

        if end > wad_bytes.len() {
            return Err(format!("lump {} range exceeds file size", index));
        }

        let name_bytes = &wad_bytes[cursor + 8..cursor + 16];
        let null_index = name_bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(name_bytes.len());
        let name = String::from_utf8_lossy(&name_bytes[..null_index])
            .trim()
            .to_ascii_uppercase();

        entries.push(LumpEntry { offset, size, name });
    }

    Ok(entries)
}

fn parse_map_lumps(wad_bytes: &[u8], lumps: &[LumpEntry], map_name: &str) -> Result<ParsedMap, String> {
    let map_name = map_name.trim().to_ascii_uppercase();

    let marker_index = lumps
        .iter()
        .position(|entry| entry.name == map_name)
        .ok_or_else(|| format!("map marker {} not found in WAD", map_name))?;

    let search_slice = &lumps[marker_index + 1..];

    let vertices_lump = search_slice
        .iter()
        .find(|entry| entry.name == "VERTEXES")
        .ok_or_else(|| format!("map {} is missing VERTEXES lump", map_name))?;
    let linedefs_lump = search_slice
        .iter()
        .find(|entry| entry.name == "LINEDEFS")
        .ok_or_else(|| format!("map {} is missing LINEDEFS lump", map_name))?;
    let sidedefs_lump = search_slice
        .iter()
        .find(|entry| entry.name == "SIDEDEFS")
        .ok_or_else(|| format!("map {} is missing SIDEDEFS lump", map_name))?;
    let sectors_lump = search_slice
        .iter()
        .find(|entry| entry.name == "SECTORS")
        .ok_or_else(|| format!("map {} is missing SECTORS lump", map_name))?;

    let things_lump = search_slice.iter().find(|entry| entry.name == "THINGS");

    // A Hexen-format map carries a BEHAVIOR lump in its directory section;
    // Doom/Boom maps never do. Bound the scan at the next map marker so a
    // later Hexen map in a mixed PWAD cannot flip this map's format. Size
    // divisibility alone is ambiguous: a Hexen LINEDEFS lump whose record
    // count is a multiple of 7 is also divisible by 14 and would silently
    // misparse as Doom records with shifted fields.
    let section_end = search_slice
        .iter()
        .position(|entry| is_map_marker_name(&entry.name))
        .unwrap_or(search_slice.len());
    let hexen_format = search_slice[..section_end]
        .iter()
        .any(|entry| entry.name == "BEHAVIOR");

    let vertices = parse_vertices(wad_bytes, vertices_lump)?;
    let linedefs = parse_linedefs(wad_bytes, linedefs_lump, hexen_format)?;
    let sidedefs = parse_sidedefs(wad_bytes, sidedefs_lump)?;
    let sectors = parse_sectors(wad_bytes, sectors_lump)?;
    let things = if let Some(things_lump) = things_lump {
        parse_things(wad_bytes, things_lump)?
    } else {
        Vec::new()
    };

    Ok(ParsedMap {
        vertices,
        linedefs,
        sidedefs,
        sectors,
        things,
    })
}

fn parse_vertices(wad_bytes: &[u8], entry: &LumpEntry) -> Result<Vec<Vertex>, String> {
    if !entry.size.is_multiple_of(4) {
        return Err("VERTEXES lump size is not divisible by 4".to_string());
    }

    let count = entry.size / 4;
    let mut vertices = Vec::with_capacity(count);

    for index in 0..count {
        let cursor = entry.offset + index * 4;
        let x = read_i16_le(wad_bytes, cursor)?;
        let y = read_i16_le(wad_bytes, cursor + 2)?;
        vertices.push(Vertex { x, y });
    }

    Ok(vertices)
}

fn parse_linedefs(wad_bytes: &[u8], entry: &LumpEntry, hexen_format: bool) -> Result<Vec<LineDef>, String> {
    // Doom format uses 14-byte linedefs, Hexen format uses 16-byte linedefs.
    // The map directory's BEHAVIOR lump decides when present; divisibility is
    // only a fallback, because a Hexen lump with a record count that is a
    // multiple of 7 is also divisible by 14.
    if !hexen_format && entry.size.is_multiple_of(14) {
        let count = entry.size / 14;
        let mut linedefs = Vec::with_capacity(count);

        for index in 0..count {
            let cursor = entry.offset + index * 14;
            let start_vertex = read_u16_le(wad_bytes, cursor)?;
            let end_vertex = read_u16_le(wad_bytes, cursor + 2)?;
            let special = read_u16_le(wad_bytes, cursor + 6)?;
            let tag = read_u16_le(wad_bytes, cursor + 8)?;
            let right_sidedef = read_u16_le(wad_bytes, cursor + 10)?;
            let left_sidedef = read_u16_le(wad_bytes, cursor + 12)?;
            linedefs.push(LineDef {
                start_vertex,
                end_vertex,
                special,
                tag,
                right_sidedef,
                left_sidedef,
            });
        }

        return Ok(linedefs);
    }

    if entry.size.is_multiple_of(16) {
        let count = entry.size / 16;
        let mut linedefs = Vec::with_capacity(count);

        for index in 0..count {
            let cursor = entry.offset + index * 16;
            let start_vertex = read_u16_le(wad_bytes, cursor)?;
            let end_vertex = read_u16_le(wad_bytes, cursor + 2)?;
            let special = read_u8(wad_bytes, cursor + 6)? as u16;
            // Hexen linedefs store action args instead of tag in this slot.
            let tag = 0;
            let right_sidedef = read_u16_le(wad_bytes, cursor + 12)?;
            let left_sidedef = read_u16_le(wad_bytes, cursor + 14)?;
            linedefs.push(LineDef {
                start_vertex,
                end_vertex,
                special,
                tag,
                right_sidedef,
                left_sidedef,
            });
        }

        return Ok(linedefs);
    }

    Err(format!(
        "LINEDEFS lump size {} is not divisible by 14 (Doom) or 16 (Hexen)",
        entry.size
    ))
}

fn parse_sidedefs(wad_bytes: &[u8], entry: &LumpEntry) -> Result<Vec<SideDef>, String> {
    if !entry.size.is_multiple_of(30) {
        return Err("SIDEDEFS lump size is not divisible by 30".to_string());
    }

    let count = entry.size / 30;
    let mut sidedefs = Vec::with_capacity(count);

    for index in 0..count {
        let cursor = entry.offset + index * 30;
        sidedefs.push(SideDef {
            upper_texture: read_name8(wad_bytes, cursor + 4)?,
            lower_texture: read_name8(wad_bytes, cursor + 12)?,
            middle_texture: read_name8(wad_bytes, cursor + 20)?,
            sector: read_u16_le(wad_bytes, cursor + 28)?,
        });
    }

    Ok(sidedefs)
}

fn parse_sectors(wad_bytes: &[u8], entry: &LumpEntry) -> Result<Vec<Sector>, String> {
    if !entry.size.is_multiple_of(26) {
        return Err("SECTORS lump size is not divisible by 26".to_string());
    }

    let count = entry.size / 26;
    let mut sectors = Vec::with_capacity(count);

    for index in 0..count {
        let cursor = entry.offset + index * 26;
        sectors.push(Sector {
            floor_height: read_i16_le(wad_bytes, cursor)?,
            ceiling_height: read_i16_le(wad_bytes, cursor + 2)?,
            floor_texture: read_name8(wad_bytes, cursor + 4)?,
            ceiling_texture: read_name8(wad_bytes, cursor + 12)?,
        });
    }

    Ok(sectors)
}

fn parse_things(wad_bytes: &[u8], entry: &LumpEntry) -> Result<Vec<Thing>, String> {
    if !entry.size.is_multiple_of(10) {
        return Err("THINGS lump size is not divisible by 10".to_string());
    }

    let count = entry.size / 10;
    let mut things = Vec::with_capacity(count);

    for index in 0..count {
        let cursor = entry.offset + index * 10;
        things.push(Thing {
            x: read_i16_le(wad_bytes, cursor)?,
            y: read_i16_le(wad_bytes, cursor + 2)?,
            angle_deg: read_i16_le(wad_bytes, cursor + 4)?,
            kind: read_u16_le(wad_bytes, cursor + 6)?,
        });
    }

    Ok(things)
}

/// Collects all spawn-point `Thing`s for a given priority tier, so multiple
/// players/deathmatch starts on the same map are all discoverable instead of
/// only the first match. Falls through tiers only when a tier has zero
/// spawns at all (matches vanilla Doom's single-player/co-op/deathmatch
/// spawn-selection precedence).
///
/// Network-side per-player slot assignment (which spawn each joining player
/// actually gets) is a server-authoritative decision, not decided here — see
/// docs/CLASSIC_MAP_SUPPORT.md item 4. This function only surfaces what the
/// map data contains.
fn find_multiplayer_spawns(map: &ParsedMap) -> Vec<Thing> {
    let mut starts: Vec<Thing> = map.things.iter().copied().filter(|thing| (1..=4).contains(&thing.kind)).collect();
    starts.sort_by_key(|thing| thing.kind);
    if starts.is_empty() {
        starts.extend(map.things.iter().copied().filter(|thing| thing.kind == 11));
    }
    starts
}

fn find_player_spawns(map: &ParsedMap) -> Vec<Thing> {
    // Prefer the single-player start (type 1), then co-op starts (2-4),
    // then deathmatch starts (11) — vanilla Doom thing-type precedence.
    const PLAYER_START_PRIORITY: [u16; 5] = [1, 2, 3, 4, 11];

    for thing_kind in PLAYER_START_PRIORITY {
        let matches: Vec<Thing> = map
            .things
            .iter()
            .copied()
            .filter(|thing| thing.kind == thing_kind)
            .collect();
        if !matches.is_empty() {
            return matches;
        }
    }

    Vec::new()
}

/// Picks the spawn `Thing` for a given player slot (0-indexed), wrapping
/// around if there are fewer spawns than players — mirrors what a real
/// Doom co-op/deathmatch session does when player count exceeds declared
/// starts.
fn find_player_spawn_for_slot(map: &ParsedMap, slot: usize) -> Option<Thing> {
    let spawns = find_player_spawns(map);
    if spawns.is_empty() {
        return None;
    }
    Some(spawns[slot % spawns.len()])
}

fn doom_to_world(point: Vec2) -> Vec2 {
    Vec2::new(point.x * DOOM_TO_WORLD_SCALE, -point.y * DOOM_TO_WORLD_SCALE)
}

fn doom_angle_to_yaw(angle_deg: i16) -> f32 {
    let radians = (angle_deg as f32).to_radians();
    let world_direction = Vec2::new(radians.cos(), -radians.sin());
    world_direction.x.atan2(-world_direction.y)
}

// Geometry helper: args are independent spawn parameters, not state.
#[allow(clippy::too_many_arguments)]
fn spawn_wall_band(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    start: Vec2,
    end: Vec2,
    bottom: f32,
    top: f32,
    texture_name: String,
) {
    let height = top - bottom;
    if height <= WALL_MIN_HEIGHT {
        return;
    }

    let segment = end - start;
    let length = segment.length();
    if length <= 0.001 {
        return;
    }

    let midpoint = (start + end) * 0.5;
    let yaw = segment.x.atan2(segment.y);

    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(WALL_THICKNESS, height, length).mesh().build())),
        MeshMaterial3d(materials.add(texture_to_material(&texture_name))),
        Transform::from_xyz(midpoint.x, bottom + height * 0.5, midpoint.y)
            .with_rotation(Quat::from_rotation_y(yaw)),
        GlobalTransform::default(),
        Visibility::default(),
        crate::scene::LevelGeometry,
    ));
}

fn spawn_collision_debug_segment(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    start: Vec2,
    end: Vec2,
    is_door: bool,
) -> Entity {
    let segment = end - start;
    let length = segment.length();
    if length <= 0.001 {
        return Entity::PLACEHOLDER;
    }

    let midpoint = (start + end) * 0.5;
    let yaw = segment.x.atan2(segment.y);

    let mut entity_commands = commands.spawn((
        Mesh3d(
            meshes.add(
                Cuboid::new(COLLISION_DEBUG_THICKNESS, COLLISION_DEBUG_HEIGHT, length)
                    .mesh()
                    .build(),
            ),
        ),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.12, 1.0, 0.35),
            emissive: LinearRgba::rgb(0.10, 0.36, 0.18),
            unlit: true,
            ..default()
        })),
        Transform::from_xyz(midpoint.x, COLLISION_DEBUG_HEIGHT * 0.5 + 0.02, midpoint.y)
            .with_rotation(Quat::from_rotation_y(yaw)),
        GlobalTransform::default(),
        Visibility::Hidden,
        crate::scene::LevelGeometry,
        crate::scene::CollisionDebugVisual,
    ));

    if is_door {
        entity_commands.insert(crate::scene::DoorCollisionDebugVisual);
    }

    entity_commands.id()
}

fn spawn_door_panel(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    start: Vec2,
    end: Vec2,
    bottom: f32,
    height: f32,
) -> Entity {
    let segment = end - start;
    let length = segment.length();
    let midpoint = (start + end) * 0.5;
    let yaw = segment.x.atan2(segment.y);

    commands
        .spawn((
            Mesh3d(meshes.add(Cuboid::new(WALL_THICKNESS, height, length).mesh().build())),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.58, 0.40, 0.22),
                emissive: LinearRgba::rgb(0.05, 0.03, 0.01),
                metallic: 0.1,
                perceptual_roughness: 0.82,
                ..default()
            })),
            Transform::from_xyz(midpoint.x, bottom + height * 0.5, midpoint.y)
                .with_rotation(Quat::from_rotation_y(yaw)),
            GlobalTransform::default(),
            Visibility::default(),
            crate::scene::LevelGeometry,
        ))
        .id()
}

fn is_door_special(special: u16) -> bool {
    decode_linedef_special(special).is_some_and(|decoded| decoded.category == SpecialCategory::Door)
}

#[cfg(test)]
mod door_dispatch_tests {
    use super::*;
    #[test]
    fn non_door_specials_do_not_spawn_door_panels() {
        for special in [0, 5, 11, 12, 62] {
            assert!(!is_door_special(special), "Special {special} must not create a door");
        }
        for special in [1, 26, 27, 28, 31, 117] {
            assert!(is_door_special(special));
        }
    }
}

fn is_traversable_step(right_floor: f32, left_floor: f32) -> bool {
    (right_floor - left_floor).abs() <= MAX_TRAVERSABLE_STEP_HEIGHT
}

fn point_segment_distance(point: Vec2, start: Vec2, end: Vec2) -> f32 {
    let closest = closest_point_on_segment(point, start, end);
    (point - closest).length()
}

fn texture_to_material(texture_name: &str) -> StandardMaterial {
    let cleaned = clean_texture_name(texture_name);
    let mut hash = 1469598103934665603u64;
    for byte in cleaned.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }

    let hue = (hash & 0xFF) as f32 / 255.0;
    let sat = 0.35 + (((hash >> 8) & 0x7F) as f32 / 255.0) * 0.25;
    let val = 0.35 + (((hash >> 16) & 0x7F) as f32 / 255.0) * 0.30;
    let color = Color::hsv(hue * 360.0, sat, val).to_srgba();

    let roughness = 0.55 + (((hash >> 24) & 0x3F) as f32 / 255.0);
    let metallic = (((hash >> 30) & 0x1F) as f32 / 255.0) * 0.2;

    StandardMaterial {
        base_color: Color::srgba(color.red, color.green, color.blue, 1.0),
        perceptual_roughness: roughness.clamp(0.0, 1.0),
        metallic: metallic.clamp(0.0, 1.0),
        ..default()
    }
}

fn clean_texture_name(texture_name: &str) -> String {
    let trimmed = texture_name.trim();
    if trimmed.is_empty() || trimmed == "-" {
        "METAL".to_string()
    } else {
        trimmed.to_ascii_uppercase()
    }
}

fn choose_lower_texture(
    right_side: Option<&SideDef>,
    left_side: Option<&SideDef>,
    right_floor: f32,
    left_floor: f32,
) -> String {
    if right_floor < left_floor {
        right_side
            .map(|side| clean_texture_name(&side.lower_texture))
            .unwrap_or("STEP".to_string())
    } else {
        left_side
            .map(|side| clean_texture_name(&side.lower_texture))
            .unwrap_or("STEP".to_string())
    }
}

fn choose_upper_texture(
    right_side: Option<&SideDef>,
    left_side: Option<&SideDef>,
    right_ceil: f32,
    left_ceil: f32,
) -> String {
    if right_ceil > left_ceil {
        right_side
            .map(|side| clean_texture_name(&side.upper_texture))
            .unwrap_or("UPPER".to_string())
    } else {
        left_side
            .map(|side| clean_texture_name(&side.upper_texture))
            .unwrap_or("UPPER".to_string())
    }
}

fn closest_point_on_segment(point: Vec2, start: Vec2, end: Vec2) -> Vec2 {
    let segment = end - start;
    let length_sq = segment.length_squared();
    if length_sq <= 0.000001 {
        return start;
    }

    let t = (point - start).dot(segment) / length_sq;
    let t = t.clamp(0.0, 1.0);
    start + segment * t
}

fn infer_map_name_from_textmap_path(entry_name: &str) -> Option<String> {
    let normalized = entry_name.replace('\\', "/");
    let upper = normalized.to_ascii_uppercase();
    if !upper.ends_with("/TEXTMAP") && upper != "TEXTMAP" {
        return None;
    }

    let trimmed = normalized.trim_end_matches('/');
    let parent = trimmed.rsplit('/').nth(1)?;
    let map_name = parent.trim().to_ascii_uppercase();
    if map_name.is_empty() {
        None
    } else {
        Some(map_name)
    }
}

fn parse_udmf_textmap(text: &str) -> Result<ParsedMap, String> {
    let stripped = strip_udmf_comments(text);
    let mut map = ParsedMap {
        vertices: Vec::new(),
        linedefs: Vec::new(),
        sidedefs: Vec::new(),
        sectors: Vec::new(),
        things: Vec::new(),
    };

    let mut cursor = 0usize;
    while let Some((block_name, block_body, next_cursor)) = next_udmf_block(&stripped, cursor) {
        match block_name.as_str() {
            "vertex" => {
                let x = parse_udmf_f32(&block_body, "x").unwrap_or(0.0);
                let y = parse_udmf_f32(&block_body, "y").unwrap_or(0.0);
                map.vertices.push(Vertex {
                    x: x.round() as i16,
                    y: y.round() as i16,
                });
            }
            "linedef" => {
                let start_vertex = parse_udmf_u16(&block_body, "v1").unwrap_or(0);
                let end_vertex = parse_udmf_u16(&block_body, "v2").unwrap_or(0);
                let special = parse_udmf_u16(&block_body, "special").unwrap_or(0);
                let tag = parse_udmf_u16(&block_body, "id").unwrap_or(0);
                let right_sidedef = parse_udmf_u16(&block_body, "sidefront").unwrap_or(NO_SIDEDEF);
                let left_sidedef = parse_udmf_u16(&block_body, "sideback").unwrap_or(NO_SIDEDEF);
                map.linedefs.push(LineDef {
                    start_vertex,
                    end_vertex,
                    special,
                    tag,
                    right_sidedef,
                    left_sidedef,
                });
            }
            "sidedef" => {
                let sector = parse_udmf_u16(&block_body, "sector").unwrap_or(0);
                let upper_texture = parse_udmf_string(&block_body, "texturetop")
                    .unwrap_or_else(|| "-".to_string())
                    .to_ascii_uppercase();
                let lower_texture = parse_udmf_string(&block_body, "texturebottom")
                    .unwrap_or_else(|| "-".to_string())
                    .to_ascii_uppercase();
                let middle_texture = parse_udmf_string(&block_body, "texturemiddle")
                    .unwrap_or_else(|| "-".to_string())
                    .to_ascii_uppercase();
                map.sidedefs.push(SideDef {
                    upper_texture,
                    lower_texture,
                    middle_texture,
                    sector,
                });
            }
            "sector" => {
                let floor_height = parse_udmf_f32(&block_body, "heightfloor").unwrap_or(0.0).round() as i16;
                let ceiling_height = parse_udmf_f32(&block_body, "heightceiling").unwrap_or(128.0).round() as i16;
                let floor_texture = parse_udmf_string(&block_body, "texturefloor")
                    .unwrap_or_else(|| "FLOOR".to_string())
                    .to_ascii_uppercase();
                let ceiling_texture = parse_udmf_string(&block_body, "textureceiling")
                    .unwrap_or_else(|| "CEIL".to_string())
                    .to_ascii_uppercase();
                map.sectors.push(Sector {
                    floor_height,
                    ceiling_height,
                    floor_texture,
                    ceiling_texture,
                });
            }
            "thing" => {
                let x = parse_udmf_f32(&block_body, "x").unwrap_or(0.0);
                let y = parse_udmf_f32(&block_body, "y").unwrap_or(0.0);
                let angle = parse_udmf_f32(&block_body, "angle").unwrap_or(0.0);
                let kind = parse_udmf_u16(&block_body, "type").unwrap_or(0);
                map.things.push(Thing {
                    x: x.round() as i16,
                    y: y.round() as i16,
                    angle_deg: angle.round() as i16,
                    kind,
                });
            }
            _ => {}
        }
        cursor = next_cursor;
    }

    if map.vertices.is_empty() || map.linedefs.is_empty() {
        return Err("UDMF TEXTMAP missing required geometry blocks".to_string());
    }

    Ok(map)
}

fn strip_udmf_comments(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0usize;
    let mut in_string = false;

    while index < chars.len() {
        let c = chars[index];
        let next = chars.get(index + 1).copied();

        if in_string {
            if c == '\\' {
                // Handle escape sequence: copy backslash and next char
                out.push(c);
                if let Some(n) = next {
                    out.push(n);
                    index += 2;
                    continue;
                } else {
                    index += 1;
                    continue;
                }
            } else if c == '"' {
                in_string = false;
                out.push(c);
                index += 1;
                continue;
            } else {
                out.push(c);
                index += 1;
                continue;
            }
        } else {
            if c == '"' {
                in_string = true;
                out.push(c);
                index += 1;
                continue;
            }

            if c == '/' && next == Some('/') {
                index += 2;
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
                continue;
            }

            if c == '/' && next == Some('*') {
                index += 2;
                while index + 1 < chars.len() {
                    if chars[index] == '*' && chars[index + 1] == '/' {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
                continue;
            }

            out.push(c);
            index += 1;
        }
    }

    out
}

fn next_udmf_block(text: &str, start: usize) -> Option<(String, String, usize)> {
    let bytes = text.as_bytes();
    let mut i = start;

    while i < bytes.len() {
        // Skip whitespace
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }

        // Try to parse an identifier
        let name_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }

        if i == name_start {
            // Not an identifier, skip this character to avoid infinite loop
            i += 1;
            continue;
        }

        let name = text[name_start..i].trim().to_ascii_lowercase();

        // Skip whitespace after identifier
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }

        if i >= bytes.len() {
            return None;
        }

        if bytes[i] == b'{' {
            // It's a block
            i += 1;
            let body_start = i;
            let mut depth = 1usize;
            let mut in_string = false;

            while i < bytes.len() {
                let c = bytes[i];
                if in_string {
                    if c == b'\\' {
                        i += 2; // Skip escaped char
                        continue;
                    } else if c == b'"' {
                        in_string = false;
                    }
                } else {
                    if c == b'"' {
                        in_string = true;
                    } else if c == b'{' {
                        depth += 1;
                    } else if c == b'}' {
                        depth -= 1;
                        if depth == 0 {
                            let body = text[body_start..i].to_string();
                            return Some((name, body, i + 1));
                        }
                    }
                }
                i += 1;
            }
            // Unbalanced braces, return None
            return None;
        } else if bytes[i] == b'=' {
            // It's a global assignment. Skip until semicolon, respecting strings.
            i += 1; // skip '='
            let mut in_string = false;
            while i < bytes.len() {
                let c = bytes[i];
                if in_string {
                    if c == b'\\' {
                        i += 2;
                        continue;
                    } else if c == b'"' {
                        in_string = false;
                    }
                } else {
                    if c == b'"' {
                        in_string = true;
                    } else if c == b';' {
                        i += 1; // skip ';'
                        break;
                    }
                }
                i += 1;
            }
            // Continue loop to find next block
            continue;
        } else {
            // Unknown token, skip it
            i += 1;
            continue;
        }
    }

    None
}

fn parse_udmf_f32(body: &str, key: &str) -> Option<f32> {
    let value = extract_udmf_value(body, key)?;
    value.trim().parse::<f32>().ok()
}

fn parse_udmf_u16(body: &str, key: &str) -> Option<u16> {
    let value = extract_udmf_value(body, key)?;
    let value = value.trim();
    if let Ok(parsed) = value.parse::<u16>() {
        Some(parsed)
    } else if let Ok(parsed) = value.parse::<i32>() {
        if parsed < 0 {
            Some(NO_SIDEDEF)
        } else {
            u16::try_from(parsed).ok()
        }
    } else {
        None
    }
}

fn parse_udmf_string(body: &str, key: &str) -> Option<String> {
    let value = extract_udmf_value(body, key)?;
    let value = value.trim();
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        Some(value[1..value.len() - 1].to_string())
    } else {
        Some(value.to_string())
    }
}

fn extract_udmf_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    let mut statement_start = 0;

    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\\' {
                i += 2;
                continue;
            } else if c == b'"' {
                in_string = false;
            }
        } else {
            if c == b'"' {
                in_string = true;
            } else if c == b';' {
                // End of statement
                let statement = &body[statement_start..i];
                if let Some(val) = check_statement(statement, key) {
                    return Some(val);
                }
                statement_start = i + 1;
            }
        }
        i += 1;
    }

    // Check last statement
    let statement = &body[statement_start..];
    if let Some(val) = check_statement(statement, key) {
        return Some(val);
    }

    None
}

fn check_statement<'a>(statement: &'a str, key: &str) -> Option<&'a str> {
    let statement = statement.trim();
    if statement.is_empty() {
        return None;
    }

    // Find '=' outside of strings
    let bytes = statement.as_bytes();
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\\' {
                i += 2;
                continue;
            } else if c == b'"' {
                in_string = false;
            }
        } else {
            if c == b'"' {
                in_string = true;
            } else if c == b'=' {
                let lhs = &statement[..i];
                let rhs = &statement[i+1..];
                if lhs.trim().eq_ignore_ascii_case(key) {
                    return Some(rhs.trim());
                }
                return None; // Key not found in this statement
            }
        }
        i += 1;
    }
    None
}
#[cfg(test)]
mod udmf_regression_tests {
    use super::*;
    const GEOMETRY: &str = r#"
vertex { x = 16; y = -8; }
vertex { x = 128; y = 64; }
linedef { v1 = 0; v2 = 1; sidefront = 0; }
sidedef { sector = 0; texturemiddle = "STARTAN3"; }
sector { heightfloor = 8; heightceiling = 144; }
thing { x = 32; y = 48; type = 1; angle = 90; }
"#;
    fn check(text: &str) {
        let map = parse_udmf_textmap(text).expect("The valid fixture must parse.");
        assert_eq!(map.vertices.len(), 2);
        assert_eq!((map.vertices[0].x, map.vertices[0].y), (16, -8));
        assert_eq!((map.vertices[1].x, map.vertices[1].y), (128, 64));
        assert_eq!(map.linedefs.len(), 1);
        assert_eq!(map.linedefs[0].end_vertex, 1);
        assert_eq!(map.sidedefs.len(), 1);
        assert_eq!(map.sidedefs[0].middle_texture, "STARTAN3");
        assert_eq!(map.sectors.len(), 1);
        assert_eq!(map.sectors[0].ceiling_height, 144);
        assert_eq!(map.things.len(), 1);
        assert_eq!(map.things[0].kind, 1);
    }
    #[test]
    fn bare_geometry_control() { check(GEOMETRY); }
    #[test]
    fn namespace_header() { check(&format!("namespace = \"Doom\";\n{GEOMETRY}")); }
    #[test]
    fn interleaved_assignment() {
        check(&GEOMETRY.replace("linedef {", "user_scale = 1.5; linedef {"));
    }
    #[test]
    fn global_quoted_delimiters() {
        check(&format!(r#"user_note = "a; {{}} // /* literal */"; {GEOMETRY}"#));
    }
    #[test]
    fn escaped_quote_before_comment_marker() {
        check(&format!(r#"user_note = "quote \" // literal"; {GEOMETRY}"#));
    }
    #[test]
    fn escaped_backslash_at_string_end() {
        check(&format!(r#"user_path = "folder\\"; {GEOMETRY}"#));
    }
    #[test]
    fn block_quoted_brace() {
        check(&GEOMETRY.replace("vertex { x = 16;", "vertex { user_note = \"}\"; x = 16;"));
    }
    #[test]
    fn block_quoted_semicolon() {
        check(&GEOMETRY.replace("vertex { x = 16;", "vertex { user_note = \"alpha; beta\"; x = 16;"));
    }
    #[test]
    fn comments_and_case() {
        check(&format!("/* header */ NAMESPACE /* note */ = \"Doom\"; // header end\n{}", GEOMETRY.replace("vertex", "VeRtEx")));
    }
    #[test]
    fn unknown_block_preserves_geometry() {
        check(&format!(r#"user_extension {{ user_text = "{{;}}"; }} {GEOMETRY}"#));
    }
    #[test]
    fn unicode_text_does_not_panic() {
        check(&format!(r#"user_note = "café 地图"; {GEOMETRY}"#));
    }
    #[test]
    fn missing_geometry_still_rejected() {
        assert!(parse_udmf_textmap("namespace = \"Doom\";").is_err());
    }
}

fn read_i32_le(bytes: &[u8], offset: usize) -> Result<i32, String> {
    let range = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("offset {} out of bounds for i32", offset))?;
    Ok(i32::from_le_bytes([range[0], range[1], range[2], range[3]]))
}

fn read_name8(bytes: &[u8], offset: usize) -> Result<String, String> {
    let range = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| format!("offset {} out of bounds for name8", offset))?;
    let null_index = range
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(range.len());
    Ok(String::from_utf8_lossy(&range[..null_index]).trim().to_ascii_uppercase())
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let range = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| format!("offset {} out of bounds for u16", offset))?;
    Ok(u16::from_le_bytes([range[0], range[1]]))
}

fn read_u8(bytes: &[u8], offset: usize) -> Result<u8, String> {
    bytes
        .get(offset)
        .copied()
        .ok_or_else(|| format!("offset {} out of bounds for u8", offset))
}

fn read_i16_le(bytes: &[u8], offset: usize) -> Result<i16, String> {
    let range = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| format!("offset {} out of bounds for i16", offset))?;
    Ok(i16::from_le_bytes([range[0], range[1]]))
}

#[cfg(test)]
mod hexen_format_tests {
    use super::*;

    /// Build a minimal in-memory PWAD from (name, data) lumps.
    fn build_wad(lumps: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(b"PWAD");
        data.extend_from_slice(&(lumps.len() as u32).to_le_bytes());
        let mut directory_offset = 12usize;
        for (_, bytes) in lumps {
            directory_offset += bytes.len();
        }
        data.extend_from_slice(&(directory_offset as u32).to_le_bytes());
        let mut offset = 12usize;
        let mut directory = Vec::new();
        for (name, bytes) in lumps {
            data.extend_from_slice(bytes);
            directory.extend_from_slice(&(offset as u32).to_le_bytes());
            directory.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            let mut name_bytes = [0u8; 8];
            name_bytes[..name.len()].copy_from_slice(name.as_bytes());
            directory.extend_from_slice(&name_bytes);
            offset += bytes.len();
        }
        data.extend_from_slice(&directory);
        data
    }

    /// One Hexen-format map whose LINEDEFS lump holds 7 records (112 bytes).
    /// 112 is also divisible by 14, so size-only detection misparses it as
    /// eight Doom records with shifted fields.
    fn hexen_ambiguity_wad(with_behavior: bool) -> Vec<u8> {
        let mut linedefs = Vec::new();
        for record in 0..7u16 {
            linedefs.extend_from_slice(&record.to_le_bytes());
            linedefs.extend_from_slice(&((record + 1) % 2).to_le_bytes());
            linedefs.extend_from_slice(&0u16.to_le_bytes()); // flags
            linedefs.push(121); // special lives at byte 6 in Hexen records
            linedefs.extend_from_slice(&[1, 2, 3, 4, 5]); // action args
            linedefs.extend_from_slice(&0u16.to_le_bytes()); // right sidedef
            linedefs.extend_from_slice(&0xFFFFu16.to_le_bytes()); // left = none
        }
        assert_eq!(linedefs.len(), 112);
        let mut lumps: Vec<(&str, Vec<u8>)> = vec![
            ("MAP01", Vec::new()),
            ("THINGS", Vec::new()),
            ("LINEDEFS", linedefs),
            ("SIDEDEFS", vec![0; 30]),
            ("VERTEXES", vec![0; 8]),
            ("SECTORS", vec![0; 26]),
        ];
        if with_behavior {
            lumps.push(("BEHAVIOR", vec![0; 4]));
        }
        build_wad(&lumps)
    }

    #[test]
    fn behavior_lump_selects_hexen_format_over_size_divisibility() {
        let bytes = hexen_ambiguity_wad(true);
        let lumps = parse_lump_directory(&bytes).unwrap();
        let map = parse_map_lumps(&bytes, &lumps, "MAP01").unwrap();
        assert_eq!(map.linedefs.len(), 7, "BEHAVIOR must force 16-byte Hexen records");
        assert_eq!(map.linedefs[0].special, 121);
        assert_eq!(map.linedefs[0].left_sidedef, NO_SIDEDEF);
    }

    #[test]
    fn without_behavior_the_divisibility_fallback_still_parses_doom() {
        let bytes = hexen_ambiguity_wad(false);
        let lumps = parse_lump_directory(&bytes).unwrap();
        let map = parse_map_lumps(&bytes, &lumps, "MAP01").unwrap();
        assert_eq!(map.linedefs.len(), 8, "no BEHAVIOR keeps the legacy Doom-first fallback");
    }

    #[test]
    fn behavior_in_a_later_map_does_not_flip_earlier_doom_map() {
        // MAP01 is Doom-format; MAP02 is a Hexen map carrying the BEHAVIOR
        // lump. The marker-bounded section scan must keep them independent.
        let lumps: Vec<(&str, Vec<u8>)> = vec![
            ("MAP01", Vec::new()),
            ("THINGS", Vec::new()),
            ("LINEDEFS", vec![0; 28]), // 2 Doom records, not divisible by 16
            ("SIDEDEFS", vec![0; 30]),
            ("VERTEXES", vec![0; 8]),
            ("SECTORS", vec![0; 26]),
            ("MAP02", Vec::new()),
            ("THINGS", Vec::new()),
            ("LINEDEFS", vec![0; 112]), // ambiguous size
            ("SIDEDEFS", vec![0; 30]),
            ("VERTEXES", vec![0; 8]),
            ("SECTORS", vec![0; 26]),
            ("BEHAVIOR", vec![0; 4]),
        ];
        let bytes = build_wad(&lumps);
        let directory = parse_lump_directory(&bytes).unwrap();
        let map = parse_map_lumps(&bytes, &directory, "MAP01").unwrap();
        assert_eq!(map.linedefs.len(), 2, "MAP01 must stay Doom despite MAP02's BEHAVIOR");
        let map2 = parse_map_lumps(&bytes, &directory, "MAP02").unwrap();
        assert_eq!(map2.linedefs.len(), 7, "MAP02's own BEHAVIOR selects Hexen");
    }
}

#[cfg(test)]
mod archive_cap_tests {
    use super::*;

    #[test]
    fn flat_resources_use_last_palette_and_ignore_outside_names() {
        let mut bytes = vec![9; 768];
        bytes.extend_from_slice(&vec![77; 768]);
        bytes.extend_from_slice(&vec![0; 4096]);
        let mut lumps = vec![
            LumpEntry { offset: 0, size: 768, name: "PLAYPAL".into() },
            LumpEntry { offset: 768, size: 768, name: "PLAYPAL".into() },
            LumpEntry { offset: 0, size: 0, name: "F_START".into() },
            LumpEntry { offset: 1536, size: 4096, name: "A".into() },
            LumpEntry { offset: 0, size: 0, name: "F_END".into() },
            LumpEntry { offset: 0, size: 768, name: "A".into() },
        ];
        let requested = std::collections::BTreeSet::from(["A".to_owned(), "MISSING".to_owned()]);
        let decoded = prepare_flat_images(&bytes, &lumps, &requested).unwrap();
        assert_eq!(decoded.len(), 1);
        assert!(decoded["A"].data.as_ref().unwrap().chunks_exact(4).all(|p| p == [77, 77, 77, 255]));
        lumps[3].size = 4095;
        let degraded = prepare_flat_images(&bytes, &lumps, &requested).unwrap();
        assert!(degraded.is_empty(), "a damaged flat must degrade to a placeholder");
        assert!(prepare_flat_images(&[], &[], &requested).unwrap().is_empty());
    }

    #[test]
    fn flat_resources_enforce_decoded_budget_and_byte_bounds() {
        let too_many = (0..2049).map(|n| format!("F{n:07}")).collect();
        assert!(prepare_flat_images(&[], &[], &too_many).unwrap_err().contains("budget"));
        let bad = [LumpEntry { offset: usize::MAX, size: 1, name: "PLAYPAL".into() }];
        assert!(
            prepare_flat_images(&[], &bad, &Default::default()).unwrap().is_empty(),
            "an out-of-range PLAYPAL must degrade to placeholders"
        );
    }

    #[test]
    fn damaged_flat_directories_degrade_to_placeholders() {
        // Non-ASCII lump names, an unclosed F_START, and one short flat lump
        // must not reject the map; the healthy flat still decodes.
        let mut bytes = vec![9; 768];
        bytes.extend_from_slice(&vec![0; 4096]);
        let lumps = vec![
            LumpEntry { offset: 0, size: 768, name: "PLAYPAL".into() },
            LumpEntry { offset: 0, size: 0, name: "F_START".into() },
            LumpEntry { offset: 768, size: 4096, name: "GOOD".into() },
            LumpEntry { offset: 768, size: 100, name: "SHORTY".into() },
            LumpEntry { offset: 0, size: 0, name: "\u{FFFD}BAD".into() },
        ];
        let requested: std::collections::BTreeSet<String> =
            ["GOOD".to_owned(), "SHORTY".to_owned()].into();
        let decoded = prepare_flat_images(&bytes, &lumps, &requested).unwrap();
        assert_eq!(decoded.len(), 1);
        assert!(decoded["GOOD"].data.as_ref().unwrap().chunks_exact(4).all(|p| p == [9, 9, 9, 255]));
    }

    #[test]
    fn unmeshable_sector_is_skipped_without_rejecting_the_map() {
        // One sector with an open boundary must lose only its own surfaces;
        // the healthy sector still meshes floor and ceiling.
        let square = [
            Vec2::new(-4.0, -4.0),
            Vec2::new(4.0, -4.0),
            Vec2::new(4.0, 4.0),
            Vec2::new(-4.0, 4.0),
        ];
        let closed: Vec<_> = (0..4)
            .map(|i| WallCollisionSegment { start: square[i], end: square[(i + 1) % 4] })
            .collect();
        let open = vec![WallCollisionSegment { start: Vec2::new(10.0, 10.0), end: Vec2::new(12.0, 10.0) }];
        let map = ParsedMap {
            vertices: Vec::new(),
            linedefs: Vec::new(),
            sidedefs: Vec::new(),
            sectors: vec![
                Sector { floor_height: 0, ceiling_height: 8, floor_texture: "FLATA".into(), ceiling_texture: "FLATB".into() },
                Sector { floor_height: 0, ceiling_height: 8, floor_texture: "FLATC".into(), ceiling_texture: "FLATD".into() },
            ],
            things: Vec::new(),
        };
        let regions = vec![
            WadSectorRegion { edges: closed, floor: 0.0, ceiling: 8.0 },
            WadSectorRegion { edges: open, floor: 0.0, ceiling: 8.0 },
        ];
        let prepared = prepare_sector_surfaces(&map, &regions, Vec2::ZERO).unwrap();
        let names: Vec<_> = prepared.iter().map(|(_, name)| name.as_str()).collect();
        assert_eq!(names, vec!["FLATA", "FLATB"], "only the healthy sector surfaces survive");
    }

    #[test]
    #[ignore = "Requires the Freedoom reference WADs in CHICKEN_REFERENCE_DIR"]
    fn real_freedoom_flat_pixels_match_source_palette() {
        let root = std::path::PathBuf::from(std::env::var("CHICKEN_REFERENCE_DIR").unwrap());
        let mut checked = 0;
        for (file, count) in [("freedoom1.wad", 36), ("freedoom2.wad", 32)] {
            let bytes = std::fs::read(root.join(file)).unwrap();
            let lumps = parse_lump_directory(&bytes).unwrap();
            let palette_entry = lumps.iter().rev().find(|l| l.name == "PLAYPAL").unwrap();
            let palette = &bytes[palette_entry.offset..palette_entry.offset + 768];
            let maps = find_map_markers(&lumps);
            assert_eq!(maps.len(), count);
            for name in maps {
                let map = parse_map_lumps(&bytes, &lumps, &name).unwrap();
                let requested: std::collections::BTreeSet<_> = map.sectors.iter()
                    .flat_map(|s| [&s.floor_texture, &s.ceiling_texture])
                    .map(|s| clean_texture_name(s)).filter(|s| s != "F_SKY1").collect();
                let images = prepare_flat_images(&bytes, &lumps, &requested).unwrap();
                assert_eq!(images.len(), requested.len(), "Missing flat in {file}:{name}");
                for (flat, image) in &images {
                    // Compare every pixel with source bytes, without the decoder or namespace helper.
                    let entry = lumps.iter().rev().find(|l| l.name == *flat).unwrap();
                    assert_eq!(entry.size, 4096);
                    let source = &bytes[entry.offset..entry.offset + entry.size];
                    let pixels = image.data.as_ref().unwrap();
                    assert_eq!(pixels.len(), 16384);
                    for (index, pixel) in source.iter().zip(pixels.chunks_exact(4)) {
                        let offset = usize::from(*index) * 3;
                        assert_eq!(&pixel[..3], &palette[offset..offset + 3]);
                        assert_eq!(pixel[3], 255);
                    }
                }
                println!("FLATS\t{file}\t{name}\ttextures={}\tpixels={}", images.len(), images.len() * 4096);
                checked += 1;
            }
        }
        assert_eq!(checked, 68);
    }


    #[test]
    fn doom_flat_uvs_follow_original_plane_axes() {
        use bevy::mesh::VertexAttributeValues;
        let native = [Vec2::new(16.0, 8.0), Vec2::new(48.0, 8.0), Vec2::new(48.0, 24.0), Vec2::new(16.0, 24.0)];
        let center = doom_to_world(Vec2::new(32.0, 16.0));
        let points = native.map(|p| doom_to_world(p) - center);
        let edges = [(points[0], points[1]), (points[1], points[2]), (points[2], points[3]), (points[3], points[0])];
        for ceiling in [false, true] {
            let mesh = crate::map_surfaces::horizontal_mesh(&edges, 0.0, ceiling, center, 64.0 * DOOM_TO_WORLD_SCALE).unwrap();
            let VertexAttributeValues::Float32x3(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("Invalid positions") };
            let VertexAttributeValues::Float32x2(uvs) = mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap() else { panic!("Invalid UVs") };
            for (p, uv) in positions.iter().zip(uvs) {
                let doom_x = (p[0] + center.x) / DOOM_TO_WORLD_SCALE;
                let doom_y = -(p[2] + center.y) / DOOM_TO_WORLD_SCALE;
                // R_MapPlane samples at world X and negative world Y.
                assert!((uv[0] - doom_x / 64.0).abs() < 0.00001);
                assert!((uv[1] + doom_y / 64.0).abs() < 0.00001, "Flat rows run along the wrong axis");
            }
        }
    }


    #[test]
    #[ignore = "Requires the Freedoom reference WADs in CHICKEN_REFERENCE_DIR"]
    fn real_freedoom_map01_camera_sweep() {
        use crate::map_camera::{camera_anchor, clip_boom};
        let root = std::env::var("CHICKEN_REFERENCE_DIR").expect("Set CHICKEN_REFERENCE_DIR");
        let path = std::path::Path::new(&root).join("freedoom2.wad");
        let mut world = World::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        spawn_headless_level(&mut commands, path.to_str().unwrap(), "MAP01").unwrap();
        queue.apply(&mut world);
        let collision = world.resource::<WadCollisionWorld>();
        assert_eq!(collision.player_spawns.len(), 4);
        let radius = 0.15;
        let mut checked = 0;
        let mut shortened = 0;
        for (slot, spawn) in collision.player_spawns.iter().enumerate() {
            let pivot = camera_anchor(collision, spawn.translation + Vec3::Y * crate::player::CUBE_REST_Y, radius);
            for yaw_step in 0..8 {
                for pitch in [-0.8, -0.2, 0.5] {
                    let rotation = Quat::from_euler(EulerRot::YXZ, yaw_step as f32 * std::f32::consts::FRAC_PI_4, pitch, 0.0);
                    let desired = pivot + rotation * Vec3::new(0.0, 1.6, 5.5);
                    let camera = clip_boom(collision, pivot, desired, radius);
                    assert!(camera.is_finite());
                    assert!(camera.distance(pivot) <= desired.distance(pivot) + 0.001);
                    assert!((camera - pivot).cross(desired - pivot).length() < 0.001);
                    // Check the whole accepted ray independently, not only its endpoint.
                    for step in 0..=32 {
                        let point = pivot.lerp(camera, step as f32 / 32.0);
                        let (floor, ceiling) = collision.vertical_bounds(point).expect("Camera left the map");
                        assert!(point.y >= floor + radius - 0.001 && point.y <= ceiling - radius + 0.001, "slot={slot} yaw={yaw_step} pitch={pitch} point={point:?} bounds={floor}..{ceiling}");
                        for segment in &collision.segments {
                            let p = Vec2::new(point.x, point.z);
                            let ab = segment.end - segment.start;
                            let t = if ab.length_squared() == 0.0 { 0.0 } else { ((p - segment.start).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) };
                            assert!(p.distance(segment.start + ab * t) >= radius - 0.001);
                        }
                    }
                    shortened += usize::from(camera.distance(desired) > 0.001);
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 96);
        assert!(shortened > 0);
        println!("CAMERA\tfreedoom2.wad\tMAP01\trays={checked}\tshortened={shortened}");
    }

    #[test]
    #[ignore = "Requires the separately downloaded Freedoom reference files"]
    fn real_headless_freedoom_load_retains_collision_and_door_entities() {
        let root = std::env::var("CHICKEN_REFERENCE_DIR").expect("Set CHICKEN_REFERENCE_DIR");
        let path = std::path::Path::new(&root).join("freedoom2.wad");
        let mut world = World::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        let level = spawn_headless_level(&mut commands, path.to_str().unwrap(), "MAP01").unwrap();
        queue.apply(&mut world);
        assert_eq!(level.map_name, "MAP01");
        assert!(level.info.suggested_plane_limit > MIN_FLOOR_HALF_EXTENT);
        let collision = world.resource::<WadCollisionWorld>();
        assert!(!collision.segments.is_empty());
        assert!(!collision.doors.is_empty());
        assert_eq!(collision.player_spawns.len(), 4);
        for start in &collision.player_spawns {
            assert!(collision.vertical_bounds(start.translation).is_some());
            assert!((start.translation.y - collision.floor_height_at(start.translation) - crate::player::CUBE_REST_Y).abs() < 0.001);
        }
        assert_ne!(collision.player_spawns[0].translation, collision.player_spawns[1].translation);
        for door in &collision.doors { assert!(world.get_entity(door.entity).is_ok()); }
    }

    #[test]
    #[ignore = "Requires the separately downloaded Freedoom reference files"]
    fn real_freedoom_maps_preserve_valid_geometry_references() {
        let root = std::env::var("CHICKEN_REFERENCE_DIR").expect("Set CHICKEN_REFERENCE_DIR");
        for (file, expected) in [("freedoom1.wad", 36), ("freedoom2.wad", 32)] {
            let bytes = std::fs::read(std::path::Path::new(&root).join(file)).unwrap();
            let lumps = parse_lump_directory(&bytes).unwrap();
            let names = find_map_markers(&lumps);
            assert_eq!(names.len(), expected);
            for name in names {
                let map = parse_map_lumps(&bytes, &lumps, &name).unwrap();
                assert!(!map.vertices.is_empty(), "{file}:{name}");
                assert!(!map.sectors.is_empty(), "{file}:{name}");
                for line in &map.linedefs {
                    assert!((line.start_vertex as usize) < map.vertices.len());
                    assert!((line.end_vertex as usize) < map.vertices.len());
                    for side in [line.right_sidedef, line.left_sidedef] {
                        assert!(side == NO_SIDEDEF || (side as usize) < map.sidedefs.len());
                    }
                }
                for side in &map.sidedefs { assert!((side.sector as usize) < map.sectors.len()); }
                println!("{file}:{name}: vertices={} lines={} sectors={}", map.vertices.len(), map.linedefs.len(), map.sectors.len());
            }
        }
    }


    #[test]
    #[ignore = "Requires the Freedoom reference WADs in CHICKEN_REFERENCE_DIR"]
    fn real_freedoom_surface_entities_use_prepared_meshes() {
        use bevy::mesh::VertexAttributeValues;
        fn signature(mesh: &Mesh) -> Vec<u32> {
            let mut bits = Vec::new();
            for attribute in [Mesh::ATTRIBUTE_POSITION, Mesh::ATTRIBUTE_NORMAL, Mesh::ATTRIBUTE_UV_0] {
                match mesh.attribute(attribute).unwrap() {
                    VertexAttributeValues::Float32x3(values) => bits.extend(values.iter().flatten().map(|v| v.to_bits())),
                    VertexAttributeValues::Float32x2(values) => bits.extend(values.iter().flatten().map(|v| v.to_bits())),
                    _ => panic!("Invalid mesh attribute type"),
                }
            }
            bits
        }
        let root = std::env::var("CHICKEN_REFERENCE_DIR").expect("Set CHICKEN_REFERENCE_DIR");
        let path = std::path::Path::new(&root).join("freedoom2.wad");
        let data = std::fs::read(&path).unwrap();
        let lumps = parse_lump_directory(&data).unwrap();
        let map = parse_map_lumps(&data, &lumps, "MAP01").unwrap();
        let points: Vec<_> = map.vertices.iter().map(|v| doom_to_world(Vec2::new(v.x as f32, v.y as f32))).collect();
        let min = points.iter().copied().fold(Vec2::splat(f32::INFINITY), Vec2::min);
        let max = points.iter().copied().fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max);
        let center = (min + max) * 0.5;
        let regions = sector_regions(&map, center);
        let prepared = prepare_sector_surfaces(&map, &regions, center).unwrap();
        let mut expected: Vec<_> = prepared.iter().map(|(mesh, _)| signature(mesh)).collect();
        let mut world = World::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        let mut meshes = Assets::<Mesh>::default();
        let mut materials = Assets::<StandardMaterial>::default();
        let mut images = Assets::<Image>::default();
        try_spawn_level_from_selection(&mut commands, &mut meshes, &mut materials, Some(&mut images), path.to_str().unwrap(), "MAP01").unwrap();
        queue.apply(&mut world);
        let by_mesh: std::collections::BTreeMap<_, _> = prepared.iter().map(|(mesh, name)| (signature(mesh), name)).collect();
        let mut material_ids = std::collections::BTreeMap::new();
        let palette_entry = lumps.iter().rev().find(|l| l.name == "PLAYPAL").unwrap();
        let palette = &data[palette_entry.offset..palette_entry.offset + 768];
        for (mesh, material) in world.query_filtered::<(&Mesh3d, &MeshMaterial3d<StandardMaterial>), With<crate::scene::LevelGeometry>>().iter(&world) {
            let mesh = meshes.get(&mesh.0).unwrap();
            if mesh.indices().is_some() { continue; }
            let name = by_mesh[&signature(mesh)];
            if let Some(previous) = material_ids.insert(name, material.0.id()) { assert_eq!(previous, material.0.id()); }
            let material = materials.get(&material.0).unwrap();
            assert_eq!(material.base_color, Color::WHITE);
            assert!(material.unlit);
            let image = images.get(material.base_color_texture.as_ref().expect("Surface has no flat image")).unwrap();
            let entry = lumps.iter().rev().find(|l| l.name == *name).unwrap();
            let source = &data[entry.offset..entry.offset + entry.size];
            assert_eq!(source.len(), 4096);
            assert_eq!(image.data.as_ref().unwrap().len(), 16384);
            for (index, pixel) in source.iter().zip(image.data.as_ref().unwrap().chunks_exact(4)) {
                let offset = usize::from(*index) * 3;
                assert_eq!(&pixel[..3], &palette[offset..offset + 3]);
                assert_eq!(pixel[3], 255);
            }
        }
        assert_eq!(images.len(), material_ids.len());
        let mut actual: Vec<_> = world.query_filtered::<&Mesh3d, With<crate::scene::LevelGeometry>>()
            .iter(&world).map(|handle| meshes.get(&handle.0).unwrap())
            .filter(|mesh| mesh.indices().is_none()).map(signature).collect();
        assert!(!actual.is_empty());
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected);
    }

    #[test]
    #[ignore = "Requires the Freedoom reference WADs in CHICKEN_REFERENCE_DIR"]
    fn real_freedoom_sector_surfaces() {
        use bevy::mesh::VertexAttributeValues;
        let root = std::path::PathBuf::from(std::env::var("CHICKEN_REFERENCE_DIR").expect("Set CHICKEN_REFERENCE_DIR"));
        let mut maps_checked = 0usize;
        for (file, expected) in [("freedoom1.wad", 36), ("freedoom2.wad", 32)] {
            let data = std::fs::read(root.join(file)).unwrap();
            let lumps = parse_lump_directory(&data).unwrap();
            let names: Vec<_> = lumps.iter().filter(|lump| is_map_marker_name(&lump.name)).map(|lump| lump.name.clone()).collect();
            assert_eq!(names.len(), expected);
            for name in names {
                let map = parse_map_lumps(&data, &lumps, &name).unwrap();
                let min = map.vertices.iter().map(|v| doom_to_world(Vec2::new(v.x as f32, v.y as f32))).fold(Vec2::splat(f32::INFINITY), Vec2::min);
                let max = map.vertices.iter().map(|v| doom_to_world(Vec2::new(v.x as f32, v.y as f32))).fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max);
                let center = (min + max) * 0.5;
                let regions = sector_regions(&map, center);
                let surfaces = prepare_sector_surfaces(&map, &regions, center).unwrap_or_else(|e| panic!("{file}:{name}: {e}"));
                assert!(!surfaces.is_empty(), "{file}:{name}");
                let mut surface_index = 0usize;
                let mut vertices = 0usize;
                for (sector_index, (sector, region)) in map.sectors.iter().zip(&regions).enumerate() {
                    if region.edges.is_empty() { continue; }
                    for (height, ceiling, texture) in [(region.floor, false, &sector.floor_texture), (region.ceiling, true, &sector.ceiling_texture)] {
                        if texture.eq_ignore_ascii_case("F_SKY1") { continue; }
                        let (mesh, name) = &surfaces[surface_index];
                        surface_index += 1;
                        assert_eq!(*name, clean_texture_name(texture));
                        let VertexAttributeValues::Float32x3(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("Invalid position type") };
                        let VertexAttributeValues::Float32x3(normals) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() else { panic!("Invalid normal type") };
                        let VertexAttributeValues::Float32x2(uvs) = mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap() else { panic!("Invalid UV type") };
                        assert_eq!(positions.len() % 3, 0);
                        assert!(!positions.is_empty());
                        assert_eq!(normals.len(), positions.len());
                        assert_eq!(uvs.len(), positions.len());
                        vertices += positions.len();
                        let expected_normal = if ceiling { -Vec3::Y } else { Vec3::Y };
                        for face in positions.chunks_exact(3) {
                            let [a, b, c] = [Vec3::from(face[0]), Vec3::from(face[1]), Vec3::from(face[2])];
                            assert!((b - a).cross(c - a).dot(expected_normal) > 0.0);
                            let mid = (a + b + c) / 3.0;
                            assert!(region.contains(Vec2::new(mid.x, mid.z)), "{file}:{sector_index}: Triangle lies outside its sector at {mid:?}");
                        }
                        for ((position, normal), uv) in positions.iter().zip(normals).zip(uvs) {
                            assert_eq!(position[1], height);
                            assert_eq!(*normal, expected_normal.to_array());
                            assert!(position.iter().chain(uv).all(|v| v.is_finite()));
                            assert_eq!(*uv, [(position[0] + center.x) / (64.0 * DOOM_TO_WORLD_SCALE), (position[2] + center.y) / (64.0 * DOOM_TO_WORLD_SCALE)]);
                        }
                    }
                }
                assert_eq!(surface_index, surfaces.len());
                println!("SURFACES\t{file}\t{name}\t{}\t{}\t{vertices}", regions.len(), surfaces.len());
                maps_checked += 1;
            }
        }
        assert_eq!(maps_checked, 68);
    }

    #[test]
    fn read_entry_bytes_capped_accepts_data_within_limit() {
        let data = vec![7u8; 1024];
        let mut cursor = Cursor::new(data.clone());
        let read = read_entry_bytes_capped(&mut cursor, "small.wad").unwrap();
        assert_eq!(read, data);
    }

    #[test]
    fn read_entry_bytes_capped_rejects_oversized_entry() {
        // One byte over the cap must be rejected rather than fully buffered.
        let data = vec![0u8; (MAX_ARCHIVE_ENTRY_BYTES + 1) as usize];
        let mut cursor = Cursor::new(data);
        let result = read_entry_bytes_capped(&mut cursor, "huge.wad");
        assert!(result.is_err());
    }

    #[test]
    fn read_entry_string_capped_rejects_oversized_text() {
        let data = vec![b'a'; (MAX_ARCHIVE_ENTRY_BYTES + 1) as usize];
        let mut cursor = Cursor::new(data);
        let result = read_entry_string_capped(&mut cursor, "huge_textmap.txt");
        assert!(result.is_err());
    }

    #[test]
    fn decode_linedef_special_matches_doomwiki_door_table() {
        // Cross-checked against https://doomwiki.org/wiki/Linedef_type
        // "Door linedef types" table — do not adjust without re-verifying.
        let unlocked = decode_linedef_special(1).unwrap();
        assert_eq!(unlocked.category, SpecialCategory::Door);
        assert_eq!(unlocked.activation, ActivationKind::Manual);
        assert!(unlocked.repeatable);
        assert_eq!(unlocked.required_key, None);

        // DR Door Blue/Yellow/Red Key — push-triggered, repeatable, locked.
        assert_eq!(decode_linedef_special(26).unwrap().required_key, Some(KeyColor::Blue));
        assert_eq!(decode_linedef_special(27).unwrap().required_key, Some(KeyColor::Yellow));
        assert_eq!(decode_linedef_special(28).unwrap().required_key, Some(KeyColor::Red));
        assert!(decode_linedef_special(26).unwrap().repeatable);

        // D1 Door {Stay Open, Blue/Red/Yellow Key} — push-triggered, one-shot.
        let stay_open = decode_linedef_special(31).unwrap();
        assert_eq!(stay_open.activation, ActivationKind::Manual);
        assert!(!stay_open.repeatable);
        assert_eq!(stay_open.required_key, None);
        assert_eq!(decode_linedef_special(32).unwrap().required_key, Some(KeyColor::Blue));
        assert_eq!(decode_linedef_special(33).unwrap().required_key, Some(KeyColor::Red));
        assert_eq!(decode_linedef_special(34).unwrap().required_key, Some(KeyColor::Yellow));

        // 117/118 = Doom II fast-door variants of 1/31 — same trigger shape.
        assert_eq!(decode_linedef_special(117).unwrap().activation, ActivationKind::Manual);
        assert!(decode_linedef_special(117).unwrap().repeatable);
        assert!(!decode_linedef_special(118).unwrap().repeatable);

        // Anything not in the table decodes to None (falls through to the
        // geometry-based fallback), not a wrong guess.
        assert!(decode_linedef_special(9999).is_none());
    }

    #[test]
    fn decode_linedef_special_floor_specials() {
        // Lower floor to ground level (fastest)
        let lower_fast = decode_linedef_special(19).unwrap();
        assert_eq!(lower_fast.category, SpecialCategory::Floor);
        assert_eq!(lower_fast.activation, ActivationKind::Walk);
        assert!(lower_fast.repeatable);
        assert_eq!(lower_fast.required_key, None);

        // Multiple walk-triggered floor types all repeat when crossed
        assert!(decode_linedef_special(18).unwrap().repeatable);
        assert!(decode_linedef_special(64).unwrap().repeatable);

        // One-time floor (36 lower then stay)
        assert!(!decode_linedef_special(36).unwrap().repeatable);
    }

    #[test]
    fn decode_linedef_special_lift_specials() {
        // Raising/lowering platforms
        let lift_up = decode_linedef_special(62).unwrap();
        assert_eq!(lift_up.category, SpecialCategory::Lift);
        assert_eq!(lift_up.activation, ActivationKind::Walk);
        assert!(lift_up.repeatable);

        assert_eq!(decode_linedef_special(63).unwrap().category, SpecialCategory::Lift);
        assert_eq!(decode_linedef_special(66).unwrap().category, SpecialCategory::Lift);
    }

    #[test]
    fn decode_linedef_special_ceiling_specials() {
        // Lowering/raising ceilings
        let lower_ceil = decode_linedef_special(40).unwrap();
        assert_eq!(lower_ceil.category, SpecialCategory::Ceiling);
        assert_eq!(lower_ceil.activation, ActivationKind::Walk);
        assert!(lower_ceil.repeatable);

        assert_eq!(decode_linedef_special(41).unwrap().category, SpecialCategory::Ceiling);
        assert_eq!(decode_linedef_special(42).unwrap().category, SpecialCategory::Ceiling);
    }

    #[test]
    fn decode_linedef_special_stair_specials() {
        // Staircase builders
        let stairs_8 = decode_linedef_special(8).unwrap();
        assert_eq!(stairs_8.category, SpecialCategory::Stair);
        assert_eq!(stairs_8.activation, ActivationKind::Walk);
        // Stairs are one-shot per trigger
        assert!(!stairs_8.repeatable);

        assert_eq!(decode_linedef_special(7).unwrap().category, SpecialCategory::Stair);
    }

    #[test]
    fn decode_linedef_special_teleporter() {
        // Teleporting to TAG-linked sector
        let teleport = decode_linedef_special(39).unwrap();
        assert_eq!(teleport.category, SpecialCategory::Teleport);
        assert_eq!(teleport.activation, ActivationKind::Walk);
        assert!(teleport.repeatable);
    }

    #[test]
    fn decode_linedef_special_exit_specials() {
        // Level exit
        let exit_normal = decode_linedef_special(11).unwrap();
        assert_eq!(exit_normal.category, SpecialCategory::Exit);
        assert_eq!(exit_normal.activation, ActivationKind::Walk);
        assert!(!exit_normal.repeatable);

        // Secret exit
        assert_eq!(decode_linedef_special(51).unwrap().category, SpecialCategory::Exit);
    }

    #[test]
    fn decode_linedef_special_light_effects() {
        // Light flicker, strobe, etc.
        let light_flash = decode_linedef_special(2).unwrap();
        assert_eq!(light_flash.category, SpecialCategory::LightEffect);
        assert_eq!(light_flash.activation, ActivationKind::Walk);
        assert!(light_flash.repeatable);

        assert_eq!(decode_linedef_special(3).unwrap().category, SpecialCategory::LightEffect);
        assert_eq!(decode_linedef_special(4).unwrap().category, SpecialCategory::LightEffect);
    }

    fn thing(kind: u16, x: i16) -> Thing {
        Thing {
            x,
            y: 0,
            angle_deg: 0,
            kind,
        }
    }

    fn map_with_things(things: Vec<Thing>) -> ParsedMap {
        ParsedMap {
            vertices: Vec::new(),
            linedefs: Vec::new(),
            sidedefs: Vec::new(),
            sectors: Vec::new(),
            things,
        }
    }

    #[test]
    fn find_player_spawns_collects_all_matches_in_the_winning_tier() {
        // Three deathmatch starts (kind 11), no single-player/co-op starts —
        // all three must be returned, not just the first.
        let map = map_with_things(vec![thing(11, 0), thing(11, 100), thing(11, 200)]);
        let spawns = find_player_spawns(&map);
        assert_eq!(spawns.len(), 3);
    }

    #[test]
    fn find_player_spawns_prefers_higher_priority_tier() {
        // A single-player start (kind 1) present alongside deathmatch starts
        // must win — only the SP start should be returned.
        let map = map_with_things(vec![thing(1, 0), thing(11, 100), thing(11, 200)]);
        let spawns = find_player_spawns(&map);
        assert_eq!(spawns.len(), 1);
        assert_eq!(spawns[0].kind, 1);
    }

    #[test]
    fn find_player_spawn_for_slot_wraps_when_players_exceed_spawns() {
        let map = map_with_things(vec![thing(11, 0), thing(11, 100)]);
        assert_eq!(find_player_spawn_for_slot(&map, 0).unwrap().x, 0);
        assert_eq!(find_player_spawn_for_slot(&map, 1).unwrap().x, 100);
        // Slot 2 wraps back to spawn 0 with only 2 spawns declared.
        assert_eq!(find_player_spawn_for_slot(&map, 2).unwrap().x, 0);
    }

    #[test]
    fn find_player_spawn_for_slot_returns_none_with_no_spawns() {
        let map = map_with_things(Vec::new());
        assert!(find_player_spawn_for_slot(&map, 0).is_none());
    }
}
