//! Server-side network manager for Tribes protocol
//!
//! Handles connection dispatch, game state management, and broadcasting.

pub mod session;

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;

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
    /// Monotonic counter for session IDs — avoids collisions after removal
    /// that a `sessions.len()`-based scheme would produce.
    next_session_id: u32,
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
            next_session_id: 0,
        }
    }

    /// Starts listening for incoming connections
    pub fn listen(&mut self) -> io::Result<()> {
        // TODO: Implement socket listening
        Ok(())
    }

    /// Handles incoming connection request. The `player_id` comes from the
    /// hello packet and binds the session identity; later proofs must match it.
    pub fn handle_connection_request(&mut self, from_addr: SocketAddr, player_id: u64) -> io::Result<(u32, ServerEgressPacket)> {
        if let Some(existing_id) = self.session_by_addr.get(&from_addr).copied()
            && let Some(existing) = self.sessions.get_mut(&existing_id)
        {
            existing.touch();
            // Rebind the hello identity only while unauthenticated. An
            // authenticated session keeps its bound identity, so a stray or
            // replayed hello cannot redirect its inputs to another player slot.
            if !existing.is_authenticated() {
                existing.player_id = Some(player_id);
            }
            return Ok((existing_id, ServerEgressPacket::AuthChallenge { nonce: existing.auth_nonce }));
        }

        let session_id = self.next_session_id;
        self.next_session_id = self.next_session_id.wrapping_add(1);
        let nonce = Self::fresh_nonce();
        let mut session = session::ClientSession::new(session_id, from_addr, nonce);
        session.player_id = Some(player_id);

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

                // Bind the proof identity to the hello identity. Possessing
                // the shared secret alone must not let a client claim another
                // player's authoritative slot. The Steam host enforces the
                // same check in its proof path.
                if session.player_id != Some(proof.player_id) {
                    return Ok(PacketDisposition::Rejected);
                }

                // Re-authentication is idempotent. A retried handshake after
                // a lost accept re-sends the existing token instead of
                // minting a new one; re-minting would silently change the
                // server-side token while the client keeps the old one,
                // locking the session out of every later input check.
                if session.is_authenticated() {
                    session.touch();
                    return Ok(PacketDisposition::Accepted);
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

                session.touch();
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

    /// Evicts sessions that haven't sent a valid packet within `timeout`
    /// and returns the player ids bound to the evicted sessions. Callers
    /// must drop each returned player's simulation state (inputs,
    /// velocity, remote state); otherwise the shared authoritative sim
    /// keeps refreshing that player's `last_seen` and its ghost never
    /// expires. Without eviction, connect/disconnect churn (or repeated
    /// spoofed hellos — this is UDP with no return-routability check)
    /// grows `sessions` and `session_by_addr` forever, since
    /// `close_session` had no caller.
    pub fn sweep_expired_sessions(&mut self, timeout: std::time::Duration) -> Vec<u64> {
        let now = std::time::Instant::now();
        let expired: Vec<u32> = self
            .sessions
            .iter()
            .filter(|(_, session)| now.duration_since(session.last_activity) > timeout)
            .map(|(id, _)| *id)
            .collect();
        let mut evicted_players = Vec::new();
        for session_id in expired {
            if let Some(player_id) = self
                .sessions
                .get(&session_id)
                .and_then(|session| session.player_id)
            {
                evicted_players.push(player_id);
            }
            self.close_session(session_id);
        }
        evicted_players
    }

    fn fresh_nonce() -> u64 {
        crate::auth::fresh_nonce()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_rejects_input_before_auth() {
        let mut server = ServerNetworkManager::new("127.0.0.1:35000".parse().unwrap());
        let client_addr: SocketAddr = "127.0.0.1:41001".parse().unwrap();
        let (_session_id, challenge) = server.handle_connection_request(client_addr, 42).unwrap();

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

    fn authed_server_with_nonce(
        port: u16,
        client_port: u16,
        player_id: u64,
    ) -> (ServerNetworkManager, SocketAddr, u64) {
        let mut server =
            ServerNetworkManager::new(format!("127.0.0.1:{port}").parse().unwrap());
        let client_addr: SocketAddr = format!("127.0.0.1:{client_port}").parse().unwrap();
        let (_session_id, challenge) = server
            .handle_connection_request(client_addr, player_id)
            .unwrap();
        let nonce = match challenge {
            ServerEgressPacket::AuthChallenge { nonce } => nonce,
            _ => panic!("expected challenge"),
        };
        (server, client_addr, nonce)
    }

    #[test]
    fn sweep_evicts_silent_sessions_and_reports_player_ids() {
        let (mut server, client_addr, nonce) = authed_server_with_nonce(35013, 41013, 42);
        let proof = crate::auth::make_auth_proof("dev-auth-secret", 42, nonce);
        assert_eq!(
            server
                .process_client_packet(client_addr, ServerIngressPacket::AuthResponse { proof })
                .unwrap(),
            PacketDisposition::Accepted
        );

        // A fresh session survives the sweep.
        assert!(server
            .sweep_expired_sessions(std::time::Duration::from_secs(30))
            .is_empty());

        // Backdate past the timeout: the session is evicted and its bound
        // player id is reported so the caller can drop the sim state.
        let session_id = server.session_by_addr[&client_addr];
        server.sessions.get_mut(&session_id).unwrap().last_activity =
            std::time::Instant::now() - std::time::Duration::from_secs(60);
        let evicted = server.sweep_expired_sessions(std::time::Duration::from_secs(30));
        assert_eq!(evicted, vec![42]);
        assert!(server.sessions.is_empty());
        assert!(server.session_by_addr.is_empty());
    }

    #[test]
    fn reauth_after_lost_accept_keeps_existing_token() {
        let (mut server, client_addr, nonce) = authed_server_with_nonce(35010, 41010, 42);

        let proof = crate::auth::make_auth_proof("dev-auth-secret", 42, nonce);
        assert_eq!(
            server
                .process_client_packet(client_addr, ServerIngressPacket::AuthResponse { proof })
                .unwrap(),
            PacketDisposition::Accepted
        );
        let session_id = server.session_by_addr[&client_addr];
        let first_token = server.sessions[&session_id]
            .session_token
            .expect("token after auth");

        // A lost accept triggers a retried handshake: the same proof arrives
        // again. The server must re-send the existing token, not re-mint,
        // or the client's stored token fails every later input check.
        assert_eq!(
            server
                .process_client_packet(client_addr, ServerIngressPacket::AuthResponse { proof })
                .unwrap(),
            PacketDisposition::Accepted
        );
        assert_eq!(
            server.sessions[&session_id].session_token,
            Some(first_token),
            "re-auth must keep the existing token"
        );

        // The original token still passes the input check.
        assert_eq!(
            server
                .process_client_packet(
                    client_addr,
                    ServerIngressPacket::ClientInput {
                        session_token: first_token,
                        input_sequence: 1,
                        payload: &[],
                    },
                )
                .unwrap(),
            PacketDisposition::Accepted
        );
    }

    #[test]
    fn proof_with_foreign_player_id_is_rejected() {
        let (mut server, client_addr, nonce) = authed_server_with_nonce(35011, 41011, 42);

        // A valid shared-secret holder claims a different player id than
        // its hello. The proof HMAC verifies, but the binding must reject.
        let hijack = crate::auth::make_auth_proof("dev-auth-secret", 43, nonce);
        assert_eq!(
            server
                .process_client_packet(client_addr, ServerIngressPacket::AuthResponse { proof: hijack })
                .unwrap(),
            PacketDisposition::Rejected
        );

        // The matching identity still authenticates.
        let proof = crate::auth::make_auth_proof("dev-auth-secret", 42, nonce);
        assert_eq!(
            server
                .process_client_packet(client_addr, ServerIngressPacket::AuthResponse { proof })
                .unwrap(),
            PacketDisposition::Accepted
        );
    }

    #[test]
    fn stray_hello_after_auth_does_not_rebind_identity() {
        let (mut server, client_addr, nonce) = authed_server_with_nonce(35012, 41012, 42);
        let proof = crate::auth::make_auth_proof("dev-auth-secret", 42, nonce);
        assert_eq!(
            server
                .process_client_packet(client_addr, ServerIngressPacket::AuthResponse { proof })
                .unwrap(),
            PacketDisposition::Accepted
        );

        // A replayed hello from the same address claiming id 43 must not
        // redirect the authenticated session's inputs to another player slot.
        server
            .handle_connection_request(client_addr, 43)
            .unwrap();
        let session_id = server.session_by_addr[&client_addr];
        assert_eq!(server.sessions[&session_id].player_id, Some(42));

        // The foreign identity still cannot prove against the bound session.
        let hijack = crate::auth::make_auth_proof("dev-auth-secret", 43, nonce);
        assert_eq!(
            server
                .process_client_packet(client_addr, ServerIngressPacket::AuthResponse { proof: hijack })
                .unwrap(),
            PacketDisposition::Rejected
        );
    }
}
