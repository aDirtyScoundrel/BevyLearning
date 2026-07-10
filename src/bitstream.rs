//! BitStream reader/writer with LSB-first (Little-Endian Bit-order) bit packing
//!
//! Allows reading and writing arbitrary bit counts (1-32 bits) from/to a byte buffer.
//! This is the foundational component for Tribes protocol serialization.

use std::io;

/// BitStream reader for LSB-first (Little-Endian Bit-order) bit packing
#[derive(Debug, Clone)]
pub struct BitStreamReader<'a> {
    data: &'a [u8],
    /// Current bit position within the buffer
    bit_offset: usize,
}

impl<'a> BitStreamReader<'a> {
    /// Creates a new BitStream reader from a byte slice
    pub fn new(data: &'a [u8]) -> Self {
        BitStreamReader {
            data,
            bit_offset: 0,
        }
    }

    /// Returns the number of bits read so far
    pub fn bit_position(&self) -> usize {
        self.bit_offset
    }

    /// Returns the number of bytes read so far (rounded up)
    pub fn byte_position(&self) -> usize {
        self.bit_offset.div_ceil(8)
    }

    /// Returns the number of bits remaining
    pub fn bits_remaining(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.bit_offset)
    }

    /// Returns the number of bytes remaining (from current bit position)
    pub fn bytes_remaining(&self) -> usize {
        self.bits_remaining().div_ceil(8)
    }

    /// Reads up to 32 bits from the stream (LSB-first)
    ///
    /// # Arguments
    /// * `num_bits` - Number of bits to read (1-32)
    ///
    /// # Returns
    /// The bits read as a u32, aligned to the right (LSBs)
    ///
    /// # Example
    /// If the stream contains bytes [0xAB, 0xCD] in LSB-first:
    /// - read_bits(1) returns 0x01 (bit 0 of 0xAB)
    /// - read_bits(8) reads next 8 bits
    pub fn read_bits(&mut self, num_bits: usize) -> io::Result<u32> {
        if num_bits == 0 || num_bits > 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "num_bits must be between 1 and 32",
            ));
        }

        if self.bits_remaining() < num_bits {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "Not enough bits: need {}, have {}",
                    num_bits,
                    self.bits_remaining()
                ),
            ));
        }

        let mut result: u32 = 0;
        let mut bits_read = 0;

        while bits_read < num_bits {
            let byte_idx = self.bit_offset / 8;
            let bit_idx = self.bit_offset % 8;
            let byte_val = self.data[byte_idx];

            // Extract bit from current position (LSB-first)
            let bit = (byte_val >> bit_idx) & 1;

            // Place bit in result at the correct position
            result |= (bit as u32) << bits_read;

            self.bit_offset += 1;
            bits_read += 1;
        }

        Ok(result)
    }

    /// Reads a single bit as a boolean
    pub fn read_bit(&mut self) -> io::Result<bool> {
        Ok(self.read_bits(1)? != 0)
    }

    /// Reads a byte (8 bits)
    pub fn read_byte(&mut self) -> io::Result<u8> {
        Ok(self.read_bits(8)? as u8)
    }

    /// Reads a 16-bit value
    pub fn read_u16(&mut self) -> io::Result<u16> {
        Ok(self.read_bits(16)? as u16)
    }

    /// Reads a 32-bit value
    pub fn read_u32(&mut self) -> io::Result<u32> {
        self.read_bits(32)
    }

    /// Skips ahead by the given number of bits
    pub fn skip_bits(&mut self, num_bits: usize) -> io::Result<()> {
        if self.bits_remaining() < num_bits {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Not enough bits to skip",
            ));
        }
        self.bit_offset += num_bits;
        Ok(())
    }

    /// Aligns to the next byte boundary (skips partial byte if needed)
    pub fn align(&mut self) {
        let remainder = self.bit_offset % 8;
        if remainder != 0 {
            self.bit_offset += 8 - remainder;
        }
    }

    /// Returns the remaining bytes as a slice (starting from current bit position, aligned to byte)
    pub fn remaining_bytes(&self) -> &'a [u8] {
        let byte_pos = self.byte_position();
        if byte_pos < self.data.len() {
            &self.data[byte_pos..]
        } else {
            &[]
        }
    }
}

