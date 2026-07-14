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

The networking model uses an authoritative server. Clients send authenticated input packets; the server validates them, advances game state, and broadcasts authoritative snapshots. Clients reconcile local position toward received snapshots.

### LAN

**Start host:**

```bash
CUBE_AUTH_SERVER=1 CUBE_AUTH_SECRET=dev-auth-secret cargo run
```

**Start client:**

```bash
CUBE_AUTH_SERVER_ADDR=<host_ip>:34567 CUBE_AUTH_SECRET=dev-auth-secret cargo run
```

- Default port is `34567`.
- `CUBE_AUTH_SECRET` must match between host and all clients.
- Run exactly one host per session.

### Steam P2P

**Start host:**

```bash
STEAM_AUTH_HOST=1 STEAM_AUTH_SECRET=dev-auth-secret STEAM_REMOTE_IDS=<client_steam64_id> cargo run --features steamworks
```

**Start client:**

```bash
STEAM_AUTH_HOST_ID=<host_steam64_id> STEAM_AUTH_SECRET=dev-auth-secret STEAM_REMOTE_IDS=<host_steam64_id> cargo run --features steamworks
```

- Use comma-separated Steam64 IDs in `STEAM_REMOTE_IDS` for multiple peers.
- Auth host auto-creates a public Steam lobby and advertises server metadata for the in-game browser.
- `STEAM_AUTH_SECRET` must match between host and all clients.

**In-game server browser (Steam mode):**

- `F6` — refresh lobby list
- `↑` / `↓` — select a row
- `F7` — join selected lobby

## Release Packaging

```bash
./scripts/release.sh --version v0.3.0
```

Upload to GitHub Releases (requires `GITHUB_TOKEN`):

```bash
GITHUB_TOKEN=<token> ./scripts/release.sh --version v0.3.0 --upload-release
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
./scripts/release.sh --version v0.3.0 --dry-run
```

### Release Checklist

1. Ensure local branch is `main` and clean enough for release.
2. Ensure version is updated in `Cargo.toml` and notes in `PATCH_NOTES.md`.
3. Ensure tag exists for release version:

```bash
git tag --list v0.3.0
```

4. Run release script:

```bash
./scripts/release.sh --version v0.3.0 --upload-release
```

5. Confirm on GitHub:

- Tag is present.
- Release entry exists.
- Uploaded asset is attached.

## Status

This repository currently contains strong networking/protocol scaffolding and local/remote multiplayer experiments. It is not yet a full server-authoritative production multiplayer implementation.
