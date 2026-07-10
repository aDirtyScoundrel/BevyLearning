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

### LAN UDP sync

Runs automatically and broadcasts transform packets on port `34567`.

Optional explicit target:

```bash
CUBE_SYNC_TARGET=192.168.1.42:34567 cargo run
```

### Steamworks P2P sync

Uses Steam P2P packet APIs with explicit peer IDs.

```bash
STEAM_REMOTE_IDS=<peer_steam64_id> cargo run --features steamworks
```

Use comma-separated Steam64 IDs for multiple peers.

## Notes for GitHub Publishing

- `target/` is ignored via `.gitignore`
- `steam_appid.txt` is for local Spacewar test setup only
- Do not commit personal secrets/tokens
- Replace App ID `480` with your real app ID when moving beyond test mode

## Release Packaging

One-command release workflow:

```bash
./scripts/release.sh --version v0.1.1
```

Include GitHub Release asset upload (requires `GITHUB_TOKEN`):

```bash
GITHUB_TOKEN=... ./scripts/release.sh --version v0.1.1 --upload-release
```

Preview actions without making changes:

```bash
./scripts/release.sh --version v0.1.1 --dry-run
```

## Status

This repository currently contains strong networking/protocol scaffolding and local/remote multiplayer experiments. It is not yet a full server-authoritative production multiplayer implementation.
