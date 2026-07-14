//! Steamworks-based remote cube sync.
//!
//! This supplements LAN broadcast sync by allowing direct P2P transform packets
//! between Steam users for remote multiplayer testing.

use bevy::prelude::*;

#[cfg(feature = "steamworks")]
mod imp {
    use super::*;
    use bevy::math::primitives::Sphere;
    use bevy::mesh::Mesh3d;
    use bevy::pbr::{MeshMaterial3d, StandardMaterial};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::collections::{HashMap, HashSet};
    use std::time::{Duration, Instant};

    // Packet framing: every packet starts with a 4-byte magic tag and a 1-byte version.
    const STEAM_SYNC_MAGIC: [u8; 4] = *b"STMC";
    const STEAM_SYNC_VERSION: u8 = 1;

    // Packet type discriminators used in the second header byte.
    const PACKET_STATE: u8 = 1;       // periodic position/color update
    const PACKET_JOIN: u8 = 2;        // initial announce on connection
    const PACKET_LEAVE: u8 = 3;       // graceful disconnect notice
    const PACKET_FREEZE: u8 = 4;      // freeze a target player briefly
    const PACKET_PROJECTILE: u8 = 5;  // replicate a projectile spawn

    // Tick rate for outbound state packets (~20 Hz).
    const SEND_INTERVAL: Duration = Duration::from_millis(50);
    // Drop a remote peer's state after this much silence.
    const REMOTE_TIMEOUT: Duration = Duration::from_secs(5);
    // How long to remember projectile IDs for deduplication.
    const PROJECTILE_DEDUP_TTL: Duration = Duration::from_secs(15);

    const FREEZE_DURATION_SECS: f32 = 2.0;
    const METRICS_OVERLAY_TOGGLE_KEY: KeyCode = KeyCode::F5;

    #[derive(Component, Debug, Clone, Copy)]
    pub struct SteamRemoteCube {
        pub player_id: u64,
    }

