//! Connection management for Tribes protocol
//!
//! Handles connection state machines, per-client sessions, and packet sequencing.

use std::net::SocketAddr;

/// Connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Initial state, waiting for connection request
    Idle,
    /// Connection request received, waiting for acknowledgement
    Connecting,
    /// Connection established and active
    Active,
    /// Connection closing or closed
    Closed,
}

/// Connection information for a client
#[derive(Debug, Clone)]
pub struct Connection {
    /// Connection unique identifier
    pub id: u32,
    /// Client socket address
    pub addr: SocketAddr,
    /// Current connection state
    pub state: ConnectionState,
    /// Last packet sequence number
    pub sequence: u32,
}

impl Connection {
    /// Creates a new connection
    pub fn new(id: u32, addr: SocketAddr) -> Self {
        Connection {
            id,
            addr,
            state: ConnectionState::Idle,
            sequence: 0,
        }
    }

    /// Advances the connection state
    pub fn advance_state(&mut self) {
        self.state = match self.state {
            ConnectionState::Idle => ConnectionState::Connecting,
            ConnectionState::Connecting => ConnectionState::Active,
            ConnectionState::Active => ConnectionState::Active,
            ConnectionState::Closed => ConnectionState::Closed,
        };
    }

    /// Marks connection as closed
    pub fn close(&mut self) {
        self.state = ConnectionState::Closed;
    }

    /// Increments and returns the next sequence number
    pub fn next_sequence(&mut self) -> u32 {
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }
}
