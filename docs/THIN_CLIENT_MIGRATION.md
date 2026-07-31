# Thin-Client Migration Plan

Goal (stated by project owner): as little actual game logic as possible
should run on the client. The client should, in effect, be a renderer +
input relay to the authoritative server — not a second physics simulation
that gets corrected after the fact.

This is a planning document. Update it as slices land; don't let it drift
from what `main.rs`'s system schedule actually does.

## Progress

**Slice 1 — LAN only, done.** The LAN auth-server host's own player now
goes through `server_tick::step_authoritative_sim` exactly like a remote
player:
- `move_cube` (`controls.rs`) checks `NetworkSync::is_auth_server()` and
  skips writing gravity/plane physics onto the rendered transform when
  true — it still samples input into `PlayerInputIntent` and still runs
  noclip locally (deliberately excluded, see doc comment).
- `send_local_state`'s `AuthServer` branch (`multiplayer.rs`) now inserts
  the host's own `PlayerInputIntent` into `authoritative_inputs` before
  calling the sim, keyed by the host's own player id.
- A new system, `multiplayer::apply_local_sim_result`, copies the sim's
  result for the host's own id out of `remote_states` back onto the
  rendered `RotatingCube` transform, `.after(send_local_state)`.
- `sync_remote_cubes` skips spawning a duplicate `RemoteCube` visual entity
  for the host's own player id (it now legitimately appears in
  `remote_states` but renders through the local player entity, not a
  remote one).

**Not done yet (tracked as follow-ups, not slipped into slice 1):**
- **Steam host parity.** `NetworkSync::is_auth_server()` only exists on the
  LAN `multiplayer::NetworkSync` type; `steam_mp::SteamSync`'s host role
  has no equivalent guard yet, so a Steam-hosted game's own player still
  runs local physics unconditionally. Mirror the same three changes
  (`move_cube` guard extended to check Steam host status too, host input
  inserted into `steam_mp`'s `authoritative_inputs`, an
  `apply_local_sim_result`-equivalent for the Steam path) — matches this
  project's existing LAN/Steam duplication pattern (see AGENTS.md).
- **Client-side prediction (slice 3).** Right now the host's own player
  has zero input lag because sim + render happen in the same process on
  the same tick (no real network round-trip) — this slice doesn't yet
  prove out the prediction/reconciliation model needed for a *connecting*
  client's local player, which does have real round-trip latency. That's
  still open work, not implicitly solved by slice 1.
- Slices 2 (server-side doors) and 4 (server-side projectile hits) are
  unstarted, as originally scoped.

## Current architecture (verified against `main.rs`, `controls.rs`,
`server_tick.rs`, `doom_wad.rs`)

There **is** a real authoritative-server sim already:
`server_tick::step_authoritative_sim` (LAN) and
`steam_mp::host_step_authoritative_sim` (Steam) advance movement, gravity,
jump, and plane-bound clamping for every *remote* player from validated
input samples, then broadcast snapshots. This part already matches the
goal.

The gap is the **local player**. Every client — including the one that
*is* the auth server, and every connecting client — runs the full movement
system locally every frame:

- `controls::move_cube` — reads keyboard, integrates gravity/jump/movement,
  clamps to plane bounds, and resolves WAD collision (`wad_collision.resolve_position`)
  entirely client-side. This is real physics, not prediction scaffolding —
  there is no separate "predicted" vs "authoritative" local state; `move_cube`
  writes directly to the rendered `Transform`.
- `doom_wad::update_wad_doors` — reads the *local* player's transform
  (`player_query.single()`), decides door open/closed state, and mutates
  door transforms — entirely client-local, entirely per-client. Two players
  near the same door compute independent, unsynced door states (flagged
  in the original code-review action plan and still true after the special
  dispatch table work).
- `scene::update_cone_projectiles` / `resolve_projectile_collisions` — local
  projectile physics and hit detection run per-client; only a spawn
  notification and (for confirmed hits) a freeze-target message go over the
  wire (`remote_runtime::broadcast_projectile_spawn`/`broadcast_freeze_target`).
  Whether a shot actually hits is decided locally by the shooter's client,
  not the server.
- Reconciliation (`multiplayer::apply_local_reconciliation`,
  `steam_mp::apply_local_reconciliation`) only lerps the *remote* players'
  rendered transforms toward server snapshots — it does not correct the
  local player toward anything, because the server never computes the local
  player's authoritative position in the first place. There's no server
  state to reconcile toward for the client's own position.

In short: remote players are already server-authoritative. The local
player is not — each client is trusted to compute its own truth.

## Target end state

1. Every client sends only **input** (movement axes, jump, look angle,
   interact/use, fire) to the server — no client ever computes its own
   authoritative position, door state, or projectile-hit outcome.