    #[derive(Debug, Clone)]
    struct RemoteState {
        transform: Transform,
        color: Color,
        last_seen: Instant,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PresenceState {
        Pending,
        Announced,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum RuntimeSteamRole {
        LegacyPeer,
        AuthHost,
        UntrustedClient,
    }

    #[derive(Debug, Clone, Copy)]
    struct SteamInputSample {
        move_x: f32,
        move_z: f32,
        jump: bool,
        color: Color,
    }

    #[derive(Debug, Default, Clone, Copy)]
    struct SteamNetMetrics {
        client_auth_peer_mismatch_drops: u64,
        client_auth_challenges_received: u64,
        client_auth_proofs_sent: u64,
        client_auth_accepts_received: u64,
        client_input_packets_sent: u64,
        host_auth_hello_received: u64,
        host_auth_challenges_sent: u64,
        host_auth_proofs_received: u64,
        host_auth_accepts_sent: u64,
        host_auth_rejects: u64,
        host_input_packets_received: u64,
        host_input_packets_accepted: u64,
        host_token_rejects: u64,
        host_peer_mismatch_drops: u64,
        host_replay_drops: u64,
        host_payload_decode_rejects: u64,
    }

    impl Default for SteamInputSample {
        fn default() -> Self {
            Self {
                move_x: 0.0,
                move_z: 0.0,
                jump: false,
                color: Color::srgb(0.96, 0.94, 0.88),
            }
        }
    }

    #[derive(Debug, Clone)]
    struct SteamAuthSession {
        player_id: u64,
        nonce: u64,
        token: Option<learning::auth::SessionToken>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum SteamClientAuthState {
        Unauthenticated,
        AwaitingChallenge,
        Authenticated,
    }

    #[derive(Resource)]
    pub struct SteamSync {
        // ── Core client ──────────────────────────────────────────────────────────
        pub client: steamworks::Client,

        // ── Network topology ─────────────────────────────────────────────────────
        /// Peers we broadcast/send state to. Updated from the joined lobby each tick.
        pub targets: Vec<steamworks::SteamId>,
        /// The authoritative host we send input to (UntrustedClient role only).
        auth_host: Option<steamworks::SteamId>,
        pub last_send: Instant,
        presence_state: PresenceState,
        role: RuntimeSteamRole,

        // ── Auth – shared ─────────────────────────────────────────────────────────
        /// HMAC secret shared out-of-band; read from `STEAM_AUTH_SECRET` env var.
        auth_secret: String,

        // ── Auth – client side ───────────────────────────────────────────────────
        client_auth_state: SteamClientAuthState,
        /// Minted by the host after a successful proof exchange; carried in every input packet.
        client_session_token: Option<learning::auth::SessionToken>,
        /// Monotonically increasing counter used to reject replayed input packets.
        client_input_sequence: u32,

        // ── Auth – host side ─────────────────────────────────────────────────────
        /// Per-peer auth handshake state (challenge nonce, player id, minted token).
        auth_sessions: HashMap<steamworks::SteamId, SteamAuthSession>,
        /// Reverse map from session token → player Steam64 ID.
        token_to_player: HashMap<learning::auth::SessionToken, u64>,
        /// Locks a player's ingress to the first peer that sent a valid token.
        player_ingress_peer: HashMap<u64, steamworks::SteamId>,
        /// Last accepted input sequence per player; used to drop replays.
        last_input_sequence_by_player: HashMap<u64, u32>,

        // ── Diagnostics ──────────────────────────────────────────────────────────
        metrics: SteamNetMetrics,
        last_metrics_log: Instant,

        // ── Authoritative sim (host only) ────────────────────────────────────────
        /// Latest validated input sample per remote player.
        authoritative_inputs: HashMap<u64, SteamInputSample>,
        /// Vertical velocity for each remote player's simulated cube.
        authoritative_vertical_velocity: HashMap<u64, f32>,

        // ── Client reconciliation ────────────────────────────────────────────────
        /// Authoritative transform snapshot pushed by the host; lerped to each frame.
        pending_local_reconciliation: Option<Transform>,

        // ── Lobby / server browser ───────────────────────────────────────────────
        hosted_lobby: Option<steamworks::LobbyId>,
        joined_lobby: Option<steamworks::LobbyId>,
        browser_entries: Vec<BrowserEntry>,
        browser_selected: usize,
        browser_status: String,
        /// Channel for lobby list / join results from background Steamworks callbacks.
        browser_mailbox: Arc<Mutex<Vec<BrowserMessage>>>,
        browser_refresh_in_flight: bool,

        // ── Remote entity state ───────────────────────────────────────────────────
        /// Latest known position/color per remote Steam64 ID.
        remote_states: HashMap<u64, RemoteState>,
        /// Bevy entity spawned for each remote player.
        spawned_entities: HashMap<u64, Entity>,
        /// Players that sent a LEAVE packet; cleaned up on the next sync pass.
        departed_players: HashSet<u64>,
        /// Freeze targets queued from incoming packets; drained each frame.
        pending_freezes: Vec<u64>,
        /// Projectile spawns queued from incoming packets; drained each frame.
        pending_projectiles: Vec<crate::scene::ProjectileSpawnData>,
        /// Recently seen (player_id, projectile_id) pairs used for deduplication.
        seen_projectiles: HashMap<(u64, u32), Instant>,
    }

    #[derive(Debug, Clone)]
    struct BrowserEntry {
        lobby: steamworks::LobbyId,
        owner: steamworks::SteamId,
        name: String,
        members: usize,
        max_members: usize,
    }

    #[derive(Debug, Clone)]
    enum BrowserMessage {
        LobbyList(Result<Vec<steamworks::LobbyId>, String>),
        JoinResult(Result<steamworks::LobbyId, String>),
        HostLobbyCreated(Result<steamworks::LobbyId, String>),
    }

    #[derive(Resource, Default)]
    pub struct SteamBrowserView {
        pub status: String,
        pub rows: Vec<String>,
        pub selected_index: Option<usize>,
    }

    #[derive(Resource)]
    pub struct SteamMetricsOverlayState {
        pub visible: bool,
    }

    #[derive(Component)]
    pub struct SteamMetricsOverlayRoot;

    #[derive(Component)]
    pub struct SteamMetricsOverlayText;

    /// Broadcast `payload` to all current targets using unreliable send.
    fn send_payload(steam: &SteamSync, payload: &[u8]) {
        if steam.targets.is_empty() {
            return;
        }

        let networking = steam.client.networking();
        for target in &steam.targets {
            networking.accept_p2p_session(*target);
            let _ = networking.send_p2p_packet(
                *target,
                steamworks::SendType::UnreliableNoDelay,
                payload,
            );
        }
    }

    /// Route a raw inbound P2P packet to the appropriate role-specific handler.
    ///
    /// AuthHost and UntrustedClient have dedicated paths that handle the
    /// challenge/proof/token handshake and authoritative inputs.  LegacyPeer
    /// falls through to direct state/projectile/freeze decoding.
    fn process_incoming_packet(
        steam: &mut SteamSync,
        remote: steamworks::SteamId,
        local_id: u64,
        data: &[u8],
    ) {
        match steam.role {
            RuntimeSteamRole::AuthHost => {
                process_host_packet(steam, remote, data);
                return;
            }
            RuntimeSteamRole::UntrustedClient => {
                process_client_packet(steam, remote, local_id, data);
                return;
            }
            // LegacyPeer: fall through to direct packet decoding below.
            RuntimeSteamRole::LegacyPeer => {}
        }

        if let Some((packet_type, player_id, transform, color)) = decode_packet(data) {
            if player_id == local_id {
                return;
            }

            match packet_type {
                PACKET_STATE | PACKET_JOIN => {
                    if let (Some(transform), Some(color)) = (transform, color) {
                        steam.remote_states.insert(
                            player_id,
                            RemoteState {
                                transform,
                                color,
                                last_seen: Instant::now(),
                            },
                        );
                    }
                }
                PACKET_LEAVE => {
                    steam.departed_players.insert(player_id);
                }
                _ => {}
            }
            return;
        }

        if let Some((player_id, spawn)) = decode_projectile_packet(data) {
            if player_id != local_id
                && crate::sync_codec::accept_recent_projectile(
                    &mut steam.seen_projectiles,
                    player_id,
                    spawn.projectile_id,
                    Instant::now(),
                    PROJECTILE_DEDUP_TTL,
                )
            {
                steam.pending_projectiles.push(spawn);
            }
            return;
        }

        if let Some((_sender_id, target_id)) = decode_freeze_packet(data) {
            steam.pending_freezes.push(target_id);
        }
    }

    /// Initialise the Steamworks client and insert [`SteamSync`] as a Bevy resource.
    ///
    /// Role is determined at startup from environment variables:
    /// - `STEAM_AUTH_HOST=1`      → AuthHost (runs the authoritative sim)
    /// - `STEAM_AUTH_HOST_ID=<id>`→ UntrustedClient (sends inputs, trusts snapshots)
    /// - neither                  → LegacyPeer (direct transform broadcast, no auth)
    ///
    /// Does nothing if the Steam API fails to initialise.
    pub fn setup_steam_sync(mut commands: Commands) {
        let role = if std::env::var("STEAM_AUTH_HOST")
            .ok()
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        {
            RuntimeSteamRole::AuthHost
        } else if std::env::var("STEAM_AUTH_HOST_ID").is_ok() {
            RuntimeSteamRole::UntrustedClient
        } else {
            RuntimeSteamRole::LegacyPeer
        };

        let targets = std::env::var("STEAM_REMOTE_IDS")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .filter_map(|s| s.trim().parse::<u64>().ok())
                    .map(steamworks::SteamId::from_raw)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let auth_host = std::env::var("STEAM_AUTH_HOST_ID")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(steamworks::SteamId::from_raw);

        let Ok((client, single)) = steamworks::Client::init() else {
            eprintln!("[steam-mp] Steam API init failed; Steam transport disabled");
            return;
        };

        thread::spawn(move || {
            loop {
                single.run_callbacks();
                thread::sleep(Duration::from_millis(16));
            }
        });

        let mailbox: Arc<Mutex<Vec<BrowserMessage>>> = Arc::new(Mutex::new(Vec::new()));

        let my_id = client.user().steam_id();
        println!("[steam-mp] local steam id: {}", my_id.raw());

        if targets.is_empty() {
            println!(
                "[steam-mp] no STEAM_REMOTE_IDS configured; set comma-separated Steam64 IDs to enable P2P sync"
            );
        } else {
            let target_list = targets
                .iter()
                .map(|id| id.raw().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            println!("[steam-mp] targets: {}", target_list);
        }

        if role == RuntimeSteamRole::AuthHost {
            let mm = client.matchmaking();
            let mailbox = mailbox.clone();
            mm.create_lobby(steamworks::LobbyType::Public, 16, move |res| {
                let msg = match res {
                    Ok(lobby) => BrowserMessage::HostLobbyCreated(Ok(lobby)),
                    Err(err) => BrowserMessage::HostLobbyCreated(Err(format!("{err:?}"))),
                };
                if let Ok(mut queue) = mailbox.lock() {
                    queue.push(msg);
                }
            });
        }

        commands.insert_resource(SteamSync {
            client,
            targets,
            auth_host,
            last_send: Instant::now(),
            presence_state: PresenceState::Pending,
            role,
            auth_secret: std::env::var("STEAM_AUTH_SECRET")
                .unwrap_or_else(|_| "dev-auth-secret".to_string()),
            client_auth_state: SteamClientAuthState::Unauthenticated,
            client_session_token: None,
            client_input_sequence: 0,
            auth_sessions: HashMap::new(),
            token_to_player: HashMap::new(),
            player_ingress_peer: HashMap::new(),
            last_input_sequence_by_player: HashMap::new(),
            metrics: SteamNetMetrics::default(),
            last_metrics_log: Instant::now(),
            authoritative_inputs: HashMap::new(),
            authoritative_vertical_velocity: HashMap::new(),
            pending_local_reconciliation: None,
            hosted_lobby: None,
            joined_lobby: None,
            browser_entries: Vec::new(),
            browser_selected: 0,
            browser_status: "Steam browser ready. F6 refresh, Up/Down select, F7 join.".to_string(),
            browser_mailbox: mailbox.clone(),
            browser_refresh_in_flight: false,
            remote_states: HashMap::new(),
            spawned_entities: HashMap::new(),
            departed_players: HashSet::new(),
            pending_freezes: Vec::new(),
            pending_projectiles: Vec::new(),
            seen_projectiles: HashMap::new(),
        });

        commands.insert_resource(SteamBrowserView {
            status: "Steam browser ready. F6 refresh, Up/Down select, F7 join.".to_string(),
            rows: Vec::new(),
            selected_index: None,
        });
    }

    pub fn setup_steam_metrics_overlay(mut commands: Commands) {
        commands.insert_resource(SteamMetricsOverlayState { visible: false });

        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(20.0),
                    top: Val::Px(20.0),
                    max_width: Val::Px(460.0),
                    padding: UiRect::all(Val::Px(10.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(10.0)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.03, 0.05, 0.08, 0.9)),
                BorderColor::all(Color::srgba(0.72, 0.84, 0.96, 0.24)),
                SteamMetricsOverlayRoot,
            ))
            .with_children(|parent| {
                parent.spawn((
                    Text::new("Steam metrics hidden. Press F5 to toggle."),
                    TextFont::from_font_size(11.0),
                    TextColor(Color::srgba(0.88, 0.94, 0.99, 0.95)),
                    SteamMetricsOverlayText,
                ));
            });
    }

    pub fn send_freeze_target(steam: &mut SteamSync, sender_id: u64, target_id: u64) {
        let payload = encode_freeze_packet(sender_id, target_id);
        send_payload(steam, &payload);
    }

    pub fn send_projectile_spawn(
        steam: &mut SteamSync,
        sender_id: u64,
        spawn: &crate::scene::ProjectileSpawnData,
    ) {
        let payload = encode_projectile_packet(sender_id, spawn);
        send_payload(steam, &payload);
    }

    /// Auto-refresh the Steam server browser on startup so servers are visible immediately.
    pub fn auto_refresh_browser_on_startup(mut steam: Option<ResMut<SteamSync>>) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };
        request_browser_refresh(steam);
    }

