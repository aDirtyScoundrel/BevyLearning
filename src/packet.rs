//! Packet structure and parsing for Tribes protocol
//!
//! Handles generic packet wrapping/unwrapping around BitStream payloads.
//! Implements minimum packet size padding and header parsing.

use std::io;
use crate::bitstream::{BitStreamReader, BitStreamWriter};

/// Minimum packet size in bytes (some ISPs block very small packets)
const MIN_PACKET_SIZE: usize = 8;

/// Generic Tribes network packet
#[derive(Debug, Clone)]
pub struct Packet {
    /// Protocol magic byte(s) identifying packet type
    pub magic: u8,
    /// Key echo value (2 bytes, little-endian) for validation
    pub key_echo: u16,
    /// Connection identifier or session info (optional)
    pub connection_id: u32,
    /// Payload data (BitStream content)
    pub payload: Vec<u8>,
}

impl Packet {
    /// Creates a new packet with the given magic byte
    pub fn new(magic: u8) -> Self {
        Packet {
            magic,
            key_echo: 0,
            connection_id: 0,
            payload: Vec::new(),
        }
    }

    /// Parses a packet from raw bytes
    ///
    /// Format:
    /// - Byte 0: Magic byte (protocol identifier)
    /// - Bytes 1-2: Key echo (little-endian u16)
    /// - Bytes 3+: Payload
    pub fn from_bytes(data: &[u8]) -> io::Result<Self> {
        if data.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Packet cannot be empty",
            ));
        }

        let magic = data[0];

        let key_echo = if data.len() >= 3 {
            u16::from_le_bytes([data[1], data[2]])
        } else {
            0
        };

        let payload = if data.len() > 3 {
            data[3..].to_vec()
        } else {
            Vec::new()
        };

        Ok(Packet {
            magic,
            key_echo,
            connection_id: 0,
            payload,
        })
    }

    /// Converts packet to raw bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(3 + self.payload.len());
        bytes.push(self.magic);
        bytes.extend_from_slice(&self.key_echo.to_le_bytes());
        bytes.extend_from_slice(&self.payload);
        bytes
    }

    /// Applies minimum packet size padding (8 bytes default)
    pub fn apply_padding(&mut self) {
        self.apply_padding_to(MIN_PACKET_SIZE);
    }

    /// Applies custom minimum packet size padding
    pub fn apply_padding_to(&mut self, min_size: usize) {
        let total_size = 3 + self.payload.len(); // magic (1) + key_echo (2) + payload
        if total_size < min_size {
            let padding_needed = min_size - total_size;
            self.payload.resize(self.payload.len() + padding_needed, 0);
        }
    }

    /// Returns the total packet size including header
    pub fn packet_size(&self) -> usize {
        3 + self.payload.len()
    }

    /// Wraps a BitStream as payload
    pub fn with_bitstream_payload(mut self, writer: &BitStreamWriter) -> Self {
        self.payload = writer.as_bytes().to_vec();
        self
    }

    /// Returns payload as a BitStream reader
    pub fn payload_as_bitstream(&self) -> BitStreamReader<'_> {
        BitStreamReader::new(&self.payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_creation() {
        let packet = Packet::new(0x10);
        assert_eq!(packet.magic, 0x10);
        assert_eq!(packet.key_echo, 0);
        assert!(packet.payload.is_empty());
    }

    #[test]
    fn test_packet_roundtrip() {
        let mut original = Packet::new(0x08);
        original.key_echo = 0x1234;
        original.payload = vec![0xAA, 0xBB, 0xCC];

        let bytes = original.to_bytes();
        let parsed = Packet::from_bytes(&bytes).unwrap();

        assert_eq!(parsed.magic, original.magic);
        assert_eq!(parsed.key_echo, original.key_echo);
        assert_eq!(parsed.payload, original.payload);
    }

    #[test]
    fn test_padding() {
        let mut packet = Packet::new(0x10);
        packet.payload = vec![0x01, 0x02];
        packet.apply_padding();

        let total_size = packet.packet_size();
        assert!(total_size >= MIN_PACKET_SIZE);
        assert_eq!(total_size, MIN_PACKET_SIZE);
    }

    #[test]
    fn test_padding_already_large() {
        let mut packet = Packet::new(0x10);
        packet.payload = vec![0; 20];
        let original_size = packet.packet_size();

        packet.apply_padding();

        assert_eq!(packet.packet_size(), original_size);
    }

    #[test]
    fn test_packet_size() {
        let mut packet = Packet::new(0x08);
        assert_eq!(packet.packet_size(), 3); // magic + key_echo

        packet.payload = vec![1, 2, 3, 4, 5];
        assert_eq!(packet.packet_size(), 8);
    }

    #[test]
    fn test_bitstream_payload() {
        let mut writer = BitStreamWriter::new();
        writer.write_byte(0xFF).unwrap();
        writer.write_u16(0x1234).unwrap();

        let mut packet = Packet::new(0x07);
        packet = packet.with_bitstream_payload(&writer);

        let mut reader = packet.payload_as_bitstream();
        assert_eq!(reader.read_byte().unwrap(), 0xFF);
        assert_eq!(reader.read_u16().unwrap(), 0x1234);
    }
}
