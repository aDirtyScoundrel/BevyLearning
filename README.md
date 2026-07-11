# Learning (Bevy + Tribes Netcode Experiments)

A Rust/Bevy project exploring classic Tribes-style networking, bitstream/Huffman protocol work, and multiplayer cube sync.

## Current Highlights

- LSB-first bitstream reader/writer
- Static Huffman codec implementation
- Query/protocol scaffolding migrated toward Steamworks/A2S
- Built-in Bevy FPS diagnostics overlay
- Keyboard-driven cube controls with acceleration
- LAN UDP cube sync scaffold
- Steamworks P2P cube sync scaffold (feature-gated)

## Requirements

- Rust toolchain (via rustup)
- Steam client installed and running for Steamworks mode
- `steam_appid.txt` is set to `480` for Spacewar development testing

## Run

Default run:

```bash
cargo run
```

Release run:

```bash
cargo run --release
```

Steamworks-enabled run:

```bash
cargo run --features steamworks
```

## Controls

- `Space`: pause/unpause rotation
- `Left` / `Right`: decrease/increase horizontal rotation speed while held
- `R`: reset horizontal speed
- `Up` / `Down`: increase/decrease vertical rotation speed while held
- `X`: reset vertical speed

## Multiplayer Scaffolds

Trust model note: `src/multiplayer.rs` and `src/steam_mp.rs` are still transitional sync scaffolds.
The long-term architecture is now split into an authentication server and untrusted clients:

- Server owns authority and only accepts client input after auth challenge/proof + session token issuance.
- Clients are treated as untrusted and must not be accepted as state authorities.
- New layout scaffolding lives in `src/server/*`, `src/client/*`, and `src/auth.rs`.

### LAN UDP sync

Runs automatically and broadcasts transform packets on port `34567`.

Optional explicit target:

```bash
CUBE_SYNC_TARGET=192.168.1.42:34567 cargo run
```

### Auth server + untrusted clients (LAN)

Start authoritative auth server:

```bash
CUBE_AUTH_SERVER=1 CUBE_AUTH_SECRET=dev-auth-secret cargo run
```

Start untrusted client targeting that server:

```bash
CUBE_AUTH_SERVER_ADDR=192.168.1.42:34567 CUBE_AUTH_SECRET=dev-auth-secret cargo run
```

Notes:

- Clients send authenticated input packets, not authoritative transform state.
- Server validates auth proof, issues session token, and broadcasts authoritative snapshots.
- Untrusted clients reconcile local player position toward server snapshots.

### Steamworks P2P sync

Uses Steam P2P packet APIs with explicit peer IDs.

```bash
STEAM_REMOTE_IDS=<peer_steam64_id> cargo run --features steamworks
```

Use comma-separated Steam64 IDs for multiple peers.

Auth host + untrusted client mode over Steam P2P:

Host:

```bash
STEAM_AUTH_HOST=1 STEAM_AUTH_SECRET=dev-auth-secret STEAM_REMOTE_IDS=<peer_steam64_id> cargo run --features steamworks
```

Client:

```bash
STEAM_AUTH_HOST_ID=<host_steam64_id> STEAM_AUTH_SECRET=dev-auth-secret STEAM_REMOTE_IDS=<host_steam64_id> cargo run --features steamworks
```

Notes:

- Client path uses auth challenge/proof, then tokened input packets.
- Host path simulates authoritative state and publishes snapshots.
- In-game Steam server browser controls:
	- `F6`: refresh server list
	- `Up` / `Down`: select server row
	- `F7`: join selected server
- Auth host mode auto-creates a public Steam lobby and advertises `server_name` metadata for browser listing.

## Notes for GitHub Publishing

- `target/` is ignored via `.gitignore`
- `steam_appid.txt` is for local Spacewar test setup only
- Do not commit personal secrets/tokens
- Replace App ID `480` with your real app ID when moving beyond test mode

## Release Packaging

One-command release workflow:

```bash
./scripts/release.sh --version v0.2.0
```

Include GitHub Release asset upload (requires `GITHUB_TOKEN`):

```bash
GITHUB_TOKEN=... ./scripts/release.sh --version v0.2.0 --upload-release
```

Preview actions without making changes:

```bash
./scripts/release.sh --version v0.2.0 --dry-run
```

## Status

This repository currently contains strong networking/protocol scaffolding and local/remote multiplayer experiments. It is not yet a full server-authoritative production multiplayer implementation.