    pub fn apply_local_freeze(
        _local_player: Res<crate::multiplayer::LocalPlayerId>,
        mut freeze: ResMut<crate::controls::MovementFreeze>,
        mut steam: Option<ResMut<SteamSync>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        let local_steam_id = steam.client.user().steam_id().raw();

        let should_freeze = steam
            .pending_freezes
            .drain(..)
            .any(|target_id| target_id == local_steam_id);

        if should_freeze {
            freeze.activate_for(FREEZE_DURATION_SECS);
        }
    }

    pub fn announce_local_presence(
        local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
        hud: Res<crate::ui::HudState>,
        mut steam: Option<ResMut<SteamSync>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        if steam.presence_state == PresenceState::Announced || steam.targets.is_empty() {
            return;
        }

        let local_id = steam.client.user().steam_id().raw();
        match steam.role {
            RuntimeSteamRole::LegacyPeer => {
                let Ok(transform) = local_cube_query.single() else {
                    return;
                };
                let payload = encode_packet(PACKET_JOIN, local_id, transform, hud.selected_color());
                send_payload(steam, &payload);
            }
            RuntimeSteamRole::UntrustedClient => {
                let payload = encode_auth_hello(local_id);
                send_auth_payload_to_host(steam, &payload);
                steam.client_auth_state = SteamClientAuthState::AwaitingChallenge;
            }
            RuntimeSteamRole::AuthHost => {}
        }

        steam.presence_state = PresenceState::Announced;
    }

    pub fn send_local_leave(
        exit_requested: Res<crate::ExitRequested>,
        local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
        mut steam: Option<ResMut<SteamSync>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        if !exit_requested.0 {
            return;
        }

        if steam.targets.is_empty() {
            return;
        }

        let Ok(local_transform) = local_cube_query.single() else {
            return;
        };

        let local_id = steam.client.user().steam_id().raw();
        let payload = encode_packet(PACKET_LEAVE, local_id, local_transform, Color::WHITE);
        send_payload(steam, &payload);
    }

    pub fn process_callbacks(
        mut steam: Option<ResMut<SteamSync>>,
        mut browser_view: Option<ResMut<SteamBrowserView>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        process_browser_messages(steam);
        sync_targets_from_joined_lobby(steam);
        log_metrics_if_due(steam);
        rebuild_browser_rows(steam, browser_view.as_deref_mut());
    }

