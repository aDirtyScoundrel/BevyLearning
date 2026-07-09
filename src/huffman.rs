//! Huffman codec for Tribes protocol string compression
//!
//! Tribes uses static Huffman compression with frequency tables hardcoded from the original engine.
//! This module implements encoding and decoding matching the t1net-go reference implementation.

use std::io;

const PROB_BOOST: u32 = 1;

/// Huffman tree node (either internal or leaf)
#[derive(Clone, Debug)]
struct HuffNode {
    /// Popularity/frequency
    pop: u32,
    /// Left branch (negative = leaf index, positive = node index)
    index0: i16,
    /// Right branch (negative = leaf index, positive = node index)
    index1: i16,
}

/// Huffman leaf (terminal symbol)
#[derive(Clone, Copy, Debug)]
struct HuffLeaf {
    /// Popularity/frequency
    pop: u32,
    /// Number of bits in the code
    num_bits: u8,
    /// The symbol (0-255)
    symbol: u8,
    /// The Huffman code
    code: u32,
}

/// Wrapper for heap ordering during tree construction
#[derive(Clone)]
struct HuffWrap {
    node_idx: i32,
    leaf_idx: i32,
}

impl HuffWrap {
    fn get_pop(&self, nodes: &[HuffNode], leaves: &[HuffLeaf; 256]) -> u32 {
        if self.node_idx >= 0 {
            nodes[self.node_idx as usize].pop
        } else {
            leaves[self.leaf_idx as usize].pop
        }
    }

    fn determine_index(&self) -> i16 {
        if self.leaf_idx >= 0 {
            -(self.leaf_idx as i16 + 1)
        } else {
            self.node_idx as i16
        }
    }
}

/// Huffman codec using the original Tribes frequency tables
pub struct HuffmanCodec {
    nodes: Vec<HuffNode>,
    leaves: [HuffLeaf; 256],
}

impl HuffmanCodec {
    /// Creates a new Huffman codec with the original Tribes frequency table
    pub fn new() -> Self {
        let mut codec = HuffmanCodec {
            nodes: Vec::new(),
            leaves: [HuffLeaf {
                pop: 0,
                num_bits: 0,
                symbol: 0,
                code: 0,
            }; 256],
        };
        codec.build_tables();
        codec
    }

    /// Builds the Huffman tree and encoding tables
    fn build_tables(&mut self) {
        let freq_table = Self::get_freq_table();

        // Initialize leaves with frequencies
        for i in 0..256 {
            self.leaves[i].symbol = i as u8;
            let mut boost = PROB_BOOST;
            // Boost alphanumeric characters
            if Self::is_alnum(i as u8) {
                boost += PROB_BOOST;
            }
            self.leaves[i].pop = freq_table[i] + boost;
        }

        // Build nodes list, starting with root placeholder
        self.nodes.push(HuffNode {
            pop: 0,
            index0: 0,
            index1: 0,
        });

        let mut wraps: Vec<HuffWrap> = (0..256)
            .map(|i| HuffWrap {
                node_idx: -1,
                leaf_idx: i,
            })
            .collect();

        let mut curr_wraps = 256;

        // Build tree by combining lowest-frequency nodes
        while curr_wraps > 1 {
            let mut min1 = 0xFFFFFFFE;
            let mut min2 = 0xFFFFFFFF;
            let mut idx1 = -1i32;
            let mut idx2 = -1i32;

            for i in 0..curr_wraps {
                let pop = wraps[i].get_pop(&self.nodes, &self.leaves);
                if pop < min1 {
                    min2 = min1;
                    idx2 = idx1;
                    min1 = pop;
                    idx1 = i as i32;
                } else if pop < min2 {
                    min2 = pop;
                    idx2 = i as i32;
                }
            }

            let new_pop = wraps[idx1 as usize].get_pop(&self.nodes, &self.leaves)
                + wraps[idx2 as usize].get_pop(&self.nodes, &self.leaves);

            let idx1_val = wraps[idx1 as usize].determine_index();
            let idx2_val = wraps[idx2 as usize].determine_index();

            self.nodes.push(HuffNode {
                pop: new_pop,
                index0: idx1_val,
                index1: idx2_val,
            });

            let new_node_idx = self.nodes.len() - 1;

            let mut merge_idx = idx1 as usize;
            let mut nuke_idx = idx2 as usize;

            if idx1 > idx2 {
                merge_idx = idx2 as usize;
                nuke_idx = idx1 as usize;
            }

            wraps[merge_idx] = HuffWrap {
                node_idx: new_node_idx as i32,
                leaf_idx: -1,
            };

            if nuke_idx != curr_wraps - 1 {
                wraps[nuke_idx] = wraps[curr_wraps - 1].clone();
            }

            curr_wraps -= 1;
        }

        // Set root node
        if !wraps.is_empty() {
            let root_idx = wraps[0].node_idx as usize;
            self.nodes[0] = self.nodes[root_idx].clone();
        }

        // Generate encoding codes
        self.generate_codes(0, 0, 0);
    }

    /// Recursively generates Huffman codes
    fn generate_codes(&mut self, index: i16, code: u32, depth: u8) {
        if index < 0 {
            // Leaf node
            let leaf_idx = (-(index + 1)) as usize;
            self.leaves[leaf_idx].code = code;
            self.leaves[leaf_idx].num_bits = depth.max(1);
        } else {
            // Internal node
            let node = self.nodes[index as usize].clone();
            self.generate_codes(node.index0, code, depth + 1);
            self.generate_codes(
                node.index1,
                code | (1u32 << depth),
                depth + 1,
            );
        }
    }

