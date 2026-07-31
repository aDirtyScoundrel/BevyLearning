//! Single-player mode: spawns the current executable as a headless
//! authoritative-server subprocess (`CUBE_AUTH_SERVER=1`) and connects this
//! process to it as an ordinary `UntrustedClient` over loopback — the exact
//! same auth-server/client split used for real multiplayer, just with the
//! server living in a child process on the same machine instead of a
//! separate host. See docs/THIN_CLIENT_MIGRATION.md: single player is not a
//! separate code path, it's the multiplayer server-authoritative path
//! pointed at `127.0.0.1`.
//!
//! `multiplayer::setup_network` already binds an ephemeral port for
//! `UntrustedClient` (see its doc comment) specifically so this client can
//! share a host with its own embedded server without a port collision.

use bevy::prelude::*;
use std::net::SocketAddr;
use std::process::{Child, Command};
use std::time::Duration;

const SINGLE_PLAYER_AUTH_SECRET: &str = "single-player-local-secret";
/// How long to give the embedded server subprocess to bind its socket
/// before this process starts sending auth-hello packets at it. UDP has no
/// connection handshake to block on, so this is a best-effort delay, not a
/// guarantee — `ClientNetworkManager`'s hello/challenge/proof flow is
/// naturally retried every frame via `announce_local_presence` until the
/// server responds, so a slow-starting server just delays authentication
/// rather than breaking it outright.
const SERVER_STARTUP_GRACE: Duration = Duration::from_millis(300);

/// Tracks the embedded server subprocess for this session so it can be
/// killed when the client exits. Absent when not in single-player mode.
#[derive(Resource)]
pub struct EmbeddedServerProcess {
    child: Child,
}

impl Drop for EmbeddedServerProcess {
    fn drop(&mut self) {
        // Best-effort: if the process already exited on its own, `kill()`
        // returning an error here is expected and not worth surfacing.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawns the embedded headless-server subprocess and inserts both
/// [`EmbeddedServerProcess`] (so it can be torn down later) and
/// [`crate::multiplayer::SinglePlayerConnect`] (so the next
/// `multiplayer::setup_network` call in this process connects to it as a
/// client instead of picking a role from env vars).
///
/// Call this once on entering single-player mode, before the `InGame`
/// `Startup` schedule runs `multiplayer::setup_network`.
pub fn spawn_embedded_server(commands: &mut Commands) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;

    let child = Command::new(exe)
        .env("CUBE_AUTH_SERVER", "1")
        .env("CUBE_AUTH_SECRET", SINGLE_PLAYER_AUTH_SECRET)
        .spawn()?;

    println!(
        "[singleplayer] spawned embedded server subprocess (pid {})",
        child.id()
    );

    commands.insert_resource(EmbeddedServerProcess { child });
    commands.insert_resource(crate::multiplayer::SinglePlayerConnect {
        target_addr: SocketAddr::from(([127, 0, 0, 1], crate::multiplayer::SYNC_PORT)),
        auth_secret: SINGLE_PLAYER_AUTH_SECRET.to_string(),
    });

    // See SERVER_STARTUP_GRACE's doc comment — this is a courtesy delay,
    // not a synchronization primitive. The auth handshake retries on its
    // own if the server isn't ready yet.
    std::thread::sleep(SERVER_STARTUP_GRACE);

    Ok(())
}

/// Kills the embedded server subprocess if one is tracked. Call this on
/// `AppExit` (mirrors `multiplayer::send_local_leave`'s existing
/// exit-handling pattern) — without it, a headless server subprocess would
/// keep running (and holding the UDP port) after the client window closes.
pub fn shutdown_embedded_server(
    exit_requested: Res<crate::ExitRequested>,
    mut commands: Commands,
    server_process: Option<Res<EmbeddedServerProcess>>,
) {
    if !exit_requested.0 || server_process.is_none() {
        return;
    }
    // Dropping the resource runs `EmbeddedServerProcess::drop`, which kills
    // and reaps the child.
    commands.remove_resource::<EmbeddedServerProcess>();
}