    pub fn update_steam_metrics_overlay(
        keyboard: Res<ButtonInput<KeyCode>>,
        steam: Option<Res<SteamSync>>,
        mut overlay_state: Option<ResMut<SteamMetricsOverlayState>>,
        mut root_query: Query<&mut Node, With<SteamMetricsOverlayRoot>>,
        mut text_query: Query<&mut Text, With<SteamMetricsOverlayText>>,
    ) {
        let Some(overlay_state) = overlay_state.as_deref_mut() else {
            return;
        };

        if keyboard.just_pressed(METRICS_OVERLAY_TOGGLE_KEY) {
            overlay_state.visible = !overlay_state.visible;
        }

        if let Ok(mut root_node) = root_query.single_mut() {
            root_node.display = if overlay_state.visible {
                Display::Flex
            } else {
                Display::None
            };
        }

        if !overlay_state.visible {
            return;
        }

        if let Ok(mut text) = text_query.single_mut() {
            text.0 = match steam.as_deref() {
                Some(steam) => format_metrics_overlay_text(steam),
                None => "Steam transport not active. Build with --features steamworks and run Steam client."
                    .to_string(),
            };
        }
    }

    pub fn update_server_browser_controls(
        keyboard: Res<ButtonInput<KeyCode>>,
        mut steam: Option<ResMut<SteamSync>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        if keyboard.just_pressed(KeyCode::F6) {
            request_browser_refresh(steam);
        }

        if keyboard.just_pressed(KeyCode::ArrowDown) && !steam.browser_entries.is_empty() {
            steam.browser_selected = (steam.browser_selected + 1).min(steam.browser_entries.len() - 1);
        }

        if keyboard.just_pressed(KeyCode::ArrowUp) && !steam.browser_entries.is_empty() {
            steam.browser_selected = steam.browser_selected.saturating_sub(1);
        }

        if keyboard.just_pressed(KeyCode::F7)
            && let Some(entry) = steam.browser_entries.get(steam.browser_selected).cloned()
        {
            let mailbox = steam.browser_mailbox.clone();
            steam.client.matchmaking().join_lobby(entry.lobby, move |res| {
                let msg = match res {
                    Ok(lobby) => BrowserMessage::JoinResult(Ok(lobby)),
                    Err(_) => BrowserMessage::JoinResult(Err("join failed".to_string())),
                };
                if let Ok(mut queue) = mailbox.lock() {
                    queue.push(msg);
                }
            });
        }
    }

    fn request_browser_refresh(steam: &mut SteamSync) {
        if steam.browser_refresh_in_flight {
            return;
        }

        steam.browser_refresh_in_flight = true;
        let mailbox = steam.browser_mailbox.clone();
        let mm = steam.client.matchmaking();
        mm.set_request_lobby_list_result_count_filter(32);
        mm.request_lobby_list(move |res| {
            let msg = match res {
                Ok(lobbies) => BrowserMessage::LobbyList(Ok(lobbies)),
                Err(err) => BrowserMessage::LobbyList(Err(format!("{err:?}"))),
            };
            if let Ok(mut queue) = mailbox.lock() {
                queue.push(msg);
            }
        });
    }

    fn process_browser_messages(steam: &mut SteamSync) {
        let mut drained = Vec::new();
        if let Ok(mut queue) = steam.browser_mailbox.lock() {
            drained.append(&mut *queue);
        }

        if drained.is_empty() {
            return;
        }

        for msg in drained {
            match msg {
                BrowserMessage::LobbyList(result) => {
                    steam.browser_refresh_in_flight = false;
                    match result {
                        Ok(lobbies) => {
                            steam.browser_entries.clear();
                            let mm = steam.client.matchmaking();
                            for lobby in lobbies {
                                let owner = mm.lobby_owner(lobby);
                                let members = mm.lobby_member_count(lobby);
                                let max_members = mm.lobby_member_limit(lobby).unwrap_or(0);
                                let name = mm
                                    .lobby_data(lobby, "server_name")
                                    .map(ToString::to_string)
                                    .unwrap_or_else(|| format!("Lobby {}", lobby.raw()));
                                steam.browser_entries.push(BrowserEntry {
                                    lobby,
                                    owner,
                                    name,
                                    members,
                                    max_members,
                                });
                            }
                            steam.browser_selected = steam
                                .browser_selected
                                .min(steam.browser_entries.len().saturating_sub(1));
                            steam.browser_status = format!(
                                "Found {} server(s). F7 join selected.",
                                steam.browser_entries.len()
                            );
                        }
                        Err(err) => {
                            steam.browser_status = format!("Refresh failed: {err}");
                        }
                    }
                }
                BrowserMessage::JoinResult(result) => match result {
                    Ok(lobby) => {
                        steam.joined_lobby = Some(lobby);
                        steam.browser_status = format!("Joined lobby {}", lobby.raw());
                    }
                    Err(err) => {
                        steam.browser_status = format!("Join failed: {err}");
                    }
                },
                BrowserMessage::HostLobbyCreated(result) => match result {
                    Ok(lobby) => {
                        steam.hosted_lobby = Some(lobby);
                        steam.joined_lobby = Some(lobby);
                        let mm = steam.client.matchmaking();
                        let my_id = steam.client.user().steam_id().raw();
                        let _ = mm.set_lobby_data(lobby, "server_name", &format!("Learning Host {my_id}"));
                        let _ = mm.set_lobby_data(lobby, "game", "learning");
                        let _ = mm.set_lobby_joinable(lobby, true);
                    }
                    Err(_err) => {
                        eprintln!("[steam-mp] failed to create host lobby: {_err:?}");
                    }
                },
            }
        }
    }

    fn rebuild_browser_rows(steam: &SteamSync, browser_view: Option<&mut SteamBrowserView>) {
        let Some(view) = browser_view else {
            return;
        };

        view.status = steam.browser_status.clone();

        view.rows = steam
            .browser_entries
            .iter()
            .enumerate()
            .map(|(idx, entry)| {
                let marker = if idx == steam.browser_selected { ">" } else { " " };
                format!(
                    "{} {} [{} / {}] owner:{}",
                    marker,
                    entry.name,
                    entry.members,
                    entry.max_members,
                    entry.owner.raw()
                )
            })
            .collect();

        view.selected_index = if steam.browser_entries.is_empty() {
            None
        } else {
            Some(steam.browser_selected)
        };
    }

    fn sync_targets_from_joined_lobby(steam: &mut SteamSync) {
        let Some(lobby) = steam.joined_lobby else {
            return;
        };
        let mm = steam.client.matchmaking();
        let me = steam.client.user().steam_id();
        steam.targets = mm
            .lobby_members(lobby)
            .into_iter()
            .filter(|id| *id != me)
            .collect();
    }

    pub fn send_local_state(
        local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
        hud: Res<crate::ui::HudState>,
        ergo: Res<crate::config::HumanErgoConfig>,
        input_intent: Res<crate::controls::PlayerInputIntent>,
        mut steam: Option<ResMut<SteamSync>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };
        if steam.targets.is_empty() || steam.last_send.elapsed() < SEND_INTERVAL {
            return;
        }