    /// Returns whether a byte is alphanumeric
    fn is_alnum(b: u8) -> bool {
        (b >= b'0' && b <= b'9') || (b >= b'A' && b <= b'Z') || (b >= b'a' && b <= b'z')
    }

    /// Returns the original Tribes character frequency table
    /// This is from the t1net-go implementation, based on the original Tribes engine
    #[rustfmt::skip]
    fn get_freq_table() -> [u32; 256] {
        [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 329, 21, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            2809, 68, 0, 27, 0, 58, 3, 62, 4, 7, 0, 0, 15, 65, 554, 3,
            394, 404, 189, 117, 30, 51, 27, 15, 34, 32, 80, 1, 142, 3, 142, 39,
            0, 144, 125, 44, 122, 275, 70, 135, 61, 127, 8, 12, 113, 246, 122, 36,
            185, 1, 149, 309, 335, 12, 11, 14, 54, 151, 0, 0, 2, 0, 0, 211,
            0, 2090, 344, 736, 993, 2872, 701, 605, 646, 1552, 328, 305, 1240, 735, 1533, 1713,
            562, 3, 1775, 1149, 1469, 979, 407, 553, 59, 279, 31, 0, 0, 0, 68, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]
    }

    /// Encodes a string using Huffman compression
    pub fn encode_string(&self, s: &str) -> io::Result<Vec<u8>> {
        let bytes = s.as_bytes();
        let mut writer = crate::bitstream::BitStreamWriter::new();

        // Compression flag: 1 = compressed
        writer.write_bit(true)?;

        // Length (8 bits)
        writer.write_byte(bytes.len().min(255) as u8)?;

        // Huffman-encoded data
        let mut skip32 = false;
        for (i, &byte) in bytes.iter().enumerate() {
            let leaf = &self.leaves[byte as usize];
            writer.write_bits(leaf.code, leaf.num_bits as usize)?;

            // Special case: if first character is >= 128, skip 32 bits for padding
            if i == 0 && byte >= 128 {
                skip32 = true;
            }
        }

        if skip32 {
            writer.write_bits(0, 32)?;
        }

        writer.align();
        Ok(writer.into_bytes())
    }

    /// Decodes a Huffman-encoded string and returns the string plus number of bytes consumed
    pub fn decode_string(&self, data: &[u8]) -> io::Result<(String, usize)> {
        let mut reader = crate::bitstream::BitStreamReader::new(data);

        // Read compression flag
        let compressed = reader.read_bit()?;

        // Read length
        let len = reader.read_byte()? as usize;

        if len == 0 {
            return Ok((String::new(), 1));
        }

        if compressed {
            let mut buf = Vec::with_capacity(len);
            let mut skip32 = false;

            for i in 0..len {
                let mut index: i16 = 0;

                // Traverse tree to find symbol
                loop {
                    if index < 0 {
                        // Reached a leaf
                        let leaf_idx = (-(index + 1)) as usize;
                        let sym = self.leaves[leaf_idx].symbol;
                        buf.push(sym);

                        if i == 0 && sym >= 128 {
                            skip32 = true;
                        }
                        break;
                    }

                    // Internal node - read bit and traverse
                    let bit = reader.read_bit()?;
                    let node = &self.nodes[index as usize];
                    index = if bit { node.index1 } else { node.index0 };
                }
            }

            if skip32 {
                reader.skip_bits(32)?;
            }

            let s = String::from_utf8(buf).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("Invalid UTF-8: {}", e))
            })?;

            let consumed = reader.byte_position();
            Ok((s, consumed))
        } else {
            // Uncompressed: read 8 bits per character
            let mut buf = Vec::with_capacity(len);
            for _ in 0..len {
                buf.push(reader.read_byte()?);
            }
            let s = String::from_utf8(buf).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("Invalid UTF-8: {}", e))
            })?;
            let consumed = reader.byte_position();
            Ok((s, consumed))
        }
    }
}

impl Default for HuffmanCodec {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_huffman_codec_creation() {
        let codec = HuffmanCodec::new();
        assert!(!codec.leaves.is_empty());
        assert!(!codec.nodes.is_empty());
    }

    #[test]
    fn test_encode_decode_string() {
        let codec = HuffmanCodec::new();

        let test_strings = vec![
            "Tribes",
            "Server",
            "Player",
            "Hello World",
            "test123",
        ];

        for s in test_strings {
            let encoded = codec.encode_string(s).unwrap();
            let (decoded, _) = codec.decode_string(&encoded).unwrap();
            assert_eq!(decoded, s, "Failed for string: {}", s);
        }
    }

    #[test]
    fn test_empty_string() {
        let codec = HuffmanCodec::new();
        let encoded = codec.encode_string("").unwrap();
        let (decoded, _) = codec.decode_string(&encoded).unwrap();
        assert_eq!(decoded, "");
    }

    #[test]
    fn test_single_char() {
        let codec = HuffmanCodec::new();
        let encoded = codec.encode_string("a").unwrap();
        let (decoded, _) = codec.decode_string(&encoded).unwrap();
        assert_eq!(decoded, "a");
    }

    #[test]
    fn test_numbers() {
        let codec = HuffmanCodec::new();
        let s = "12345";
        let encoded = codec.encode_string(s).unwrap();
        let (decoded, _) = codec.decode_string(&encoded).unwrap();
        assert_eq!(decoded, s);
    }
}
