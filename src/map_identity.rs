//! Content identity for a selected map. Local file paths are not part of the identity.

use sha2::{Digest, Sha256};
use std::io::{self, Read};
use std::path::Path;

pub const MAX_MAP_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapIdentity(pub [u8; 32]);

impl MapIdentity {
    pub fn from_reader(mut reader: impl Read, map_name: &str) -> io::Result<Self> {
        if map_name.is_empty() || map_name.len() > 255 || map_name.chars().any(char::is_control) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid map name"));
        }
        let mut hash = Sha256::new();
        hash.update(b"chicken-gun/map-identity/v1\0");
        hash.update((map_name.len() as u16).to_le_bytes());
        hash.update(map_name.as_bytes());
        let mut bytes = [0u8; 65536];
        let mut total = 0u64;
        loop {
            let count = reader.read(&mut bytes)?;
            if count == 0 { break; }
            total += count as u64;
            if total > MAX_MAP_BYTES {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Map file exceeds the size limit"));
            }
            hash.update(&bytes[..count]);
        }
        if total == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Map file is empty"));
        }
        Ok(Self(hash.finalize().into()))
    }

    pub fn from_file(path: &Path, map_name: &str) -> io::Result<Self> {
        Self::from_reader(std::fs::File::open(path)?, map_name)
    }
}

/// Keep the default arena protocol unchanged. Bind selected maps to authentication.
pub fn configured_auth_secret(base_secret: &str) -> Result<String, String> {
    let path = match std::env::var("DOOM_WAD") {
        Ok(path) => path,
        Err(std::env::VarError::NotPresent) => return Ok(base_secret.to_owned()),
        Err(_) => return Err("The map path is not valid UTF-8".to_owned()),
    };
    let name = match std::env::var("DOOM_MAP") {
        Ok(name) => name,
        Err(std::env::VarError::NotPresent) => "MAP01".to_owned(),
        Err(_) => return Err("The map name is not valid UTF-8".to_owned()),
    };
    let identity = MapIdentity::from_file(Path::new(&path), &name)
        .map_err(|error| format!("Cannot identify the selected map: {error}"))?;
    Ok(crate::auth::bind_map_secret(base_secret, &identity.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_changes_with_content_or_selection() {
        let first = MapIdentity::from_reader(&b"fixture-a"[..], "MAP01").unwrap();
        assert_eq!(first, MapIdentity::from_reader(&b"fixture-a"[..], "MAP01").unwrap());
        assert_ne!(first, MapIdentity::from_reader(&b"fixture-b"[..], "MAP01").unwrap());
        assert_ne!(first, MapIdentity::from_reader(&b"fixture-a"[..], "MAP02").unwrap());
    }

    #[test]
    fn invalid_inputs_and_read_errors_do_not_produce_identities() {
        assert!(MapIdentity::from_reader(&b""[..], "MAP01").is_err());
        for name in ["".to_owned(), "x".repeat(256), "MAP01\n".to_owned()] {
            assert!(MapIdentity::from_reader(&b"fixture"[..], &name).is_err());
        }
        assert!(MapIdentity::from_reader(io::repeat(0).take(MAX_MAP_BYTES + 1), "MAP01").is_err());
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> { Err(io::Error::other("Test read failure")) }
        }
        assert!(MapIdentity::from_reader(Broken, "MAP01").is_err());
    }
}
