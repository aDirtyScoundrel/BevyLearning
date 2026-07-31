# AGENTS.md

Guidance for AI agents working in this repo. For user-facing run/setup
instructions (controls, env vars, Steam/LAN multiplayer walkthrough), see
`README.md` — this file does not duplicate that.

## What this is

Rust/Bevy 3D multiplayer game (`edition = "2024"`, Bevy 0.19). Players are
chickens on a shared floor; movement/shooting sync over either LAN UDP or
Steam P2P, with an authoritative-server model. Also embeds a DOOM WAD/PK3
loader for level geometry. ~11k lines across a lib crate + one bin crate.

## Crate structure — read this first

This is **two crates sharing a directory**: a library (`src/lib.rs`) and a
binary (`src/main.rs`), each with their **own separate module tree**. Don't
assume a module declared in one is visible from the other without going
through `learning::`.

- **Library (`learning::`, declared in `lib.rs`)** — transport-agnostic wire
  protocol and auth primitives: `bitstream`, `huffman`, `auth`, `sync_codec`,
  `protocol` (`a2s`, `gameinfo`, `query`, `mod`), `connection`, `packet`,
  `client`, `server` (`mod`, `session`). These have no Bevy dependency and are
  reusable by both transports.
- **Binary (`main.rs` + its `mod` declarations)** — the actual game: `controls`,
  `config`, `auth_codec`, `multiplayer` (LAN transport), `scene`, `ui`,
  `skybox`, `steam_mp` (Steam P2P transport), `player`, `doom_wad`,
  `sync_codec` (unified with lib, shared by both LAN and Steam transports),
  `remote_runtime`, `server_tick`, `mainmenu` (title screen + `AppState`),
  `singleplayer` (embedded-server subprocess for single player).

When you add networking logic that's transport-agnostic, it likely belongs in
the lib crate under `learning::`. Bevy systems, ECS components, and anything
touching `Commands`/`Query`/`Res` belong in the bin crate.

## Build & test

```bash
cargo check                          # default features (steamworks on)
cargo check --no-default-features    # LAN-only path — always check both
cargo test                           # same: run under both feature sets
cargo test --no-default-features
```

`steamworks` is a **default-on** feature (see `Cargo.toml`). Any change
touching `steam_mp.rs`, `main.rs`'s `#[cfg(feature = "steamworks")]` blocks,
or shared types they depend on must be verified under **both** configs — the
`#[cfg(not(feature = "steamworks"))]` stub module in `steam_mp.rs` has to keep
matching the real impl's public function signatures (Bevy registers systems
by function item, not call site, so slimmed stub signatures are fine as long
as `main.rs` never calls them positionally).

Tests live inline as `#[cfg(test)] mod tests { ... }` at the bottom of the
file they cover (see `auth_codec.rs`, `protocol/a2s.rs`,
`server/session.rs`, `doom_wad.rs` for examples). Add tests in the same file,
same convention — don't create a separate `tests/` integration dir unless the
test genuinely needs to exercise the compiled binary.

## Tooling pitfalls specific to this repo

- **Let-chains (edition 2024) trip up naive linters.** Code like
  `if let Some(x) = a && let Some(y) = b { ... }` is valid and used
  pervasively (`server/mod.rs`, `steam_mp.rs`, `ui.rs`, `controls.rs`,
  `multiplayer.rs`). If a standalone syntax checker flags "let chains are
  only allowed in Rust 2024 or later" as an error, that's the checker not
  knowing the crate's edition — it's a false positive. Trust `cargo check`
  output, not a bare `rustc`/lint pass.
- **rustfmt-diff noise on unrelated lines.** Automated patch tools in this
  environment may show a large reformatting diff alongside a small logical
  edit — that's rustfmt normalizing pre-existing style debt elsewhere in the
  file, not a sign your edit broke something. Only `cargo check`/`cargo test`
  failures are real signal.

## Duplication you'll run into on purpose

LAN (`multiplayer.rs`) and Steam P2P (`steam_mp.rs`) transports are separate,
parallel implementations of the same auth/session/sim model — this is
acknowledged tech debt (see doc comments in both files), not an oversight.
Concretely:

- ~~`server_tick.rs`'s sim-step function mirrors `steam_mp::host_step_authoritative_sim`
  near byte-for-byte.~~ **Resolved.** `steam_mp::host_step_authoritative_sim`
  is now a thin adapter that delegates to `server_tick::step_authoritative_sim`.
  The two `RemoteState` / input-sample structs (which were field-for-field
  identical) were also unified onto the `multiplayer::` definitions, so the
  type system now prevents the sim from silently forking again.
  `server_tick.rs` carries the tests, including one that pins both callers to
  identical output.
- Packet-codec glue (encode/decode wrappers) is duplicated between the two
  files, differing only in the magic/version constant.
- Session/auth bookkeeping (challenge → proof → token → input-sequence
  replay protection) is reimplemented per-transport: `server/mod.rs` +
  `server/session.rs` for LAN, in-module structs in `steam_mp.rs` for Steam.

**If you fix a bug or add a check in one transport's auth/session/sim path,
check whether the same class of bug exists in the other transport before
calling the fix done.** `remote_runtime.rs` holds the housekeeping that *is*
already shared (`apply_departures`, `prune_remote_states`,
`prune_seen_projectiles`, `drain_remote_projectiles`,
`find_remote_hit_target`) — prefer extending that module over adding a third
copy of shared logic.