        let local_id = steam.client.user().steam_id().raw();
        match steam.role {
            RuntimeSteamRole::LegacyPeer => {
                let Ok(transform) = local_cube_query.single() else {
                    return;
                };
                let payload = encode_packet(PACKET_STATE, local_id, transform, hud.selected_color());
                send_payload(steam, &payload);
            }
            RuntimeSteamRole::UntrustedClient => {
                if steam.client_auth_state != SteamClientAuthState::Authenticated {
                    return;
                }
                let Some(token) = steam.client_session_token else {
                    return;
                };

                steam.client_input_sequence = steam.client_input_sequence.wrapping_add(1);
                let payload = encode_input_packet(
                    token,
                    steam.client_input_sequence,
                    &encode_input_payload(
                        input_intent.move_x,
                        input_intent.move_z,
                        input_intent.jump,
                        hud.selected_color(),
                    ),
                );
                steam.metrics.client_input_packets_sent += 1;
                send_game_payload_to_host(steam, &payload);
            }
            RuntimeSteamRole::AuthHost => {
                host_step_authoritative_sim(steam, &ergo, SEND_INTERVAL.as_secs_f32());
                host_broadcast_snapshot(steam);
            }
        }

        steam.last_send = Instant::now();
    }

    pub fn receive_remote_states(mut steam: Option<ResMut<SteamSync>>) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        let local_id = steam.client.user().steam_id().raw();
        let networking = steam.client.networking();

        while let Some(size) = networking.is_p2p_packet_available() {
            let mut buf = vec![0u8; size];
            if let Some((remote, packet_size)) = networking.read_p2p_packet(&mut buf) {
                process_incoming_packet(steam, remote, local_id, &buf[..packet_size]);
            } else {
                break;
            }
        }
    }

    pub fn apply_local_reconciliation(
        time: Res<Time>,
        mut steam: Option<ResMut<SteamSync>>,
        mut local_cube: Query<&mut Transform, With<crate::RotatingCube>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        let Some(target) = steam.pending_local_reconciliation else {
            return;
        };

        let Ok(mut transform) = local_cube.single_mut() else {
            return;
        };

        let alpha = (time.delta_secs() * 12.0).clamp(0.0, 1.0);
        transform.translation = transform.translation.lerp(target.translation, alpha);

        if transform.translation.distance(target.translation) < 0.01 {
            transform.translation = target.translation;
            steam.pending_local_reconciliation = None;
        }
    }

    pub fn sync_remote_projectiles(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut steam: Option<ResMut<SteamSync>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        crate::remote_runtime::drain_remote_projectiles(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut steam.pending_projectiles,
        );
    }

    pub fn sync_remote_cubes(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut cube_query: Query<(Entity, &mut Transform, &SteamRemoteCube, &MeshMaterial3d<StandardMaterial>)>,
        mut steam: Option<ResMut<SteamSync>>,
    ) {
        let Some(steam) = steam.as_deref_mut() else {
            return;
        };

        crate::remote_runtime::apply_departures(
            &mut commands,
            &mut steam.departed_players,
            &mut steam.remote_states,
            &mut steam.spawned_entities,
        );

        let now = Instant::now();
        crate::remote_runtime::prune_remote_states(
            &mut steam.remote_states,
            now,
            REMOTE_TIMEOUT,
            |state| state.last_seen,
        );
        crate::remote_runtime::prune_seen_projectiles(
            &mut steam.seen_projectiles,
            now,
            PROJECTILE_DEDUP_TTL,
        );

        for (player_id, state) in &steam.remote_states {
            if let Some(entity) = steam.spawned_entities.get(player_id).copied() {
                if let Ok((_entity, mut transform, _remote, material_handle)) = cube_query.get_mut(entity) {
                    *transform = state.transform;

                    if let Some(mut material) = materials.get_mut(&material_handle.0) {
                        material.base_color = state.color;
                    }
                } else {
                    steam.spawned_entities.remove(player_id);
                }
            } else {
                let entity = commands
                    .spawn((
                        Mesh3d(meshes.add(Sphere::new(crate::player::CHICKEN_BODY_RADIUS).mesh().uv(32, 18))),
                        MeshMaterial3d(materials.add(player_material(state.color))),
                        state.transform,
                        GlobalTransform::default(),
                        SteamRemoteCube {
                            player_id: *player_id,
                        },
                        crate::player::ChickenBody,
                        crate::player::WalkCycleState::new(Vec2::new(
                            state.transform.translation.x,
                            state.transform.translation.z,
                        )),
                        crate::player::HeadTurnDelayTimer {
                            elapsed: 0.0,
                            delay_secs: 0.5,
                        },
                    ))
                    .with_children(|chicken| {
                        crate::player::spawn_chicken_parts(chicken, &mut *meshes, &mut *materials);
                    })
                    .id();
                steam.spawned_entities.insert(*player_id, entity);
            }
        }

        // Despawn entities whose remote_state was pruned (timeout or departure)
        // but whose Bevy entity wasn't yet removed.
        let active_ids: std::collections::HashSet<u64> = steam.remote_states.keys().copied().collect();
        let stale_ids: Vec<u64> = steam
            .spawned_entities
            .keys()
            .filter(|id| !active_ids.contains(id))
            .copied()
            .collect();

        for player_id in stale_ids {
            if let Some(entity) = steam.spawned_entities.remove(&player_id) {
                commands.entity(entity).despawn();
            }
        }
    }

    fn player_material(color: Color) -> StandardMaterial {
        StandardMaterial {
            base_color: color,
            metallic: 0.1,
            perceptual_roughness: 0.55,
            ..default()
        }
    }

    fn encode_packet(packet_type: u8, player_id: u64, transform: &Transform, color: Color) -> Vec<u8> {
        crate::sync_codec::encode_state_packet(
            STEAM_SYNC_MAGIC,
            STEAM_SYNC_VERSION,
            packet_type,
            PACKET_LEAVE,
            player_id,
            transform,
            color,
        )
    }

    fn encode_freeze_packet(sender_id: u64, target_id: u64) -> Vec<u8> {
        crate::sync_codec::encode_freeze_packet(
            STEAM_SYNC_MAGIC,
            STEAM_SYNC_VERSION,
            PACKET_FREEZE,
            sender_id,
            target_id,
        )
    }

    fn encode_projectile_packet(
        sender_id: u64,
        spawn: &crate::scene::ProjectileSpawnData,
    ) -> Vec<u8> {
        crate::sync_codec::encode_projectile_packet(
            STEAM_SYNC_MAGIC,
            STEAM_SYNC_VERSION,
            PACKET_PROJECTILE,
            sender_id,
            spawn.projectile_id,
            spawn.position,
            spawn.velocity,
            spawn.lifetime_secs,
        )
    }

    fn decode_packet(data: &[u8]) -> Option<(u8, u64, Option<Transform>, Option<Color>)> {
        crate::sync_codec::decode_state_packet(STEAM_SYNC_MAGIC, STEAM_SYNC_VERSION, PACKET_LEAVE, data)
    }

    fn decode_freeze_packet(data: &[u8]) -> Option<(u64, u64)> {
        crate::sync_codec::decode_freeze_packet(
            STEAM_SYNC_MAGIC,
            STEAM_SYNC_VERSION,
            PACKET_FREEZE,
            data,
        )
    }

    fn decode_projectile_packet(
        data: &[u8],
    ) -> Option<(u64, crate::scene::ProjectileSpawnData)> {
        let (player_id, spawn) = crate::sync_codec::decode_projectile_packet(
            STEAM_SYNC_MAGIC,
            STEAM_SYNC_VERSION,
            PACKET_PROJECTILE,
            data,
        )?;

        Some((
            player_id,
            crate::scene::ProjectileSpawnData {
                projectile_id: spawn.projectile_id,
                position: spawn.position,
                velocity: spawn.velocity,
                lifetime_secs: spawn.lifetime_secs,
            },
        ))
    }

    fn process_client_packet(
        steam: &mut SteamSync,
        remote: steamworks::SteamId,
        local_id: u64,
        data: &[u8],
    ) {
        if steam.auth_host.is_some_and(|host| host != remote) {
            steam.metrics.client_auth_peer_mismatch_drops += 1;
            return;
        }

        if let Some(nonce) = decode_auth_challenge(data) {
            steam.metrics.client_auth_challenges_received += 1;
            let proof = learning::auth::make_auth_proof(&steam.auth_secret, local_id, nonce);
            let payload = encode_auth_proof(proof);
            steam.metrics.client_auth_proofs_sent += 1;
            send_auth_payload_to_host(steam, &payload);
            return;
        }

        if let Some(token) = decode_auth_accept(data) {
            steam.metrics.client_auth_accepts_received += 1;
            steam.client_session_token = Some(token);
            steam.client_auth_state = SteamClientAuthState::Authenticated;
            return;
        }

        if let Some(states) = decode_snapshot_packet(data) {
            for (player_id, transform, color) in states {
                if player_id == local_id {
                    steam.pending_local_reconciliation = Some(transform);
                    continue;
                }
                steam.remote_states.insert(
                    player_id,
                    RemoteState {
                        transform,
                        color,
                        last_seen: Instant::now(),
                    },
                );
            }
        }
    }

    fn process_host_packet(steam: &mut SteamSync, remote: steamworks::SteamId, data: &[u8]) {
        if process_host_auth_service_packet(steam, remote, data) {
            return;
        }

        process_host_game_service_packet(steam, remote, data);
    }

    fn process_host_auth_service_packet(
        steam: &mut SteamSync,
        remote: steamworks::SteamId,
        data: &[u8],
    ) -> bool {
        if let Some(player_id) = decode_auth_hello(data) {
            steam.metrics.host_auth_hello_received += 1;
            let nonce = steam
                .auth_sessions
                .get(&remote)
                .map(|s| s.nonce)
                .unwrap_or_else(fresh_nonce);
            steam.auth_sessions.insert(
                remote,
                SteamAuthSession {
                    player_id,
                    nonce,
                    token: None,
                },
            );
            steam.metrics.host_auth_challenges_sent += 1;
            send_auth_payload_to_peer(steam, remote, &encode_auth_challenge(nonce));
            return true;
        }

        if let Some(proof) = decode_auth_proof(data) {
            steam.metrics.host_auth_proofs_received += 1;
            let accepted = if let Some(session) = steam.auth_sessions.get_mut(&remote)
                && proof.nonce == session.nonce
                && proof.player_id == session.player_id
                && learning::auth::verify_auth_proof(&steam.auth_secret, proof)
            {
                let token = learning::auth::mint_session_token(
                    &steam.auth_secret,
                    proof.player_id,
                    proof.nonce,
                );
                session.token = Some(token);
                steam.token_to_player.insert(token, proof.player_id);
                steam.metrics.host_auth_accepts_sent += 1;
                send_auth_payload_to_peer(steam, remote, &encode_auth_accept(token));
                true
            } else {
                false
            };

            if !accepted {
                steam.metrics.host_auth_rejects += 1;
            }
            return true;
        }

        false
    }

    fn process_host_game_service_packet(steam: &mut SteamSync, remote: steamworks::SteamId, data: &[u8]) {
        if let Some((session_token, input_sequence, payload)) = decode_input_packet(data) {
            steam.metrics.host_input_packets_received += 1;

            let Some(player_id) = steam.token_to_player.get(&session_token).copied() else {
                steam.metrics.host_token_rejects += 1;
                return;
            };

            let accepted_peer = match steam.player_ingress_peer.get(&player_id).copied() {
                Some(peer) => peer == remote,
                None => {
                    steam.player_ingress_peer.insert(player_id, remote);
                    true
                }
            };

            if !accepted_peer {
                steam.metrics.host_peer_mismatch_drops += 1;
                return;
            }

            let last_sequence = steam
                .last_input_sequence_by_player
                .get(&player_id)
                .copied()
                .unwrap_or(0);
            if input_sequence <= last_sequence {
                steam.metrics.host_replay_drops += 1;
                return;
            }

            if let Some((move_x, move_z, jump, color)) = decode_input_payload(&payload) {
                steam
                    .last_input_sequence_by_player
                    .insert(player_id, input_sequence);
                steam.authoritative_inputs.insert(
                    player_id,
                    SteamInputSample {
                        move_x,
                        move_z,
                        jump,
                        color,
                    },
                );
                steam.metrics.host_input_packets_accepted += 1;
            } else {
                steam.metrics.host_payload_decode_rejects += 1;
            }
        }
    }

    fn log_metrics_if_due(steam: &mut SteamSync) {
        if steam.last_metrics_log.elapsed() < Duration::from_secs(5) {
            return;
        }
        steam.last_metrics_log = Instant::now();

        let role = match steam.role {
            RuntimeSteamRole::LegacyPeer => "legacy-peer",
            RuntimeSteamRole::AuthHost => "auth-host",
            RuntimeSteamRole::UntrustedClient => "untrusted-client",
        };

        let m = steam.metrics;
        println!(
            "[steam-metrics][{}] client_auth_peer_mismatch_drops={} client_auth_challenges_received={} client_auth_proofs_sent={} client_auth_accepts_received={} client_input_packets_sent={} host_auth_hello_received={} host_auth_challenges_sent={} host_auth_proofs_received={} host_auth_accepts_sent={} host_auth_rejects={} host_input_packets_received={} host_input_packets_accepted={} host_token_rejects={} host_peer_mismatch_drops={} host_replay_drops={} host_payload_decode_rejects={}",
            role,
            m.client_auth_peer_mismatch_drops,
            m.client_auth_challenges_received,
            m.client_auth_proofs_sent,
            m.client_auth_accepts_received,
            m.client_input_packets_sent,
            m.host_auth_hello_received,
            m.host_auth_challenges_sent,
            m.host_auth_proofs_received,
            m.host_auth_accepts_sent,
            m.host_auth_rejects,
            m.host_input_packets_received,
            m.host_input_packets_accepted,
            m.host_token_rejects,
            m.host_peer_mismatch_drops,
            m.host_replay_drops,
            m.host_payload_decode_rejects,
        );
    }

    fn format_metrics_overlay_text(steam: &SteamSync) -> String {
        let role = match steam.role {
            RuntimeSteamRole::LegacyPeer => "legacy-peer",
            RuntimeSteamRole::AuthHost => "auth-host",
            RuntimeSteamRole::UntrustedClient => "untrusted-client",
        };

        let m = steam.metrics;
        format!(
            "Steam Net Metrics (F5 toggle)\nrole: {}\nclient: challenge_rx={} proof_tx={} accept_rx={} auth_peer_drop={} input_tx={}\nhost: hello_rx={} challenge_tx={} proof_rx={} accept_tx={} auth_reject={}\nhost input: rx={} accepted={} token_reject={} peer_drop={} replay_drop={} payload_reject={}",
            role,
            m.client_auth_challenges_received,
            m.client_auth_proofs_sent,
            m.client_auth_accepts_received,
            m.client_auth_peer_mismatch_drops,
            m.client_input_packets_sent,
            m.host_auth_hello_received,
            m.host_auth_challenges_sent,
            m.host_auth_proofs_received,
            m.host_auth_accepts_sent,
            m.host_auth_rejects,
            m.host_input_packets_received,
            m.host_input_packets_accepted,
            m.host_token_rejects,
            m.host_peer_mismatch_drops,
            m.host_replay_drops,
            m.host_payload_decode_rejects,
        )
    }

    /// Advance the authoritative physics simulation for every remote player by `dt` seconds.
    ///
    /// Reads the latest validated [`SteamInputSample`] per player and applies
    /// movement, gravity, jumping, and world-boundary clamping. Results are
    /// written back into `remote_states` so `host_broadcast_snapshot` can
    /// distribute them to connected clients.
    fn host_step_authoritative_sim(
        steam: &mut SteamSync,
        ergo: &crate::config::HumanErgoConfig,
        dt: f32,
    ) {
        if dt <= f32::EPSILON {
            return;
        }

        for (player_id, input) in &steam.authoritative_inputs {
            let vy = steam
                .authoritative_vertical_velocity
                .entry(*player_id)
                .or_insert(0.0);
            let state = steam.remote_states.entry(*player_id).or_insert(RemoteState {
                transform: Transform::from_xyz(0.0, crate::player::CUBE_REST_Y, 0.0),
                color: input.color,
                last_seen: Instant::now(),
            });

            state.transform.translation.x += input.move_x.clamp(-1.0, 1.0) * ergo.movement.move_speed * dt;
            state.transform.translation.z += input.move_z.clamp(-1.0, 1.0) * ergo.movement.move_speed * dt;

            if input.jump && state.transform.translation.y <= crate::player::CUBE_REST_Y + 0.001 {
                *vy = ergo.movement.jump_velocity;
            }

            *vy -= ergo.movement.gravity * dt;
            state.transform.translation.y += *vy * dt;

            if state.transform.translation.y <= crate::player::CUBE_REST_Y {
                state.transform.translation.y = crate::player::CUBE_REST_Y;
                *vy = 0.0;
            }

            state.transform.translation.x = state
                .transform
                .translation
                .x
                .clamp(-ergo.movement.plane_limit, ergo.movement.plane_limit);
            state.transform.translation.z = state
                .transform
                .translation
                .z
                .clamp(-ergo.movement.plane_limit, ergo.movement.plane_limit);
            state.color = input.color;
            state.last_seen = Instant::now();
        }
    }

    fn host_broadcast_snapshot(steam: &SteamSync) {
        let states = steam
            .remote_states
            .iter()
            .map(|(player_id, state)| (*player_id, state.transform, state.color))
            .collect::<Vec<_>>();

        let payload = encode_snapshot_packet(&states);
        for (peer, session) in &steam.auth_sessions {
            if session.token.is_some() {
                send_game_payload_to_peer(steam, *peer, &payload);
            }
        }
    }

    fn send_auth_payload_to_host(steam: &SteamSync, payload: &[u8]) {
        if let Some(host) = steam.auth_host {
            send_payload_to(steam, host, payload, steamworks::SendType::Reliable);
        } else {
            send_payload(steam, payload);
        }
    }

    fn send_game_payload_to_host(steam: &SteamSync, payload: &[u8]) {
        if let Some(host) = steam.auth_host {
            send_payload_to(
                steam,
                host,
                payload,
                steamworks::SendType::UnreliableNoDelay,
            );
        } else {
            send_payload(steam, payload);
        }
    }

    fn send_auth_payload_to_peer(steam: &SteamSync, target: steamworks::SteamId, payload: &[u8]) {
        send_payload_to(steam, target, payload, steamworks::SendType::Reliable);
    }

    fn send_game_payload_to_peer(steam: &SteamSync, target: steamworks::SteamId, payload: &[u8]) {
        send_payload_to(
            steam,
            target,
            payload,
            steamworks::SendType::UnreliableNoDelay,
        );
    }

    fn send_payload_to(
        steam: &SteamSync,
        target: steamworks::SteamId,
        payload: &[u8],
        send_type: steamworks::SendType,
    ) {
        let networking = steam.client.networking();
        networking.accept_p2p_session(target);
        let _ = networking.send_p2p_packet(target, send_type, payload);
    }

    /// Generate a unique challenge nonce from the current wall-clock nanoseconds.
    /// Used when a peer sends a hello with no existing auth session.
    fn fresh_nonce() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }

    fn encode_auth_hello(player_id: u64) -> Vec<u8> {
        crate::auth_codec::encode_auth_hello(STEAM_SYNC_MAGIC, STEAM_SYNC_VERSION, player_id)
    }

    fn decode_auth_hello(data: &[u8]) -> Option<u64> {
        crate::auth_codec::decode_auth_hello(STEAM_SYNC_MAGIC, STEAM_SYNC_VERSION, data)
    }

    fn encode_auth_challenge(nonce: u64) -> Vec<u8> {
        crate::auth_codec::encode_auth_challenge(STEAM_SYNC_MAGIC, STEAM_SYNC_VERSION, nonce)
    }

    fn decode_auth_challenge(data: &[u8]) -> Option<u64> {
        crate::auth_codec::decode_auth_challenge(STEAM_SYNC_MAGIC, STEAM_SYNC_VERSION, data)
    }

    fn encode_auth_proof(proof: learning::auth::AuthProof) -> Vec<u8> {
        crate::auth_codec::encode_auth_proof(STEAM_SYNC_MAGIC, STEAM_SYNC_VERSION, proof)
    }

    fn decode_auth_proof(data: &[u8]) -> Option<learning::auth::AuthProof> {
        crate::auth_codec::decode_auth_proof(STEAM_SYNC_MAGIC, STEAM_SYNC_VERSION, data)
    }

    fn encode_auth_accept(token: learning::auth::SessionToken) -> Vec<u8> {
        crate::auth_codec::encode_auth_accept(STEAM_SYNC_MAGIC, STEAM_SYNC_VERSION, token)
    }

    fn decode_auth_accept(data: &[u8]) -> Option<learning::auth::SessionToken> {
        crate::auth_codec::decode_auth_accept(STEAM_SYNC_MAGIC, STEAM_SYNC_VERSION, data)
    }

    fn encode_input_payload(move_x: f32, move_z: f32, jump: bool, color: Color) -> Vec<u8> {
        crate::auth_codec::encode_input_payload(move_x, move_z, jump, color)
    }

    fn decode_input_payload(payload: &[u8]) -> Option<(f32, f32, bool, Color)> {
        crate::auth_codec::decode_input_payload(payload)
    }

    fn encode_input_packet(
        session_token: learning::auth::SessionToken,
        input_sequence: u32,
        payload: &[u8],
    ) -> Vec<u8> {
        crate::auth_codec::encode_input_packet(
            STEAM_SYNC_MAGIC,
            STEAM_SYNC_VERSION,
            session_token,
            input_sequence,
            payload,
        )
    }

    fn decode_input_packet(data: &[u8]) -> Option<(learning::auth::SessionToken, u32, Vec<u8>)> {
        crate::auth_codec::decode_input_packet(STEAM_SYNC_MAGIC, STEAM_SYNC_VERSION, data)
    }

    fn encode_snapshot_packet(states: &[(u64, Transform, Color)]) -> Vec<u8> {
        crate::auth_codec::encode_snapshot_packet(STEAM_SYNC_MAGIC, STEAM_SYNC_VERSION, states)
    }

    fn decode_snapshot_packet(data: &[u8]) -> Option<Vec<(u64, Transform, Color)>> {
        crate::auth_codec::decode_snapshot_packet(STEAM_SYNC_MAGIC, STEAM_SYNC_VERSION, data)
    }
}

