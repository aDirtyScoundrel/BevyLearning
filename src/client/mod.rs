//! Client-side network manager for Tribes protocol
//!
//! Handles client connection establishment, state reception, and prediction.

pub mod state_sync;

#[derive(Debug, Default)]
pub struct HandshakeRetry {
    started: Option<std::time::Instant>,
    last_sent: Option<std::time::Instant>,
    finished: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HandshakeAction {
    Wait,
    SendHello,
    TimedOut,
}

impl HandshakeRetry {
    pub fn poll(&mut self, now: std::time::Instant, authenticated: bool) -> HandshakeAction {
        use std::time::Duration;
        if authenticated {
            self.finished = true;
        }
        if self.finished {
            return HandshakeAction::Wait;
        }
        let started = *self.started.get_or_insert(now);
        if now.saturating_duration_since(started) >= Duration::from_secs(10) {
            self.finished = true;
            return HandshakeAction::TimedOut;
        }
        if self.last_sent.is_some_and(|last| now.saturating_duration_since(last) < Duration::from_millis(500)) {
            return HandshakeAction::Wait;
        }
        self.last_sent = Some(now);
        HandshakeAction::SendHello
    }
}

#[cfg(test)]
mod handshake_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn retries_have_a_deadline_and_stop_after_authentication() {
        let start = Instant::now();
        let mut retry = HandshakeRetry::default();
        assert_eq!(retry.poll(start, false), HandshakeAction::SendHello);
        assert_eq!(retry.poll(start + Duration::from_millis(499), false), HandshakeAction::Wait);
        assert_eq!(retry.poll(start + Duration::from_millis(500), false), HandshakeAction::SendHello);
        assert_eq!(retry.poll(start + Duration::from_secs(10), false), HandshakeAction::TimedOut);
        assert_eq!(retry.poll(start + Duration::from_secs(11), false), HandshakeAction::Wait);
        let mut retry = HandshakeRetry::default();
        assert_eq!(retry.poll(start, false), HandshakeAction::SendHello);
        assert_eq!(retry.poll(start + Duration::from_secs(1), true), HandshakeAction::Wait);
        assert_eq!(retry.poll(start + Duration::from_secs(20), true), HandshakeAction::Wait);
    }
}

use std::io;
use std::net::SocketAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientAuthState {
    Unauthenticated,
    AwaitingChallenge,
    AwaitingAcceptance,
    Authenticated,
}

/// Client network manager
pub struct ClientNetworkManager {
    /// Server address to connect to
    pub server_addr: SocketAddr,
    /// Current connection state
    pub state: crate::connection::ConnectionState,
    /// Client trust state with the server.
    pub auth_state: ClientAuthState,
    /// Static player identity for this runtime.
    pub player_id: u64,
    /// Shared secret used to answer auth challenges.
    pub auth_secret: String,
    /// Session token minted by the server once authenticated.
    pub session_token: Option<crate::auth::SessionToken>,
    /// Monotonic sequence for client input packets.
    pub next_input_sequence: u32,
}

impl ClientNetworkManager {
    /// Creates a new client network manager
    pub fn new(server_addr: SocketAddr) -> Self {
        Self::with_identity(server_addr, 0, "dev-auth-secret".to_string())
    }

    pub fn with_identity(server_addr: SocketAddr, player_id: u64, auth_secret: String) -> Self {
        ClientNetworkManager {
            server_addr,
            state: crate::connection::ConnectionState::Idle,
            auth_state: ClientAuthState::Unauthenticated,
            player_id,
            auth_secret,
            session_token: None,
            next_input_sequence: 0,
        }
    }

    /// Initiates connection to server
    pub fn connect(&mut self) -> io::Result<()> {
        // Transport connect remains TODO; we still move auth state forward so
        // higher-level logic can request/expect a challenge.
        self.state = crate::connection::ConnectionState::Connecting;
        self.auth_state = ClientAuthState::AwaitingChallenge;
        Ok(())
    }

    pub fn respond_to_challenge(&mut self, nonce: u64) -> crate::auth::AuthProof {
        self.auth_state = ClientAuthState::AwaitingAcceptance;
        crate::auth::make_auth_proof(&self.auth_secret, self.player_id, nonce)
    }

    pub fn mark_authenticated(&mut self, token: crate::auth::SessionToken) {
        self.state = crate::connection::ConnectionState::Active;
        self.auth_state = ClientAuthState::Authenticated;
        self.session_token = Some(token);
    }

    /// Receives and processes game state update
    pub fn receive_state_update(&mut self, _data: &[u8]) -> io::Result<()> {
        // TODO: Implement state reception and prediction
        Ok(())
    }

    /// Sends player input to server
    pub fn next_input_packet<'a>(&mut self, payload: &'a [u8]) -> io::Result<ClientInputPacket<'a>> {
        if self.auth_state != ClientAuthState::Authenticated {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "client is not authenticated",
            ));
        }

        let Some(session_token) = self.session_token else {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "missing session token",
            ));
        };

        self.next_input_sequence = self.next_input_sequence.wrapping_add(1);

        Ok(ClientInputPacket {
            session_token,
            input_sequence: self.next_input_sequence,
            payload,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ClientInputPacket<'a> {
    pub session_token: crate::auth::SessionToken,
    pub input_sequence: u32,
    pub payload: &'a [u8],
}