## Untrusted-input surfaces — extra scrutiny required

These files parse bytes from the network or from user-supplied game files.
Treat all input as adversarial: no `unwrap()`/`expect()`/raw indexing on
attacker-controlled data, validate lengths before allocating, and clamp
anything read directly off the wire before using it as a size/count.

- Network wire format: `packet.rs`, `bitstream.rs`, `huffman.rs`,
  `sync_codec.rs`, `auth_codec.rs`, `protocol/*`, `auth.rs`.
- File format: `doom_wad.rs` (WAD binary lumps + PK3/zip + UDMF TEXTMAP text).
  Existing convention here: bounds-checked helpers (`read_u16_le`,
  `read_i16_le`, `read_u8`, `read_name8`) that return `Result` via
  `.get(range).ok_or_else(...)` instead of raw slice indexing, and
  `checked_mul`/`checked_add` for any offset/size arithmetic. Follow this
  pattern rather than introducing raw indexing.
- Archive/text reads from PK3 entries go through `read_entry_bytes_capped`/
  `read_entry_string_capped` (defined near the top of `doom_wad.rs`) which
  enforce a 64 MiB cap — route any new archive-entry read through these
  instead of calling `read_to_end`/`read_to_string` directly.

## Conventions worth matching

- Config/tuning values live in `config.rs` (`HumanErgoConfig` and friends);
  UI-side clamping goes through `ErgoSetting::range()`/`set_value()` in
  `ui.rs` — any code path that sets a tuning value (including file-loaded
  presets) should route through `set_value()` rather than assigning fields
  directly, so out-of-range/malformed values can't skip the clamp.
- Per-frame systems that integrate `time.delta_secs()` into physics/position
  state should clamp `dt` first (see `move_cube` in `controls.rs`,
  `doom_wad.rs`'s existing pattern) — an unclamped `dt` after a frame stall
  is a real blow-up vector for accumulated state like velocity.
- Sequence-number/replay comparisons must be wraparound-aware
  (`seq.wrapping_sub(last) as i32 > 0`), not a raw `<=`/`>` — see
  `server/session.rs::accepts_input_sequence` for the reference
  implementation, mirrored in `steam_mp.rs`'s host input-replay check.

## Design goal: classic-map compatibility

Being able to load and play **open-source/free maps from classic games
as-is, with no modifications**, is a standing design goal — not just "parse
enough WAD lumps to render something." Doom/Doom II/BOOM-format community
maps are the active target; Marathon-format maps are an explicit future
extension, tracked separately since it's a structurally different map
format (polygon-based, not linedef/sector) and must not be bolted onto
`doom_wad.rs`. See `docs/CLASSIC_MAP_SUPPORT.md` for the current gap
analysis and ordered action plan (linedef special dispatch, real
walk/switch/shoot activation, keycards, multi-spawn, sector specials, BOOM
generalized specials, then collision spatial partitioning) before starting
any work on doors, triggers, keys, or spawn logic in `doom_wad.rs`.

## Design goal: server-authoritative thin client

Client should run as little authoritative game logic as possible — ideally
just render state + relay input to the server, with the server owning
movement, doors/triggers, and projectile-hit outcomes. This is NOT yet the
case today: only *remote* players are server-simulated
(`server_tick::step_authoritative_sim`/`steam_mp::host_step_authoritative_sim`);
the local player's own movement (`controls::move_cube`) still runs
authoritatively on the client itself. See `docs/THIN_CLIENT_MIGRATION.md`
for the gap analysis, the client-side-prediction tradeoff this implies, and
ordered migration slices — read it before touching `move_cube`,
`update_wad_doors`, or projectile-hit logic.

## App state and single-player

The app has a top-level `AppState` (`mainmenu.rs`): `MainMenu` (title
screen) → `InGame`. What used to be unconditional `Startup` systems in
`main.rs` (network setup, scene spawn, HUD, escape menu, ...) now run on
`OnEnter(AppState::InGame)`, and the corresponding `Update` systems are
gated with `.run_if(in_state(AppState::InGame))`. If you add a new
in-game-only system, gate it the same way — it won't error without the
gate (resources just won't exist yet), but it'll run one frame too early
relative to setup, or run over the main menu.

Single player (`singleplayer.rs`) is **not a separate game mode** — the
"Single Player" button spawns the current executable as a headless
`CUBE_AUTH_SERVER=1` subprocess and this process connects to it as an
ordinary `UntrustedClient` over `127.0.0.1`, using the same auth-server/
client split as real LAN multiplayer. `multiplayer::SinglePlayerConnect`
is the override resource that makes `multiplayer::setup_network` pick
loopback/`UntrustedClient` instead of reading env vars for role detection;
it's only inserted by `singleplayer::spawn_embedded_server`, so every other
launch path (LAN, Steam, headless server, direct `CUBE_AUTH_SERVER_ADDR`
join) is unaffected. `UntrustedClient` binds an ephemeral port (`0.0.0.0:0`)
rather than the well-known `SYNC_PORT` specifically so a client can share a
host with its own embedded server without a bind collision — don't
"simplify" that back to a fixed port. The embedded server subprocess is
killed via `EmbeddedServerProcess`'s `Drop` impl, triggered from
`singleplayer::shutdown_embedded_server` on `AppExit`.

## When uncertain

If a change spans both transports, both feature configs, or touches a file
in the untrusted-input list above, prefer asking or making the smallest
reversible change and running both `cargo check` variants before reporting
done.
