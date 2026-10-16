use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct MarathonMap {
    pub name: String,
    pub vertices: Vec<[i16; 2]>,
    pub lines: Vec<Line>,
    pub polygons: Vec<Polygon>,
    pub objects: Vec<MapObject>,
    pub entry_flags: u32,
    pub chunks: BTreeMap<[u8; 4], Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub vertices: [u16; 2],
    pub flags: u16,
    pub sides: [i16; 2],
    pub polygons: [i16; 2],
}

#[derive(Debug, Clone)]
pub struct Polygon {
    pub kind: i16,
    pub flags: u16,
    pub permutation: i16,
    pub vertices: Vec<u16>,
    pub lines: Vec<u16>,
    pub floor_texture: u16,
    pub ceiling_texture: u16,
    pub floor: i16,
    pub ceiling: i16,
    pub floor_light: i16,
    pub ceiling_light: i16,
    pub adjacent: Vec<i16>,
    pub sides: Vec<i16>,
    pub floor_origin: [i16; 2],
    pub ceiling_origin: [i16; 2],
    pub media: i16,
}

#[derive(Debug, Clone)]
pub struct MapObject {
    pub kind: i16,
    pub index: i16,
    pub facing: i16,
    pub polygon: u16,
    pub position: [i16; 3],
    pub flags: u16,
}

const MAX_FILE_SIZE: usize = 64 * 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 4096;
const MAX_CHUNKS: usize = 4096;
const MAX_RECORDS: usize = 32767;
const MAX_VERTICES_PER_POLYGON: usize = 8;

fn span(data: &[u8], offset: usize, length: usize) -> Result<&[u8], String> {
    let end = offset.checked_add(length).ok_or("Byte range overflow")?;
    data.get(offset..end).ok_or_else(|| "Byte range exceeds file size".into())
}
fn bytes<const N: usize>(data: &[u8], offset: usize) -> Result<[u8; N], String> {
    span(data, offset, N)?.try_into().map_err(|_| "Invalid field size".into())
}
fn read_be_i16(data: &[u8], offset: usize) -> Result<i16, String> { Ok(i16::from_be_bytes(bytes(data, offset)?)) }
fn read_be_u16(data: &[u8], offset: usize) -> Result<u16, String> { Ok(u16::from_be_bytes(bytes(data, offset)?)) }
fn read_be_i32(data: &[u8], offset: usize) -> Result<i32, String> { Ok(i32::from_be_bytes(bytes(data, offset)?)) }
fn read_be_u32(data: &[u8], offset: usize) -> Result<u32, String> { Ok(u32::from_be_bytes(bytes(data, offset)?)) }
fn size(value: i32) -> Result<usize, String> { usize::try_from(value).map_err(|_| "Negative size or offset".into()) }

