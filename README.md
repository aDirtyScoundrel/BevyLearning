# Learning

A 3D multiplayer game built with Rust and Bevy. You play as a chicken on a shared floor, moving around, jumping, and shooting seed projectiles at other players. Multiplayer runs over LAN UDP or Steam P2P, with an authoritative server architecture and client-side reconciliation.

## Features

- **Chicken player character** — animated walk cycle, wing flap, head turn delay, and beak cone
- **3D movement** — WASD + mouse camera orbit, jump, seed projectile firing with hit detection
- **Movement freeze** — networked freeze effect triggered by projectile hits
- **Authoritative server tick** — server owns movement state; clients send authenticated input and reconcile toward snapshots
- **LAN and Steam P2P multiplayer** — both transports share a single packet codec
- **In-game Steam server browser** — lobby refresh, row selection, and join via keyboard (`F6` / `F7`)
- **Rebindable controls** — ergo preset system with save/load (`F8` to open)
- **HUD** — player color picker (hex + sliders for metallic/roughness)
- **FPS overlay** — built-in frame time diagnostics
- **Bitstream + Huffman codec** — LSB-first bitstream and static Huffman implementation underlying the packet protocol
- **A2S / GameInfo query scaffolding** — server info query protocol support

## Requirements

- Rust toolchain (via rustup)
- Steam client running for Steamworks mode
- `steam_appid.txt` set to `480` for Spacewar development testing (replace with your app ID for production)

## Run

```bash
# Default (LAN, no Steam)
cargo run

# Load a DOOM map container (WAD or PK3, defaults to MAP01)
DOOM_WAD=/path/to/doom2.wad DOOM_MAP=MAP01 cargo run

# Release build
cargo run --release

# Steam P2P enabled
cargo run --features steamworks
```

## Controls

| Action | Default Key |
|---|---|
| Move | `W` `A` `S` `D` |
| Turn camera | Mouse / `←` `→` |
| Pitch camera | `↑` `↓` |
| Jump | `Space` |
| Shoot seed | Mouse button / configured key |
| Toggle pause | `F` |
| Toggle Mach menu (ergo/rebind) | `F8` |
| Toggle WAD picker | `F9` |
| Toggle collision wireframe | `F10` |
| Toggle noclip | `F11` |
| Reset vertical speed | `X` |
| Reset horizontal speed | `R` |

Controls are fully rebindable from the in-game ergo menu. Settings persist to `human_ergo_preset.cfg`.

## Multiplayer

The networking model now uses a true auth-server split inside the authoritative host runtime:

- Auth service path: handles hello/challenge/proof and mints session tokens.
- Game service path: accepts only token-bound, sequence-validated input and advances simulation.

Clients authenticate first, then send gameplay input packets using the issued session token. The server advances game state and broadcasts authoritative snapshots; clients reconcile local position toward received snapshots.

Quick checklist before you start:

1. Build once so dependencies are ready:

```bash
cargo check
```

2. If using Steam mode, run Steam client and sign in on every machine.
3. Use the same auth secret string for everyone in one match.
4. Open this game in terminal windows side-by-side so host and client logs are visible.

### 60-second sanity test (first time setup)

Use this once to confirm your setup is healthy before tuning anything.

1. Start host terminal first.
2. Start one client terminal second.
3. Wait 5 to 10 seconds.
4. Confirm expected log signals below.

LAN expected host log signals:

- `[server] starting headless authoritative server`
- `[multiplayer] listening on 0.0.0.0:34567 and broadcasting to ...`

LAN expected client log signals:

- `[multiplayer] listening on 0.0.0.0:34567 and broadcasting to <host_ip>:34567`
- Local and remote player motion eventually reconcile (no permanent desync).

Steam expected host log signals:

- `[steam-mp] local steam id: ...`
- Repeating `[steam-metrics]` lines where host auth/input counters increase from zero after client joins.

Steam expected client log signals:

- `[steam-mp] local steam id: ...`
- Repeating `[steam-metrics]` lines where `challenge_rx`, `proof_tx`, and `accept_rx` move above zero.
- Press `F5` to open overlay and confirm counters change while moving.

If these signals do not appear:

1. Re-check that host/client use identical auth secret values.
2. Re-check that host address or host Steam64 ID is correct.
3. For LAN, verify UDP 34567 is allowed through firewall on host.
4. For Steam, ensure both users are online in Steam and running with `--features steamworks`.

### Single-machine quick start (2 terminals)

If you just want a fast local test on one PC, open two terminal windows in the project root.

Terminal A (host):

```bash
CUBE_AUTH_SERVER=1 CUBE_AUTH_SECRET=dev-auth-secret cargo run
```

Terminal B (client):

