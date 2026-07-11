//! Tribes Networking - Rust port of the Tribes game networking protocol
//!
//! This library implements the UDP-based networking protocol used in Starsiege: Tribes (1998+)
//! optimized for fast-paced multiplayer games with bandwidth efficiency through compression
//! and bit-level packing.

pub mod bitstream;
pub mod huffman;
pub mod auth;
pub mod sync_codec;
pub mod protocol;
pub mod connection;
pub mod packet;
pub mod client;
pub mod server;

pub use bitstream::{BitStreamReader, BitStreamWriter};
pub use huffman::HuffmanCodec;
