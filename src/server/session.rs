//! Per-client session handler for server
//!
//! Manages individual client connections and state on the server side.

use std::io;
use std::net::SocketAddr;

/// Server-side per-client session
#[derive(Debug, Clone)]
pub struct ClientSession {
    /// Session unique identifier
    pub session_id: u32,
    /// Client socket address
    pub addr: SocketAddr,
    /// Session state
    pub state: crate::connection::ConnectionState,
    /// Player identity bound after auth succeeds
    pub player_id: Option<u64>,
    /// One-time server challenge nonce
    pub auth_nonce: u64,
    /// Session token issued on successful auth
    pub session_token: Option<crate::auth::SessionToken>,
    /// Last accepted client input sequence (replay protection)
    pub last_input_sequence: u32,
    /// Client-specific game state
    pub game_state: Vec<u8>,
}

impl ClientSession {
    /// Creates a new client session
    pub fn new(session_id: u32, addr: SocketAddr, auth_nonce: u64) -> Self {
        ClientSession {
            session_id,
            addr,
            state: crate::connection::ConnectionState::Idle,
            player_id: None,
            auth_nonce,
            session_token: None,
            last_input_sequence: 0,
            game_state: Vec::new(),
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.player_id.is_some() && self.session_token.is_some()
    }

    pub fn mark_authenticated(
        &mut self,
        player_id: u64,
        session_token: crate::auth::SessionToken,
    ) {
        self.player_id = Some(player_id);
        self.session_token = Some(session_token);
        self.activate();
    }

    pub fn accepts_input_sequence(&mut self, input_sequence: u32) -> bool {
        if input_sequence <= self.last_input_sequence {
            return false;
        }
        self.last_input_sequence = input_sequence;
        true
    }

    /// Processes client input packet
    pub fn process_input(&mut self, _data: &[u8]) -> io::Result<()> {
        // TODO: Implement input processing
        Ok(())
    }

    /// Serializes current game state for transmission
    pub fn serialize_state(&self) -> Vec<u8> {
        // TODO: Implement state serialization
        self.game_state.clone()
    }

    /// Updates session game state
    pub fn update_state(&mut self, new_state: Vec<u8>) {
        self.game_state = new_state;
    }

    /// Marks session as active
    pub fn activate(&mut self) {
        self.state = crate::connection::ConnectionState::Active;
    }

    /// Closes the session
    pub fn close(&mut self) {
        self.state = crate::connection::ConnectionState::Closed;
    }
}
