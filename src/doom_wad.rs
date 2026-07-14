use std::io::{Cursor, Read};
use std::path::Path;

use bevy::math::primitives::{Cuboid, Plane3d};
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
const DOOR_COLLISION_OPEN_THRESHOLD: f32 = 0.88;
const MIN_DOOR_SCALE_Y: f32 = 0.04;
const MAX_TRAVERSABLE_STEP_HEIGHT: f32 = 0.50;
const DOOR_THIN_SECTOR_HEIGHT: f32 = 0.22;
const DOOR_TALL_NEIGHBOR_HEIGHT: f32 = 1.10;

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
    pub start: Vec2,
    pub end: Vec2,
    pub bottom: f32,
    pub closed_height: f32,
    pub open_amount: f32,
    pub hold_timer_secs: f32,
}

#[derive(Resource, Debug, Clone, Default)]
pub struct WadCollisionWorld {
    pub segments: Vec<WallCollisionSegment>,
    pub doors: Vec<WadDoor>,
    pub push_radius: f32,
}

impl WadCollisionWorld {
    pub fn resolve_position(&self, position: Vec3) -> Vec3 {
        if self.segments.is_empty() {
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

pub fn spawn_level_from_env(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Option<LevelSpawnInfo> {
    let Some(wad_path) = std::env::var("DOOM_WAD").ok() else {
        commands.remove_resource::<WadCollisionWorld>();
        return None;
    };
    let map_name = std::env::var("DOOM_MAP").unwrap_or_else(|_| "MAP01".to_string());
    spawn_level_from_selection(commands, meshes, materials, &wad_path, &map_name)
}

pub fn update_wad_doors(
    time: Res<Time>,
    menu_state: Option<Res<crate::ui::EscapeMenuState>>,
    world: Option<ResMut<WadCollisionWorld>>,
    mut queries: ParamSet<(
        Query<&Transform, With<crate::RotatingCube>>,
        Query<&mut Transform, Without<crate::RotatingCube>>,
    )>,
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
        let player_query = queries.p0();
        let Ok(player_transform) = player_query.single() else {
            return;
        };
        Vec2::new(player_transform.translation.x, player_transform.translation.z)
    };

    let dt = time.delta_secs().clamp(0.0, 0.1);

    let mut transforms = queries.p1();
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

        if let Ok(mut transform) = transforms.get_mut(door.entity) {
            let scale_y = (1.0 - door.open_amount).max(MIN_DOOR_SCALE_Y);
            transform.scale.y = scale_y;
            transform.translation.y = door.bottom
                + (door.closed_height * scale_y) * 0.5
                + door.closed_height * door.open_amount;
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
    wad_path: &str,
    map_name: &str,
) -> Option<LevelSpawnInfo> {
    match try_spawn_level_from_selection(commands, meshes, materials, wad_path, map_name) {
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
    wad_path: &str,
    map_name: &str,
) -> Result<LoadedLevel, String> {
    match spawn_level_from_wad(commands, meshes, materials, wad_path, map_name) {
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

fn spawn_level_from_wad(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    wad_path: &str,
    map_name: &str,
) -> Result<(LevelSpawnInfo, WadCollisionWorld, String, String), String> {
    let map_source = load_map_source(wad_path, map_name)?;
    let map = match &map_source.map_data {
        MapSourceData::ClassicWad(wad_bytes) => {
            let lumps = parse_lump_directory(wad_bytes)?;
            parse_map_lumps(wad_bytes, &lumps, &map_source.resolved_map_name)?
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

            let is_door = is_door_linedef(linedef.special)
                || is_door_geometry_candidate(right_floor, right_ceil, left_floor, left_ceil, linedef.special);

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

                let _door_tag = linedef.tag;

                doors.push(WadDoor {
                    entity: door_entity,
                    start,
                    end,
                    bottom,
                    closed_height: door_height,
                    open_amount: 0.0,
                    hold_timer_secs: 0.0,
                });
                collision_segments.push(WallCollisionSegment { start, end });
                spawn_collision_debug_segment(commands, meshes, materials, start, end);
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
                spawn_collision_debug_segment(commands, meshes, materials, start, end);
            }
        } else {
            let middle_texture = right_side
                .map(|side| clean_texture_name(&side.middle_texture))
                .unwrap_or("METAL".to_string());
            spawn_wall_band(commands, meshes, materials, start, end, right_floor, right_ceil, middle_texture);
            collision_segments.push(WallCollisionSegment { start, end });
            spawn_collision_debug_segment(commands, meshes, materials, start, end);
        }
    }

    let extent = (max - min) * 0.5 + Vec2::splat(FLOOR_PADDING);
    let floor_half_extent = extent
        .x
        .max(extent.y)
        .max(MIN_FLOOR_HALF_EXTENT);

    let floor_texture_name = map
        .sectors
        .first()
        .map(|sector| clean_texture_name(&sector.floor_texture))
        .unwrap_or("FLOOR".to_string());
    let _ceiling_texture_name = map
        .sectors
        .first()
        .map(|sector| clean_texture_name(&sector.ceiling_texture))
        .unwrap_or("CEIL".to_string());

    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(floor_half_extent)).mesh().build())),
        MeshMaterial3d(materials.add(texture_to_material(&floor_texture_name))),
        Transform::default(),
        GlobalTransform::default(),
        crate::scene::LevelGeometry,
    ));

    let (player_spawn, player_yaw) = find_player_spawn(&map)
        .map(|spawn| {
            let flat = doom_to_world(Vec2::new(spawn.x as f32, spawn.y as f32)) - center;
            let spawn_pos = Vec3::new(flat.x, crate::player::CUBE_REST_Y, flat.y);
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

        let mut wad_bytes = Vec::new();
        entry
            .read_to_end(&mut wad_bytes)
            .map_err(|error| format!("unable to extract embedded WAD {}: {error}", entry_name))?;

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

        let mut nested_bytes = Vec::new();
        entry
            .read_to_end(&mut nested_bytes)
            .map_err(|error| format!("unable to read nested archive {}: {error}", entry_name))?;

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

        let mut wad_bytes = Vec::new();
        entry
            .read_to_end(&mut wad_bytes)
            .map_err(|error| format!("unable to extract nested WAD {}: {error}", entry_name))?;

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

        let mut text = String::new();
        entry
            .read_to_string(&mut text)
            .map_err(|error| format!("unable to read nested UDMF TEXTMAP {}: {error}", entry_name))?;

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

        let mut nested_bytes = Vec::new();
        entry
            .read_to_end(&mut nested_bytes)
            .map_err(|error| format!("unable to read deeper nested archive {}: {error}", entry_name))?;

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

        let mut text = String::new();
        entry
            .read_to_string(&mut text)
            .map_err(|error| format!("unable to read UDMF TEXTMAP {}: {error}", entry_name))?;

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

    if requested == "MAP01" {
        if markers.iter().any(|name| name == "E1M1") {
            return Ok("E1M1".to_string());
        }
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

    let vertices = parse_vertices(wad_bytes, vertices_lump)?;
    let linedefs = parse_linedefs(wad_bytes, linedefs_lump)?;
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
    if entry.size % 4 != 0 {
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

fn parse_linedefs(wad_bytes: &[u8], entry: &LumpEntry) -> Result<Vec<LineDef>, String> {
    // Doom format uses 14-byte linedefs, Hexen format uses 16-byte linedefs.
    if entry.size % 14 == 0 {
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

    if entry.size % 16 == 0 {
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
    if entry.size % 30 != 0 {
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
    if entry.size % 26 != 0 {
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
    if entry.size % 10 != 0 {
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

fn find_player_spawn(map: &ParsedMap) -> Option<Thing> {
    // Prefer the single-player start (type 1), then co-op starts, then deathmatch.
    const PLAYER_START_PRIORITY: [u16; 5] = [1, 2, 3, 4, 11];

    for thing_kind in PLAYER_START_PRIORITY {
        if let Some(spawn) = map
            .things
            .iter()
            .copied()
            .find(|thing| thing.kind == thing_kind)
        {
            return Some(spawn);
        }
    }

    None
}

fn doom_to_world(point: Vec2) -> Vec2 {
    Vec2::new(point.x * DOOM_TO_WORLD_SCALE, -point.y * DOOM_TO_WORLD_SCALE)
}

fn doom_angle_to_yaw(angle_deg: i16) -> f32 {
    let radians = (angle_deg as f32).to_radians();
    let world_direction = Vec2::new(radians.cos(), -radians.sin());
    world_direction.x.atan2(-world_direction.y)
}

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
) {
    let segment = end - start;
    let length = segment.length();
    if length <= 0.001 {
        return;
    }

    let midpoint = (start + end) * 0.5;
    let yaw = segment.x.atan2(segment.y);

    commands.spawn((
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

fn is_door_linedef(special: u16) -> bool {
    matches!(
        special,
        1 | 26 | 27 | 28 | 31 | 32 | 33 | 34 | 117 | 118
    )
}

fn is_door_geometry_candidate(
    right_floor: f32,
    right_ceil: f32,
    left_floor: f32,
    left_ceil: f32,
    special: u16,
) -> bool {
    if special == 0 {
        return false;
    }

    let floor_diff = (right_floor - left_floor).abs();
    let right_height = (right_ceil - right_floor).abs();
    let left_height = (left_ceil - left_floor).abs();
    let min_height = right_height.min(left_height);
    let max_height = right_height.max(left_height);

    floor_diff <= 0.24
        && min_height <= DOOR_THIN_SECTOR_HEIGHT
        && max_height >= DOOR_TALL_NEIGHBOR_HEIGHT
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

        if c == '"' {
            in_string = !in_string;
            out.push(c);
            index += 1;
            continue;
        }

        if !in_string && c == '/' && next == Some('/') {
            index += 2;
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            continue;
        }

        if !in_string && c == '/' && next == Some('*') {
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

    out
}

fn next_udmf_block(text: &str, start: usize) -> Option<(String, String, usize)> {
    let bytes = text.as_bytes();
    let mut i = start;

    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }

    let name_start = i;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    if i == name_start {
        return None;
    }
    let name = text[name_start..i].trim().to_ascii_lowercase();

    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'{' {
        return None;
    }

    i += 1;
    let body_start = i;
    let mut depth = 1usize;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let body = text[body_start..i].to_string();
                    return Some((name, body, i + 1));
                }
            }
            _ => {}
        }
        i += 1;
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
    for statement in body.split(';') {
        let statement = statement.trim();
        if statement.is_empty() {
            continue;
        }
        let (lhs, rhs) = statement.split_once('=')?;
        if lhs.trim().eq_ignore_ascii_case(key) {
            return Some(rhs.trim());
        }
    }
    None
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
