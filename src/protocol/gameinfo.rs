//! GameInfo protocol - Core Tribes game server query protocol
//!
//! This is the primary protocol used for querying game server information.
//! Uses BitStream with Huffman compression for efficient encoding.

use std::io;
use crate::bitstream::{BitStreamReader, BitStreamWriter};
use crate::huffman::HuffmanCodec;

/// Player information in GameInfo packet
#[derive(Debug, Clone)]
pub struct PlayerInfo {
    /// Player name
    pub name: String,
    /// Player score
    pub score: i32,
    /// Player ping in milliseconds
    pub ping: u32,
}

/// GameInfo packet structure - Server information query response
#[derive(Debug, Clone)]
pub struct GameInfo {
    /// Server information flags
    pub flags: u32,
    /// Server name (Huffman compressed)
    pub name: String,
    /// Game type (e.g., "CTF", "DeathMatch", "Deathmatch", etc.)
    pub game_type: String,
    /// Mission/map name
    pub mission: String,
    /// Current number of players
    pub player_count: u32,
    /// Maximum number of players
    pub max_players: u32,
    /// Server CPU speed (MHz, optional)
    pub cpu_speed: u32,
    /// Server bandwidth (kbps, optional)
    pub bandwidth: u32,
    /// Player list
    pub players: Vec<PlayerInfo>,
}

impl GameInfo {
    /// Creates a new empty GameInfo packet
    pub fn new() -> Self {
        GameInfo {
            flags: 0,
            name: String::new(),
            game_type: String::new(),
            mission: String::new(),
            player_count: 0,
            max_players: 0,
            cpu_speed: 0,
            bandwidth: 0,
            players: Vec::new(),
        }
    }

    /// Parses GameInfo from a BitStream
    pub fn from_bitstream(reader: &mut BitStreamReader) -> io::Result<Self> {
        let mut info = GameInfo::new();

        // Read server flags (32 bits) - naturally aligned
        info.flags = reader.read_u32()?;

        // Read server name (compression flag + length + data, all byte-aligned)
        info.name = Self::read_huffman_string(reader)?;

        // Read game type
        info.game_type = Self::read_huffman_string(reader)?;

        // Read mission name
        info.mission = Self::read_huffman_string(reader)?;

        // Read player count and max players (4 bits each)
        info.player_count = reader.read_bits(4)?;
        info.max_players = reader.read_bits(4)?;

        // Read additional server info if available
        info.cpu_speed = reader.read_bits(16)?;
        info.bandwidth = reader.read_bits(16)?;

        // Read player list
        for _ in 0..info.player_count {
            let player_name = Self::read_huffman_string(reader)?;
            let score = reader.read_bits(16)? as i32;
            let ping = reader.read_bits(16)?;

            info.players.push(PlayerInfo {
                name: player_name,
                score,
                ping,
            });
        }

        Ok(info)
    }

    /// Writes GameInfo to a BitStream
    pub fn to_bitstream(&self, writer: &mut BitStreamWriter) -> io::Result<()> {
        // Write server flags (32 bits) - naturally aligned
        writer.write_u32(self.flags)?;

        // Write server name
        Self::write_huffman_string(writer, &self.name)?;

        // Write game type
        Self::write_huffman_string(writer, &self.game_type)?;

        // Write mission name
        Self::write_huffman_string(writer, &self.mission)?;

        // Write player count and max players (4 bits each)
        let player_count = (self.players.len() as u32).min(15);
        writer.write_bits(player_count, 4)?;
        writer.write_bits(self.max_players.min(15), 4)?;

        // Write additional server info
        writer.write_bits(self.cpu_speed.min(65535), 16)?;
        writer.write_bits(self.bandwidth.min(65535), 16)?;

        // Write player list
        for player in &self.players {
            Self::write_huffman_string(writer, &player.name)?;
            writer.write_bits((player.score as u32) & 0xFFFF, 16)?;
            writer.write_bits(player.ping.min(65535), 16)?;
        }

        Ok(())
    }

    /// Helper: read a Huffman-encoded string from the stream
    fn read_huffman_string(reader: &mut BitStreamReader) -> io::Result<String> {
        // Strings are byte-aligned Huffman-coded blocks. Align first.
        reader.align();

        let rem = reader.remaining_bytes();

        let codec = HuffmanCodec::new();
        let (s, consumed) = codec.decode_string(rem)?;

        // Advance outer reader by the consumed bytes
        reader.skip_bits(consumed * 8)?;

        Ok(s)
    }

    /// Helper: write a Huffman-encoded string to the stream
    fn write_huffman_string(
        writer: &mut BitStreamWriter,
        s: &str,
    ) -> io::Result<()> {
        // Use Huffman codec to encode the string as a byte-aligned block
        let codec = HuffmanCodec::new();
        let encoded = codec.encode_string(s)?;

        // Ensure byte alignment then write encoded bytes
        writer.align();
        for b in encoded {
            writer.write_byte(b)?;
        }

        Ok(())
    }
}

impl Default for GameInfo {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gameinfo_creation() {
        let info = GameInfo::new();
        assert_eq!(info.player_count, 0);
        assert_eq!(info.max_players, 0);
        assert!(info.players.is_empty());
    }

    #[test]
    fn test_gameinfo_roundtrip() {
        let mut original = GameInfo::new();
        original.name = "Test".to_string();
        original.game_type = "CTF".to_string();
        original.mission = "Map".to_string();
        original.player_count = 2;
        original.max_players = 8;

        // Serialize
        let mut writer = BitStreamWriter::new();
        original.to_bitstream(&mut writer).unwrap();
        let bytes = writer.as_bytes().to_vec();

        println!("Serialized {} bytes", bytes.len());

        // Deserialize
        let mut reader = BitStreamReader::new(&bytes);
        let parsed = GameInfo::from_bitstream(&mut reader).unwrap();

        assert_eq!(parsed.name, original.name);
        assert_eq!(parsed.game_type, original.game_type);
        assert_eq!(parsed.mission, original.mission);
    }
}