```bash
CUBE_AUTH_SERVER_ADDR=127.0.0.1:34567 CUBE_AUTH_SECRET=dev-auth-secret cargo run
```

You should see host and client exchange state within a few seconds.

### LAN

Use this when players are on the same local network and you do not need Steam relay.

1. Pick a host machine.
2. On host, start server runtime:

```bash
CUBE_AUTH_SERVER=1 CUBE_AUTH_SECRET=dev-auth-secret cargo run
```

3. Find host LAN IP (example `192.168.1.20`).
4. On each client machine, connect to host:

```bash
CUBE_AUTH_SERVER_ADDR=<host_ip>:34567 CUBE_AUTH_SECRET=dev-auth-secret cargo run
```

Example:

```bash
CUBE_AUTH_SERVER_ADDR=192.168.1.20:34567 CUBE_AUTH_SECRET=dev-auth-secret cargo run
```

- Default port is `34567`.
- `CUBE_AUTH_SECRET` must match between host and all clients.
- Run exactly one host per session.
- If clients cannot connect, verify OS firewall allows UDP `34567` on host.

### Steam P2P

Use this when players are on different networks or want Steam NAT traversal.

1. Host and clients launch with Steamworks feature:

```bash
cargo run --features steamworks
```

2. Host gets Steam64 ID from startup log line: `[steam-mp] local steam id: ...`
3. Host starts authoritative Steam auth host:

```bash
STEAM_AUTH_HOST=1 STEAM_AUTH_SECRET=dev-auth-secret STEAM_REMOTE_IDS=<client_steam64_id> cargo run --features steamworks
```

4. Client starts and points at host Steam64 ID:

```bash
STEAM_AUTH_HOST_ID=<host_steam64_id> STEAM_AUTH_SECRET=dev-auth-secret STEAM_REMOTE_IDS=<host_steam64_id> cargo run --features steamworks
```

Example client command:

```bash
STEAM_AUTH_HOST_ID=76561198000000000 STEAM_AUTH_SECRET=dev-auth-secret STEAM_REMOTE_IDS=76561198000000000 cargo run --features steamworks
```

- Use comma-separated Steam64 IDs in `STEAM_REMOTE_IDS` for multiple peers.
- Auth host auto-creates a public Steam lobby and advertises server metadata for the in-game browser.
- `STEAM_AUTH_SECRET` must match between host and all clients.
- Steam auth uses a dedicated auth-service path (hello/challenge/proof/accept) with reliable Steamworks packet send.
- Steam gameplay uses a separate game-service path (token-bound input + snapshots) over low-latency unreliable send.
- Runtime prints `[steam-metrics]` every 5s with auth attempts/accepts/rejects plus token, replay, and peer-mismatch drops.
- Press `F5` in-game to toggle on-screen Steam net metrics overlay.

**In-game server browser (Steam mode):**

- `F6` — refresh lobby list
- `↑` / `↓` — select a row
- `F7` — join selected lobby

## Release Packaging

```bash
./scripts/release.sh --version v0.3.1
```

Upload to GitHub Releases (requires `GITHUB_TOKEN`):

```bash
GITHUB_TOKEN=<token> ./scripts/release.sh --version v0.3.1 --upload-release
```

## Notes

- `target/` is gitignored.
- `steam_appid.txt` is for local Spacewar testing only — do not commit secrets or tokens.
- Replace App ID `480` with your real Steam app ID before shipping.
- If release creation fails with API errors, verify token permissions include repository write access (classic PAT: `repo`; fine-grained PAT: Contents `Read and write`).
- After publishing, revoke/rotate temporary tokens used for release automation.
- WAD/PK3 loading currently uses classic map lumps (`VERTEXES`, `LINEDEFS`, `SIDEDEFS`, `SECTORS`, optional `THINGS`) to build geometry.
- PK3 support loads maps from embedded `.wad` entries and direct UDMF `TEXTMAP` entries.
- Runtime map picker scans `./` and `./wads` for `.wad` and `.pk3` files; press `F9`, choose with `Up`/`Down`, `Enter` to load, `R` to refresh.

Preview actions without making changes:

```bash
./scripts/release.sh --version v0.3.1 --dry-run
```

### Release Checklist

1. Ensure local branch is `main` and clean enough for release.
2. Ensure version is updated in `Cargo.toml` and notes in `PATCH_NOTES.md`.
3. Ensure tag exists for release version:

```bash
git tag --list v0.3.1
```

4. Run release script:

```bash
./scripts/release.sh --version v0.3.1 --upload-release
```

5. Confirm on GitHub:

- Tag is present.
- Release entry exists.
- Uploaded asset is attached.

## Status

This repository currently contains strong networking/protocol scaffolding and local/remote multiplayer experiments. It is not yet a full server-authoritative production multiplayer implementation.