#[derive(Debug)]
struct Header { version: i16, data_version: i16, directory: usize, directory_end: usize, count: usize, stride: usize, entry_size: usize }
fn header(data: &[u8]) -> Result<Header, String> {
    span(data, 0, 128)?;
    let version = read_be_i16(data, 0)?;
    let data_version = read_be_i16(data, 2)?;
    if ![0, 1, 2, 4].contains(&version) || ![0, 1].contains(&data_version) {
        return Err("Unsupported Marathon map version".into());
    }
    if read_be_u32(data, 84)? != 0 { return Err("Overlay maps require a parent map".into()); }
    let directory = size(read_be_i32(data, 72)?)?;
    let count = size(read_be_i16(data, 76)? as i32)?;
    if directory < 128 || count == 0 || count > MAX_DIRECTORY_ENTRIES { return Err("Invalid map directory".into()); }
    let app_size = size(read_be_i16(data, 78)? as i32)?;
    let (base_size, entry_size) = match version {
        0 => (8, 12),
        1 => (10, 12),
        _ => {
            let base = size(read_be_i16(data, 82)? as i32)?;
            let entry = size(read_be_i16(data, 80)? as i32)?;
            if base < 10 || entry < 16 { return Err("Invalid map record sizes".into()); }
            (base, entry)
        }
    };
    let stride = base_size.checked_add(app_size).ok_or("Directory stride overflow")?;
    let length = count.checked_mul(stride).ok_or("Directory size overflow")?;
    span(data, directory, length)?;
    Ok(Header { version, data_version, directory, directory_end: directory + length, count, stride, entry_size })
}
fn native(data: &[u8]) -> Result<(&[u8], Header), String> {
    if data.len() > MAX_FILE_SIZE { return Err("Map file exceeds 64 MiB".into()); }
    if let Ok(header) = header(data) { return Ok((data, header)); }
    if data.first() != Some(&0) || !data.get(1).is_some_and(|n| (1..=63).contains(n))
        || data.get(74) != Some(&0) || data.get(82) != Some(&0) {
        return Err("Invalid native map or MacBinary header".into());
    }
    let fork = span(data, 128, read_be_u32(data, 83)? as usize)?;
    Ok((fork, header(fork)?))
}
fn directory(data: &[u8], header: &Header) -> Result<Vec<(u16, usize, usize)>, String> {
    let mut entries = Vec::with_capacity(header.count);
    let mut ids = std::collections::BTreeSet::new();
    for index in 0..header.count {
        let record = header.directory + index * header.stride;
        let offset = size(read_be_i32(data, record)?)?;
        let length = size(read_be_i32(data, record + 4)?)?;
        span(data, offset, length)?;
        if offset < 128 || length < header.entry_size || (offset < header.directory_end && offset + length > header.directory) {
            return Err("Level data overlaps the map directory or header".into());
        }
        let id = if header.version == 0 { index as u16 } else { read_be_u16(data, record + 8)? };
        if !ids.insert(id) { return Err("Duplicate level index".into()); }
        entries.push((id, offset, length));
    }
    Ok(entries)
}
pub fn list_levels(data: &[u8]) -> Result<Vec<u16>, String> {
    let (data, header) = native(data)?;
    Ok(directory(data, &header)?.into_iter().map(|entry| entry.0).collect())
}
pub fn parse(data: &[u8], level_index: u16) -> Result<MarathonMap, String> {
    let (data, header) = native(data)?;
    let (_, offset, length) = directory(data, &header)?.into_iter().find(|entry| entry.0 == level_index)
        .ok_or_else(|| format!("Level {level_index} does not exist"))?;
    parse_level(span(data, offset, length)?, &header, level_index)
}
fn parse_level(data: &[u8], header: &Header, level_index: u16) -> Result<MarathonMap, String> {
    let mut chunks = BTreeMap::new();
    let mut offset = 0usize;
    loop {
        if chunks.len() >= MAX_CHUNKS { return Err("Too many map chunks".into()); }
        span(data, offset, header.entry_size)?;
        let tag = bytes::<4>(data, offset)?;
        let next = size(read_be_i32(data, offset + 4)?)?;
        let length = size(read_be_i32(data, offset + 8)?)?;
        if header.version >= 2 && read_be_i32(data, offset + 12)? != 0 { return Err("Chunk overlays are not supported".into()); }
        let start = offset.checked_add(header.entry_size).ok_or("Chunk offset overflow")?;
        let payload = span(data, start, length)?;
        if chunks.insert(tag, payload.to_vec()).is_some() { return Err("Duplicate chunk tag".into()); }
        if next == 0 { break; }
        if next < start + length || next <= offset || next >= data.len() { return Err("Invalid chunk link".into()); }
        offset = next;
    }
    let mut map = MarathonMap { name: format!("Level {level_index}"), vertices: Vec::new(), lines: Vec::new(), polygons: Vec::new(), objects: Vec::new(), entry_flags: 0, chunks };
    let side_bytes = map.chunks.get(b"SIDS").map_or(0, |data| data.len());
    if side_bytes % 64 != 0 { return Err("Invalid SIDS size".into()); }
    let side_count = side_bytes / 64;
    if let Some(minf_data) = map.chunks.get(b"Minf") {
        if minf_data.len() != 88 { return Err("Invalid Minf size".into()); }
        {
            let name_bytes = span(minf_data, 18, 66)?;
            let name = String::from_utf8_lossy(name_bytes.split(|b| *b == 0).next().unwrap_or_default()).into_owned();
            if !name.is_empty() {
                map.name = name;
            }
            map.entry_flags = read_be_u32(minf_data, 84)?;
        }
    }
    
    let vertex_count;
    if let Some(epnt_data) = map.chunks.get(b"EPNT") {
        if epnt_data.len() % 16 != 0 {
            return Err("Invalid EPNT length".to_string());
        }
        vertex_count = epnt_data.len() / 16;
        if vertex_count > MAX_RECORDS {
            return Err("Too many vertices".to_string());
        }
        for i in 0..vertex_count {
            let offset = i * 16;
            let x = read_be_i16(epnt_data, offset + 6)?;
            let y = read_be_i16(epnt_data, offset + 8)?;
            map.vertices.push([x, y]);
        }
    } else if let Some(pnts_data) = map.chunks.get(b"PNTS") {
        if pnts_data.len() % 4 != 0 {
            return Err("Invalid PNTS length".to_string());
        }
        vertex_count = pnts_data.len() / 4;
        if vertex_count > MAX_RECORDS {
            return Err("Too many vertices".to_string());
        }
        for i in 0..vertex_count {
            let offset = i * 4;
            let x = read_be_i16(pnts_data, offset)?;
            let y = read_be_i16(pnts_data, offset + 2)?;
            map.vertices.push([x, y]);
        }
    } else {
        return Err("Missing PNTS or EPNT chunk".to_string());
    }
    
    if let Some(lins_data) = map.chunks.get(b"LINS") {
        if lins_data.len() % 32 != 0 {
            return Err("Invalid LINS length".to_string());
        }
        let line_count = lins_data.len() / 32;
        if line_count > MAX_RECORDS {
            return Err("Too many lines".to_string());
        }
        for i in 0..line_count {
            let offset = i * 32;
            let v1 = read_be_u16(lins_data, offset)?;
            let v2 = read_be_u16(lins_data, offset + 2)?;
            let flags = read_be_u16(lins_data, offset + 4)?;
            
            
            let side1 = read_be_i16(lins_data, offset + 12)?;
            let side2 = read_be_i16(lins_data, offset + 14)?;
            
            let poly1 = read_be_i16(lins_data, offset + 16)?;
            let poly2 = read_be_i16(lins_data, offset + 18)?;
            
            
            if v1 as usize >= vertex_count || v2 as usize >= vertex_count {
                return Err("Line vertex index out of bounds".to_string());
            }
            
            map.lines.push(Line {
                vertices: [v1, v2],
                flags,
                sides: [side1, side2],
                polygons: [poly1, poly2],
            });
        }
    } else {
        return Err("Missing LINS chunk".to_string());
    }
    
    if let Some(poly_data) = map.chunks.get(b"POLY") {
        if poly_data.len() % 128 != 0 {
            return Err("Invalid POLY length".to_string());
        }
        let poly_count = poly_data.len() / 128;
        if poly_count > MAX_RECORDS {
            return Err("Too many polygons".to_string());
        }
        
        for i in 0..poly_count {
            let offset = i * 128;
            let kind = read_be_i16(poly_data, offset)?;
            let flags = read_be_u16(poly_data, offset + 2)?;
            let permutation = read_be_i16(poly_data, offset + 4)?;
            
            let vertex_count_poly = read_be_i16(poly_data, offset + 6)? as usize;
            if vertex_count_poly < 3 || vertex_count_poly > MAX_VERTICES_PER_POLYGON {
                return Err("Invalid polygon vertex count".to_string());
            }
            
            let mut vertices = Vec::new();
            let mut lines = Vec::new();
            
            for j in 0..MAX_VERTICES_PER_POLYGON {
                let idx = read_be_u16(poly_data, offset + 8 + j * 2)?;
                if j < vertex_count_poly {
                    if idx as usize >= vertex_count {
                        return Err("Polygon vertex index out of bounds".to_string());
                    }
                    vertices.push(idx);
                }
            }
            
            for j in 0..MAX_VERTICES_PER_POLYGON {
                let idx = read_be_u16(poly_data, offset + 8 + 16 + j * 2)?;
                if j < vertex_count_poly {
                    if idx as usize >= map.lines.len() {
                        return Err("Polygon line index out of bounds".to_string());
                    }
                    lines.push(idx);
                }
            }
            
            let floor_texture = read_be_u16(poly_data, offset + 8 + 16 + 16)?;
            let ceiling_texture = read_be_u16(poly_data, offset + 8 + 16 + 16 + 2)?;
            let floor = read_be_i16(poly_data, offset + 8 + 16 + 16 + 4)?;
            let ceiling = read_be_i16(poly_data, offset + 8 + 16 + 16 + 6)?;
            let floor_light = read_be_i16(poly_data, offset + 8 + 16 + 16 + 8)?;
            let ceiling_light = read_be_i16(poly_data, offset + 8 + 16 + 16 + 10)?;
            
            let mut adjacent = Vec::new();
            for j in 0..MAX_VERTICES_PER_POLYGON {
                let idx = read_be_i16(poly_data, offset + 8 + 16 + 16 + 12 + 4 + 2 + 2 + 2 + 2 + 2 + 2 + j * 2)?;
                if j < vertex_count_poly {
                    if idx != -1 && (idx as usize >= poly_count) {
                        return Err("Adjacent polygon index out of bounds".to_string());
                    }
                    adjacent.push(idx);
                }
            }
            
            let mut sides = Vec::new();
            for j in 0..vertex_count_poly {
                let index = read_be_i16(poly_data, offset + 92 + j * 2)?;
                if index != -1 && index as usize >= side_count { return Err("Polygon side index exceeds side count".into()); }
                sides.push(index);
            }
            let floor_origin_x = read_be_i16(poly_data, offset + 108)?;
            let floor_origin_y = read_be_i16(poly_data, offset + 110)?;
            let ceiling_origin_x = read_be_i16(poly_data, offset + 112)?;
            let ceiling_origin_y = read_be_i16(poly_data, offset + 114)?;
            let media = read_be_i16(poly_data, offset + 116)?;
            map.polygons.push(Polygon {
                kind,
                flags,
                permutation,
                vertices,
                lines,
                floor_texture,
                ceiling_texture,
                floor,
                ceiling,
                floor_light,
                ceiling_light,
                adjacent,
                sides,
                floor_origin: [floor_origin_x, floor_origin_y],
                ceiling_origin: [ceiling_origin_x, ceiling_origin_y],
                media,
            });
        }
    } else {
        return Err("Missing POLY chunk".to_string());
    }
    
    if let Some(objs_data) = map.chunks.get(b"OBJS") {
        if objs_data.len() % 16 != 0 {
            return Err("Invalid OBJS length".to_string());
        }
        let obj_count = objs_data.len() / 16;
        if obj_count > MAX_RECORDS {
            return Err("Too many objects".to_string());
        }
        
        for i in 0..obj_count {
            let offset = i * 16;
            let kind = read_be_i16(objs_data, offset)?;
            let index = read_be_i16(objs_data, offset + 2)?;
            let facing = read_be_i16(objs_data, offset + 4)?;
            let polygon = read_be_u16(objs_data, offset + 6)?;
            
            if polygon as usize >= map.polygons.len() {
                return Err("Object polygon index out of bounds".to_string());
            }
            
            let x = read_be_i16(objs_data, offset + 8)?;
            let y = read_be_i16(objs_data, offset + 10)?;
            
            let z;
            let flags;
            
            if header.data_version == 0 {
                z = 0;
                flags = 0;
            } else {
                z = read_be_i16(objs_data, offset + 12)?;
                flags = read_be_u16(objs_data, offset + 14)?;
            }
            
            map.objects.push(MapObject {
                kind,
                index,
                facing,
                polygon,
                position: [x, y, z],
                flags,
            });
        }
    }
    
    for line in &map.lines {
        for index in line.polygons { if index != -1 && index as usize >= map.polygons.len() { return Err("Line polygon index exceeds polygon count".into()); } }
        for index in line.sides { if index != -1 && index as usize >= side_count { return Err("Line side index exceeds side count".into()); } }
    }
    Ok(map)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(version: i16, entry_size: usize) -> Vec<u8> {
        let points: Vec<u8> = [0i16,0,1024,0,0,1024].into_iter().flat_map(i16::to_be_bytes).collect();
        let mut lines = vec![0u8; 96];
        for i in 0..3 {
            let p = i * 32;
            lines[p..p+2].copy_from_slice(&(i as u16).to_be_bytes());
            lines[p+2..p+4].copy_from_slice(&(((i+1)%3) as u16).to_be_bytes());
            for j in [12,14,18] { lines[p+j..p+j+2].copy_from_slice(&(-1i16).to_be_bytes()); }
        }
        let mut polygon = vec![0u8;128];
        polygon[6..8].copy_from_slice(&3i16.to_be_bytes());
        for i in 0..3 {
            for base in [8,24] { polygon[base+i*2..base+i*2+2].copy_from_slice(&(i as u16).to_be_bytes()); }
        }
        for offset in (68..84).chain(92..108).step_by(2) { polygon[offset..offset+2].copy_from_slice(&(-1i16).to_be_bytes()); }
        for (offset,value) in [(44,64i16),(46,1024),(108,7),(110,9),(112,11),(114,13),(116,-1)] {
            polygon[offset..offset+2].copy_from_slice(&value.to_be_bytes());
        }
        let mut object = vec![0u8;16];
        for (offset,value) in [(0,3i16),(4,128),(8,128),(10,256),(12,32)] { object[offset..offset+2].copy_from_slice(&value.to_be_bytes()); }
        let mut info = vec![0u8;88]; info[18..22].copy_from_slice(b"Test"); info[23..26].copy_from_slice(b"old");
        info[84..88].copy_from_slice(&4u32.to_be_bytes());
        let chunks = [(b"PNTS",points),(b"LINS",lines),(b"POLY",polygon),(b"OBJS",object),(b"Minf",info)];
        let mut level = Vec::new();
        for (index,(tag,payload)) in chunks.iter().enumerate() {
            let offset = level.len(); let mut entry = vec![0u8;entry_size];
            entry[..4].copy_from_slice(*tag);
            if index+1 != chunks.len() { entry[4..8].copy_from_slice(&((offset+entry_size+payload.len()) as i32).to_be_bytes()); }
            entry[8..12].copy_from_slice(&(payload.len() as i32).to_be_bytes());
            level.extend_from_slice(&entry);level.extend_from_slice(payload);
        }
        let mut data=vec![0u8;128];
        data[..2].copy_from_slice(&version.to_be_bytes());data[2..4].copy_from_slice(&1i16.to_be_bytes());
        data[72..76].copy_from_slice(&((128+level.len()) as i32).to_be_bytes());
        data[76..78].copy_from_slice(&1i16.to_be_bytes());
        if version>=2 {data[80..82].copy_from_slice(&(entry_size as i16).to_be_bytes());data[82..84].copy_from_slice(&10i16.to_be_bytes());}
        data.extend_from_slice(&level);data.extend_from_slice(&128i32.to_be_bytes());data.extend_from_slice(&(level.len() as i32).to_be_bytes());
        if version!=0 { data.extend_from_slice(&0u16.to_be_bytes()); }
        data
    }
    #[test]
    fn native_versions_and_macbinary_preserve_geometry() {
        for (version,size) in [(0,12),(1,12),(2,16),(4,16),(4,24)] {
            let data=fixture(version,size);assert_eq!(list_levels(&data).unwrap(),[0]);
            let map=parse(&data,0).unwrap();assert_eq!(map.name,"Test");assert_eq!(map.vertices,[[0,0],[1024,0],[0,1024]]);
            assert_eq!((map.polygons[0].floor,map.polygons[0].ceiling),(64,1024));
            assert_eq!(map.polygons[0].floor_origin,[7,9]);assert_eq!(map.polygons[0].ceiling_origin,[11,13]);
            assert_eq!(map.objects[0].position,[128,256,32]);assert_eq!(map.entry_flags,4);
            let mut wrapped=vec![0u8;128];wrapped[1]=4;wrapped[2..6].copy_from_slice(b"Test");
            wrapped[83..87].copy_from_slice(&(data.len() as u32).to_be_bytes());wrapped.extend_from_slice(&data);
            assert_eq!(parse(&wrapped,0).unwrap().name,"Test");
        }
    }
    #[test]
    fn malformed_containers_and_references_return_errors() {
        let good=fixture(4,16);assert!(parse(&good,0).is_ok());
        for length in 0..good.len() { assert!(parse(&good[..length],0).is_err(),"Length {length}"); }
        for (offset,bytes) in [(72,(-1i32).to_be_bytes()),(84,1i32.to_be_bytes()),(132,1i32.to_be_bytes()),(136,(-1i32).to_be_bytes()),(140,1i32.to_be_bytes())] {
            let mut bad=good.clone();bad[offset..offset+4].copy_from_slice(&bytes);assert!(parse(&bad,0).is_err());
        }
        let mut bad=good.clone();bad[172..174].copy_from_slice(&999u16.to_be_bytes());assert!(parse(&bad,0).is_err());
        assert!(span(&good,usize::MAX,2).is_err());
    }
    #[test]
    #[ignore = "Requires the separately downloaded Marathon Infinity map"]
    fn real_infinity_file_contains_57_readable_levels() {
        let root=std::env::var("CHICKEN_REFERENCE_DIR").expect("Set CHICKEN_REFERENCE_DIR");
        let data=std::fs::read(std::path::Path::new(&root).join("MarathonInfinity-Map.sceA")).unwrap();
        let levels=list_levels(&data).unwrap();assert_eq!(levels.len(),57);
        for id in levels { let map=parse(&data,id).unwrap();assert!(!map.vertices.is_empty());assert!(!map.polygons.is_empty()); }
        let first=parse(&data,0).unwrap();assert_eq!(first.name,"Ne Cede Malis");
        assert_eq!((first.vertices.len(),first.lines.len(),first.polygons.len(),first.objects.len()),(1225,1802,560,145));
    }
}