2. The server runs one simulation per connected player (already exists for
   remote players via `server_tick`/`host_step_authoritative_sim`) — extend
   it to include the *locally-hosted* player too, so the host client's own
   player goes through the same authoritative path as everyone else's,
   rather than being special-cased as "trusted local."
3. Door/trigger state (`WadCollisionWorld.doors`), once item 2 of
   `docs/CLASSIC_MAP_SUPPORT.md` lands, becomes server-owned: the server
   evaluates proximity/walk/switch/shoot triggers using *its own*
   authoritative player positions, and broadcasts door open/closed state
   in the snapshot. Clients never decide door state locally.
4. Projectile hit resolution moves server-side: clients send "fired
   projectile with this origin/direction/timestamp," the server simulates
   the projectile and determines hits, and broadcasts confirmed hit/freeze
   events. Clients render projectiles predictively for feel, but the
   *authoritative* hit outcome comes from the server.
5. Collision against WAD geometry (`WadCollisionWorld::resolve_position`)
   runs on the server for every player's authoritative position. Clients
   may still run it locally for **client-side prediction only** (to avoid
   visible rubber-banding against walls) but the server's result is what
   actually persists and gets broadcast — client prediction must never be
   allowed to diverge into "the truth."

## Why this is bigger than a drop-in fix

The server currently only simulates *remote* players because
`authoritative_inputs`/`remote_states` are populated from *other* clients'
input packets — the locally-hosted player was never added to that map,
because it was assumed to be "the trusted local player." Making the local
player go through the same authoritative path means:

- The host's own client needs to submit input packets to its own embedded
  server (or the embedded server needs to read local input the same way it
  reads remote packets) instead of calling `move_cube` directly on the
  rendered transform.
- `move_cube`'s role on every client shrinks to: sample input, clamp axes,
  send an input packet. It stops touching `Transform` for the local player
  at all — the rendered position instead comes from the server's snapshot,
  same as remote players, just with a shorter round-trip (loopback) for the
  host.
- This reintroduces the classic "no client-side prediction = visible input
  lag" problem that `apply_local_reconciliation` currently sidesteps by
  letting the client be its own truth. A real thin-client architecture
  needs **client-side prediction with server reconciliation for the local
  player too** (predict locally for responsiveness, snap/lerp to the
  server's authoritative answer when it disagrees) — not just "stop
  running physics locally," or input will feel terrible over any real
  network latency. This is a deliberate design tradeoff to flag, not an
  oversight: less "client trusted as truth," but not zero client-side
  computation, since some client-side prediction is required for playable
  net-code. Confirm this tradeoff is acceptable before deep work starts —
  it changes the reconciliation model, not just where physics runs.

## Suggested slices, ordered by risk/size

1. **Server accepts the host's own input as just another input packet.**
   Smallest concrete step: on the host, loop the local player's input
   through the same encode/decode path used for remote input packets
   (`encode_input_packet`/`decode_input_packet` — already shared code)
   instead of calling `move_cube`'s movement math directly. This makes
   `server_tick::step_authoritative_sim` cover the host player too, with no
   protocol changes — same packet types, same auth flow, just also applied
   to the process's own player. Verify: host player's position should now
   come from `remote_states`/snapshot reconciliation exactly like every
   other player, not from a locally-mutated `Transform`.
2. **Door state moves server-side** (depends on
   `docs/CLASSIC_MAP_SUPPORT.md` item 2's activation model landing first —
   don't build this against proximity-only triggering, or it'll need
   rework). Server evaluates door triggers against its authoritative
   player positions; add door open/closed state to the snapshot packet;
   `update_wad_doors` on the client becomes purely visual (interpolates
   toward server-reported open_amount, doesn't decide open/closed itself).
3. **Client-side prediction for the local player**, once slice 1 exists —
   predict locally using the same movement math the server uses (ideally
   the exact same function, not a re-implementation, to avoid the
   duplication class of bug already flagged for LAN/Steam parity), then
   reconcile/snap when the server's answer disagrees. This is what makes
   slice 1 not feel laggy.
4. **Projectile hit resolution moves server-side** — biggest slice, touches
   `scene.rs`'s projectile spawn/update/collision systems and the
   projectile-spawn network messages. Do this last; it's the most
   gameplay-visible change and benefits from slices 1-3 already proving out
   the server-authoritative-local-player pattern.

## What NOT to move to the server

Pure presentation has no business on the server and should stay client-side
forever: camera orbit/pitch (`mouse_look`, `follow_cube_camera`), wing-flap/
walk-cycle animation (cosmetic, driven by already-synced position/velocity),
UI/HUD rendering, WAD/PK3 file picker UI. Don't over-apply "move it to the
server" — the target is authoritative *game state*, not literally every
system.