/// BitStream writer for LSB-first (Little-Endian Bit-order) bit packing
#[derive(Debug)]
pub struct BitStreamWriter {
    data: Vec<u8>,
    /// Current bit position within the buffer
    bit_offset: usize,
}

impl BitStreamWriter {
    /// Creates a new empty BitStream writer
    pub fn new() -> Self {
        BitStreamWriter {
            data: Vec::new(),
            bit_offset: 0,
        }
    }

    /// Creates a BitStream writer with pre-allocated capacity
    pub fn with_capacity(capacity: usize) -> Self {
        BitStreamWriter {
            data: Vec::with_capacity(capacity),
            bit_offset: 0,
        }
    }

    /// Returns the number of bits written so far
    pub fn bit_position(&self) -> usize {
        self.bit_offset
    }

    /// Returns the number of bytes written so far (rounded up)
    pub fn byte_position(&self) -> usize {
        self.bit_offset.div_ceil(8)
    }

    /// Writes up to 32 bits to the stream (LSB-first)
    ///
    /// # Arguments
    /// * `value` - The value to write
    /// * `num_bits` - Number of bits to write (1-32)
    pub fn write_bits(&mut self, mut value: u32, num_bits: usize) -> io::Result<()> {
        if num_bits == 0 || num_bits > 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "num_bits must be between 1 and 32",
            ));
        }

        // Mask the value to the requested bit count
        let mask = if num_bits == 32 {
            0xFFFFFFFF
        } else {
            (1u32 << num_bits) - 1
        };
        value &= mask;

        for i in 0..num_bits {
            let byte_idx = self.bit_offset / 8;
            let bit_idx = self.bit_offset % 8;

            // Extend buffer if needed
            while byte_idx >= self.data.len() {
                self.data.push(0);
            }

            // Extract the bit we want to write (LSB-first)
            let bit = (value >> i) & 1;

            // Set the bit in the buffer
            self.data[byte_idx] |= (bit as u8) << bit_idx;

            self.bit_offset += 1;
        }

        Ok(())
    }

    /// Writes a single bit
    pub fn write_bit(&mut self, value: bool) -> io::Result<()> {
        self.write_bits(if value { 1 } else { 0 }, 1)
    }

    /// Writes a byte (8 bits)
    pub fn write_byte(&mut self, value: u8) -> io::Result<()> {
        self.write_bits(value as u32, 8)
    }

    /// Writes a 16-bit value
    pub fn write_u16(&mut self, value: u16) -> io::Result<()> {
        self.write_bits(value as u32, 16)
    }

    /// Writes a 32-bit value
    pub fn write_u32(&mut self, value: u32) -> io::Result<()> {
        self.write_bits(value, 32)
    }

    /// Pads to the next byte boundary (fills with zeros)
    pub fn align(&mut self) {
        let remainder = self.bit_offset % 8;
        if remainder != 0 {
            let _ = self.write_bits(0, 8 - remainder);
        }
    }

    /// Pads to a minimum packet size (in bytes) by adding zero bytes
    pub fn pad_to_minimum(&mut self, min_size: usize) {
        while self.byte_position() < min_size {
            let _ = self.write_byte(0);
        }
    }

    /// Consumes the writer and returns the underlying bytes
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    /// Returns a reference to the underlying bytes
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Returns the length in bytes (rounded up from bit position)
    pub fn len(&self) -> usize {
        self.byte_position()
    }

    /// Returns true if the writer has no data
    pub fn is_empty(&self) -> bool {
        self.bit_offset == 0
    }
}

impl Default for BitStreamWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_write_single_bits() {
        let mut writer = BitStreamWriter::new();
        writer.write_bit(true).unwrap();
        writer.write_bit(false).unwrap();
        writer.write_bit(true).unwrap();
        writer.write_bit(true).unwrap();
        writer.align();

        let bytes = writer.as_bytes();
        assert_eq!(bytes[0], 0b00001101); // bits 0=1, 1=0, 2=1, 3=1 (LSB-first)

