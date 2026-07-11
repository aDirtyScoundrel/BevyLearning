//! Server-side network manager for Tribes protocol
//!
//! Handles connection dispatch, game state management, and broadcasting.

pub mod session;

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct ServerAuthConfig {
    pub auth_secret: String,
    pub token_secret: String,
}

impl Default for ServerAuthConfig {
    fn default() -> Self {
        Self {
            auth_secret: "dev-auth-secret".to_string(),
            token_secret: "dev-token-secret".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ServerIngressPacket<'a> {
    AuthResponse {
        proof: crate::auth::AuthProof,
    },
    ClientInput {
        session_token: crate::auth::SessionToken,
        input_sequence: u32,
        payload: &'a [u8],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerEgressPacket {
    AuthChallenge { nonce: u64 },
    AuthAccepted { session_id: u32, session_token: crate::auth::SessionToken },
    AuthRejected,
    Snapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketDisposition {
    Accepted,
    Rejected,
}

/// Server network manager
pub struct ServerNetworkManager {
    /// Main server socket address (listening)
    pub listen_addr: SocketAddr,
    /// Active per-client sessions
    pub sessions: HashMap<u32, session::ClientSession>,
    /// Reverse lookup for active clients by socket address
    pub session_by_addr: HashMap<SocketAddr, u32>,
    /// Server auth config/secrets
    pub auth: ServerAuthConfig,
}

impl ServerNetworkManager {
    /// Creates a new server network manager
    pub fn new(listen_addr: SocketAddr) -> Self {
        Self::with_auth(listen_addr, ServerAuthConfig::default())
    }

    pub fn with_auth(listen_addr: SocketAddr, auth: ServerAuthConfig) -> Self {
        ServerNetworkManager {
            listen_addr,
            sessions: HashMap::new(),
            session_by_addr: HashMap::new(),
            auth,
        }
    }

    /// Starts listening for incoming connections
    pub fn listen(&mut self) -> io::Result<()> {
        // TODO: Implement socket listening
        Ok(())
    }

    /// Handles incoming connection request
    pub fn handle_connection_request(&mut self, from_addr: SocketAddr) -> io::Result<(u32, ServerEgressPacket)> {
        if let Some(existing_id) = self.session_by_addr.get(&from_addr).copied()
            && let Some(existing) = self.sessions.get(&existing_id)
        {
            return Ok((existing_id, ServerEgressPacket::AuthChallenge { nonce: existing.auth_nonce }));
        }

        let session_id = self.sessions.len() as u32;
        let nonce = Self::fresh_nonce();
        let session = session::ClientSession::new(session_id, from_addr, nonce);

        self.sessions.insert(session_id, session);
        self.session_by_addr.insert(from_addr, session_id);

        Ok((session_id, ServerEgressPacket::AuthChallenge { nonce }))
    }

    /// Broadcasts game state to all connected clients
    pub fn broadcast_state(&self, _state_data: &[u8]) -> io::Result<()> {
        // TODO: Implement state broadcasting
        Ok(())
    }

    /// Processes received packet from client
    pub fn process_client_packet(
        &mut self,
        from_addr: SocketAddr,
        packet: ServerIngressPacket<'_>,
    ) -> io::Result<PacketDisposition> {
        let Some(session_id) = self.session_by_addr.get(&from_addr).copied() else {
            return Ok(PacketDisposition::Rejected);
        };

        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Ok(PacketDisposition::Rejected);
        };

        match packet {
            ServerIngressPacket::AuthResponse { proof } => {
                if proof.nonce != session.auth_nonce {
                    return Ok(PacketDisposition::Rejected);
                }

                if !crate::auth::verify_auth_proof(&self.auth.auth_secret, proof) {
                    return Ok(PacketDisposition::Rejected);
                }

                let token = crate::auth::mint_session_token(
                    &self.auth.token_secret,
                    proof.player_id,
                    proof.nonce,
                );
                session.mark_authenticated(proof.player_id, token);
                Ok(PacketDisposition::Accepted)
            }
            ServerIngressPacket::ClientInput {
                session_token,
                input_sequence,
                payload,
            } => {
                if !session.is_authenticated() {
                    return Ok(PacketDisposition::Rejected);
                }

                if session.session_token != Some(session_token) {
                    return Ok(PacketDisposition::Rejected);
                }

                if !session.accepts_input_sequence(input_sequence) {
                    return Ok(PacketDisposition::Rejected);
                }

                session.process_input(payload)?;
                Ok(PacketDisposition::Accepted)
            }
        }
    }

    /// Removes a client session
    pub fn close_session(&mut self, session_id: u32) {
        if let Some(session) = self.sessions.remove(&session_id) {
            self.session_by_addr.remove(&session.addr);
        }
    }

    fn fresh_nonce() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_rejects_input_before_auth() {
        let mut server = ServerNetworkManager::new("127.0.0.1:35000".parse().unwrap());
        let client_addr: SocketAddr = "127.0.0.1:41001".parse().unwrap();
        let (_session_id, challenge) = server.handle_connection_request(client_addr).unwrap();

        let nonce = match challenge {
            ServerEgressPacket::AuthChallenge { nonce } => nonce,
            _ => panic!("expected challenge"),
        };

        let rejected = server
            .process_client_packet(
                client_addr,
                ServerIngressPacket::ClientInput {
                    session_token: 123,
                    input_sequence: 1,
                    payload: &[1, 2, 3],
                },
            )
            .unwrap();
        assert_eq!(rejected, PacketDisposition::Rejected);

        let proof = crate::auth::make_auth_proof("dev-auth-secret", 42, nonce);
        let accepted = server
            .process_client_packet(client_addr, ServerIngressPacket::AuthResponse { proof })
            .unwrap();
        assert_eq!(accepted, PacketDisposition::Accepted);
    }
}