#[cfg(not(feature = "steamworks"))]
mod imp {
    use super::*;

    #[derive(Component, Debug, Clone, Copy)]
    pub struct SteamRemoteCube {
        pub player_id: u64,
    }

    #[derive(Resource)]
    pub struct SteamSync;

    #[derive(Resource, Default)]
    pub struct SteamBrowserView {
        pub status: String,
        pub rows: Vec<String>,
        pub selected_index: Option<usize>,
    }

    pub fn setup_steam_sync(_commands: Commands) {}
    pub fn setup_steam_metrics_overlay(_commands: Commands) {}
    pub fn auto_refresh_browser_on_startup(_steam: Option<ResMut<SteamSync>>) {}
    pub fn process_callbacks(_browser_view: Option<ResMut<SteamBrowserView>>) {}
    pub fn update_steam_metrics_overlay(_keyboard: Res<ButtonInput<KeyCode>>) {}
    pub fn update_server_browser_controls(_keyboard: Res<ButtonInput<KeyCode>>) {}
    pub fn send_freeze_target(_steam: &mut SteamSync, _sender_id: u64, _target_id: u64) {}
    pub fn send_projectile_spawn(
        _steam: &mut SteamSync,
        _sender_id: u64,
        _spawn: &crate::scene::ProjectileSpawnData,
    ) {
    }
    pub fn apply_local_freeze(
        _local_player: Res<crate::multiplayer::LocalPlayerId>,
        _freeze: ResMut<crate::controls::MovementFreeze>,
    ) {
    }
    pub fn announce_local_presence(_local_cube_query: Query<&Transform, With<crate::RotatingCube>>) {}
    pub fn send_local_leave(
        _exit_requested: Res<crate::ExitRequested>,
        _local_cube_query: Query<&Transform, With<crate::RotatingCube>>,
    ) {
    }
    pub fn send_local_state(_local_cube_query: Query<&Transform, With<crate::RotatingCube>>) {}
    pub fn receive_remote_states() {}
    pub fn apply_local_reconciliation(
        _time: Res<Time>,
        _local_cube: Query<&mut Transform, With<crate::RotatingCube>>,
    ) {
    }
    pub fn sync_remote_cubes(
        _commands: Commands,
        _meshes: ResMut<Assets<Mesh>>,
        _materials: ResMut<Assets<StandardMaterial>>,
    ) {
    }
    pub fn sync_remote_projectiles(
        _commands: Commands,
        _meshes: ResMut<Assets<Mesh>>,
        _materials: ResMut<Assets<StandardMaterial>>,
    ) {
    }
}

pub use imp::*;
