//! Protocol definitions and packet types for Tribes networking

pub mod gameinfo;
pub mod query;
pub mod a2s;

use std::fmt;

/// Represents all possible Tribes protocol packet types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketType {
    /// Ping query packet
    Ping,
    /// GameInfo query packet  
    GameInfo,
    /// Steamworks (A2S) protocol packet
    Steamworks,
    /// Master server packet
    MasterServer,
    /// Unknown packet type
    Unknown(u8),
}

impl fmt::Display for PacketType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PacketType::Ping => write!(f, "Ping"),
            PacketType::GameInfo => write!(f, "GameInfo"),
            PacketType::Steamworks => write!(f, "Steamworks"),
            PacketType::MasterServer => write!(f, "MasterServer"),
            PacketType::Unknown(b) => write!(f, "Unknown(0x{:02x})", b),
        }
    }
}
