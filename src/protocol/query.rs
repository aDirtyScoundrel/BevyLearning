//! Query protocol - Game server discovery and queries
//!
//! Supports Ping, GameInfo, Steamworks (A2S), and Master server query protocols.
//! Each protocol has a different format and encoding.

use std::io;
use crate::protocol::a2s::A2S_HEADER;

/// Query packet types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryType {
    /// Ping request (0x01) or response (0x02)
    Ping,
    /// GameInfo query request (0x07) or response (0x08)
    GameInfo,
    /// Steamworks (A2S) query request/response
    Steamworks,
    /// Master server list query
    MasterServer,
}

/// Query packet structure with protocol magic and key echo
#[derive(Debug, Clone)]
pub struct Query {
    /// Protocol magic byte
    pub magic: u8,
    /// Key echo value (for validation, 2 bytes little-endian)
    pub key_echo: u16,
    /// Query type
    pub query_type: QueryType,
    /// Payload data (query-specific)
    pub payload: Vec<u8>,
}

impl Query {
    /// Creates a new query with the given magic byte
    pub fn new(magic: u8, query_type: QueryType) -> Self {
        Query {
            magic,
            key_echo: 0,
            query_type,
            payload: Vec::new(),
        }
    }

    /// Parses a Query packet from raw bytes
    ///
    /// Expected format:
    /// - Byte 0: Protocol magic byte (0x10, 0x08, 0x62, etc.)
    /// - Bytes 1-2: Key echo (little-endian u16)
    /// - Bytes 3+: Query-specific payload
    pub fn from_bytes(data: &[u8]) -> io::Result<Self> {
        if data.len() < 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Query packet too short (min 3 bytes)",
            ));
        }

        let magic = data[0];
        let key_echo = u16::from_le_bytes([data[1], data[2]]);

        let query_type = match magic {
            0x01 | 0x02 => QueryType::Ping,
            0x07 | 0x08 => QueryType::GameInfo,
            // Transitional marker for Steamworks migration.
            // TODO: move to raw A2S framing (0xFF 0xFF 0xFF 0xFF + query type).
            0x62 | 0x63 => QueryType::Steamworks,
            _ => QueryType::MasterServer,
        };

        Ok(Query {
            magic,
            key_echo,
            query_type,
            payload: data[3..].to_vec(),
        })
    }

    /// Converts query to raw bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(3 + self.payload.len());
        bytes.push(self.magic);
        bytes.extend_from_slice(&self.key_echo.to_le_bytes());
        bytes.extend_from_slice(&self.payload);
        bytes
    }

    /// Converts a Steamworks query to raw A2S wire packet bytes.
    ///
    /// TODO: once migration is complete, Steamworks queries should bypass `Query`
    /// and use A2S-native structs directly.
    pub fn to_a2s_packet(&self) -> io::Result<Vec<u8>> {
        if self.query_type != QueryType::Steamworks {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "to_a2s_packet is only valid for Steamworks queries",
            ));
        }

        let mut bytes = Vec::with_capacity(4 + self.payload.len());
        bytes.extend_from_slice(&A2S_HEADER);
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }

    /// Builds a Steamworks query wrapper from a raw A2S packet.
    pub fn steamworks_from_a2s_packet(data: &[u8]) -> io::Result<Self> {
        if data.len() < 5 || data[0..4] != A2S_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a valid A2S packet",
            ));
        }

        Ok(Query {
            magic: 0x62,
            key_echo: 0,
            query_type: QueryType::Steamworks,
            payload: data[4..].to_vec(),
        })
    }

    /// Creates a Ping query request
    pub fn ping_request() -> Self {
        Query::new(0x01, QueryType::Ping)
    }

    /// Creates a Ping query response
    pub fn ping_response(key_echo: u16) -> Self {
        let mut query = Query::new(0x02, QueryType::Ping);
        query.key_echo = key_echo;
        query
    }

    /// Creates a GameInfo query request
    pub fn gameinfo_request() -> Self {
        Query::new(0x07, QueryType::GameInfo)
    }

    /// Creates a GameInfo query response
    pub fn gameinfo_response(key_echo: u16) -> Self {
        let mut query = Query::new(0x08, QueryType::GameInfo);
        query.key_echo = key_echo;
        query
    }

    /// Creates a Steamworks (A2S_INFO) query request.
    ///
    /// TODO: move this to pure A2S UDP framing and remove Query header wrapping.
    pub fn steamworks_info_request() -> Self {
        let mut query = Query::new(0x62, QueryType::Steamworks);
        query.payload = Self::a2s_info_payload();
        query
    }

    /// Creates a Steamworks (A2S_PLAYER) query request with challenge.
    ///
    /// `challenge` should be -1 for initial challenge request.
    pub fn steamworks_player_request(challenge: i32) -> Self {
        let mut query = Query::new(0x62, QueryType::Steamworks);
        query.payload.push(0x55); // A2S_PLAYER
        query.payload.extend_from_slice(&challenge.to_le_bytes());
        query
    }

    /// Creates a Steamworks (A2S_RULES) query request with challenge.
    ///
    /// `challenge` should be -1 for initial challenge request.
    pub fn steamworks_rules_request(challenge: i32) -> Self {
        let mut query = Query::new(0x62, QueryType::Steamworks);
        query.payload.push(0x56); // A2S_RULES
        query.payload.extend_from_slice(&challenge.to_le_bytes());
        query
    }

    /// Creates a Steamworks query response wrapper.
    ///
    /// TODO: replace with raw A2S response packets in dedicated A2S server module.
    pub fn steamworks_response(key_echo: u16, payload: Vec<u8>) -> Self {
        let mut query = Query::new(0x63, QueryType::Steamworks);
        query.key_echo = key_echo;
        query.payload = payload;
        query
    }

    fn a2s_info_payload() -> Vec<u8> {
        // A2S_INFO body begins with 0x54 and "Source Engine Query\0".
        let mut payload = vec![0x54];
        payload.extend_from_slice(b"Source Engine Query\0");
        payload
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ping_request_creation() {
        let query = Query::ping_request();
        assert_eq!(query.magic, 0x01);
        assert_eq!(query.query_type, QueryType::Ping);
    }

    #[test]
    fn test_ping_response_creation() {
        let query = Query::ping_response(0x1234);
        assert_eq!(query.magic, 0x02);
        assert_eq!(query.key_echo, 0x1234);
        assert_eq!(query.query_type, QueryType::Ping);
    }

    #[test]
    fn test_gameinfo_request_creation() {
        let query = Query::gameinfo_request();
        assert_eq!(query.magic, 0x07);
        assert_eq!(query.query_type, QueryType::GameInfo);
    }

    #[test]
    fn test_query_roundtrip() {
        let original = Query::ping_request();
        let bytes = original.to_bytes();
        let parsed = Query::from_bytes(&bytes).unwrap();

        assert_eq!(parsed.magic, original.magic);
        assert_eq!(parsed.query_type, original.query_type);
    }

    #[test]
    fn test_query_with_key_echo() {
        let mut original = Query::gameinfo_request();
        original.key_echo = 0xABCD;
        original.payload = vec![0x01, 0x02, 0x03];

        let bytes = original.to_bytes();
        let parsed = Query::from_bytes(&bytes).unwrap();

        assert_eq!(parsed.key_echo, 0xABCD);
        assert_eq!(parsed.payload, vec![0x01, 0x02, 0x03]);
    }

    #[test]
    fn test_steamworks_queries() {
        let req = Query::steamworks_info_request();
        assert_eq!(req.magic, 0x62);
        assert_eq!(req.query_type, QueryType::Steamworks);
        assert_eq!(req.payload[0], 0x54);

        let player = Query::steamworks_player_request(-1);
        assert_eq!(player.payload[0], 0x55);
        assert_eq!(&player.payload[1..5], &(-1_i32).to_le_bytes());

        let resp = Query::steamworks_response(0x5678, vec![0x49, 0x00]);
        assert_eq!(resp.magic, 0x63);
        assert_eq!(resp.key_echo, 0x5678);
        assert_eq!(resp.payload, vec![0x49, 0x00]);
    }

    #[test]
    fn test_steamworks_a2s_packet_roundtrip() {
        let query = Query::steamworks_info_request();
        let wire = query.to_a2s_packet().unwrap();
        let parsed = Query::steamworks_from_a2s_packet(&wire).unwrap();

        assert_eq!(parsed.query_type, QueryType::Steamworks);
        assert_eq!(parsed.payload, query.payload);
    }
}