        let mut reader = BitStreamReader::new(bytes);
        assert_eq!(reader.read_bit().unwrap(), true);
        assert_eq!(reader.read_bit().unwrap(), false);
        assert_eq!(reader.read_bit().unwrap(), true);
        assert_eq!(reader.read_bit().unwrap(), true);
    }

    #[test]
    fn test_read_write_bytes() {
        let mut writer = BitStreamWriter::new();
        writer.write_byte(0xAB).unwrap();
        writer.write_byte(0xCD).unwrap();

        let bytes = writer.as_bytes();
        assert_eq!(bytes, &[0xAB, 0xCD]);

        let mut reader = BitStreamReader::new(bytes);
        assert_eq!(reader.read_byte().unwrap(), 0xAB);
        assert_eq!(reader.read_byte().unwrap(), 0xCD);
    }

    #[test]
    fn test_read_write_arbitrary_bits() {
        let mut writer = BitStreamWriter::new();
        writer.write_bits(0xF, 4).unwrap(); // 4 bits: 1111
        writer.write_bits(0x3, 2).unwrap(); // 2 bits: 11
        writer.write_bits(0x0, 2).unwrap(); // 2 bits: 00
        writer.align();

        let bytes = writer.as_bytes();
        // byte 0: bits 0-3 = 1111, bits 4-5 = 11, bits 6-7 = 00 = 0b00111111 = 0x3F
        assert_eq!(bytes[0], 0x3F);

        let mut reader = BitStreamReader::new(bytes);
        assert_eq!(reader.read_bits(4).unwrap(), 0xF);
        assert_eq!(reader.read_bits(2).unwrap(), 0x3);
        assert_eq!(reader.read_bits(2).unwrap(), 0x0);
    }

    #[test]
    fn test_bit_spanning_bytes() {
        let mut writer = BitStreamWriter::new();
        writer.write_bits(0x1FF, 9).unwrap(); // 9 bits spanning byte boundary
        writer.align();

        let bytes = writer.as_bytes();
        assert_eq!(bytes.len(), 2);
        // byte 0: bits 0-7 of 0x1FF = 0xFF
        // byte 1: bits 8-8 of 0x1FF = 0x01
        assert_eq!(bytes[0], 0xFF);
        assert_eq!(bytes[1], 0x01);

        let mut reader = BitStreamReader::new(bytes);
        assert_eq!(reader.read_bits(9).unwrap(), 0x1FF);
    }

    #[test]
    fn test_bits_remaining() {
        let data = vec![0xFF, 0xFF];
        let mut reader = BitStreamReader::new(&data);
        assert_eq!(reader.bits_remaining(), 16);
        reader.read_bits(5).unwrap();
        assert_eq!(reader.bits_remaining(), 11);
    }

    #[test]
    fn test_alignment() {
        let mut writer = BitStreamWriter::new();
        writer.write_bits(0x1, 3).unwrap();
        assert_eq!(writer.bit_position(), 3);
        writer.align();
        assert_eq!(writer.bit_position(), 8);
    }

    #[test]
    fn test_write_u16_u32() {
        let mut writer = BitStreamWriter::new();
        writer.write_u16(0xABCD).unwrap();
        writer.write_u32(0x12345678).unwrap();

        let bytes = writer.as_bytes();
        let mut reader = BitStreamReader::new(bytes);
        assert_eq!(reader.read_u16().unwrap(), 0xABCD);
        assert_eq!(reader.read_u32().unwrap(), 0x12345678);
    }

    #[test]
    fn test_padding() {
        let mut writer = BitStreamWriter::new();
        writer.write_byte(0xFF).unwrap();
        writer.pad_to_minimum(8);
        assert_eq!(writer.len(), 8);
        assert_eq!(writer.as_bytes()[0], 0xFF);
        assert!(writer.as_bytes()[1..].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_error_on_overflow() {
        let data = vec![0xFF];
        let mut reader = BitStreamReader::new(&data);
        reader.skip_bits(8).unwrap();
        assert!(reader.read_bit().is_err());
    }

    #[test]
    fn test_error_on_invalid_bit_count() {
        let mut writer = BitStreamWriter::new();
        assert!(writer.write_bits(0xFF, 0).is_err());
        assert!(writer.write_bits(0xFF, 33).is_err());
    }
}
