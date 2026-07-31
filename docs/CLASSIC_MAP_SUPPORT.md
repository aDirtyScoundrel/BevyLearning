# Classic Map Compatibility — Action Plan

Design goal: this engine should be able to load and play open-source /
freely-distributable maps from classic id-Tech-lineage games **as-is, with
no modifications to the map data**. Doom (and Doom II / Boom / UDMF-flavored
community maps) is the primary target now; Marathon-style maps are an
explicit secondary target, tracked as a separate future extension so Doom
support isn't blocked waiting on it.

This is a planning document, not a changelog — nothing below has landed yet.
Update this file as items are completed or scope changes.

## Current state (verified against `src/doom_wad.rs`)

**Progress so far:**
- **Item 1 (linedef special dispatch table) — done for all vanilla Doom categories.**
  `decode_linedef_special()` now decodes all vanilla Doom special IDs across all categories:
  - Doors (1, 26-28, 31-34, 117, 118) — cross-checked against DoomWiki
  - Floors (18-19, 36, 56, 60, 64) — walk-triggered platforms
  - Lifts (62-63, 66) — raising/lowering platforms
  - Ceilings (40-42) — ceiling movers
  - Stairs (7-8) — staircase builders
  - Teleporters (39) — TAG-linked sector teleports
  - Exits (11, 51) — map exits (normal + secret)
  - Light effects (2-4) — flashing/strobe/flicker
  
  Carries `category`/`activation`/`repeatable`/`required_key` metadata for all types.
  Activation is decoded but **not yet enforced** — see item 2. Tests added for all categories
  to catch regressions. Anything not in the table returns None (falls through to old heuristics).

- **Item 4 (multi-spawn resolution) — data layer done, network wiring not
  done.** `find_player_spawns()` collects *all* spawn `Thing`s in the
  winning priority tier (was: first match only); `find_player_spawn_for_slot()`
  picks one by player-slot index with wraparound. `spawn_level_from_wad`
  still only calls slot 0 — assigning real per-player slots through the
  join handshake in `multiplayer.rs`/`steam_mp.rs` is unstarted.
- **`WadDoor.required_key`** now carries the decoded key requirement, but
  nothing reads it yet — no inventory exists, so locked doors still behave
  identically to unlocked ones at runtime (item 3 unstarted).

What already worked before this pass:
- Classic WAD binary lumps: `VERTEXES`, `LINEDEFS`, `SIDEDEFS`, `SECTORS`,
  `THINGS` — enough for basic vanilla-Doom level geometry.
- UDMF `TEXTMAP` parsing (community-format maps), for both standalone WADs
  and PK3/zip-embedded entries.
- PK3/zip container loading, including one level of nested-archive recursion
  (depth-capped) and a per-entry 64 MiB size cap.
- A door system, but it is **not real Doom door logic** — doors now come
  from a real special-ID decode (`decode_linedef_special`, see progress
  above) but still open purely on player proximity
  (`DOOR_TRIGGER_DISTANCE`), not real walk/switch/shoot activation, and
  locked doors are decoded but not gated on key possession.
- No keycard/inventory system at all — `WadDoor.required_key` is decoded
  but unused; locked-door specials still behave identically to unlocked
  ones at runtime.
- Spawn resolution now finds all spawns in a map (see progress above) but
  network-side per-player slot assignment doesn't exist yet — every
  player still effectively spawns at slot 0.
- Collision is a linear scan over all wall segments + doors per player per
  frame — no spatial partition.

What "play as-is with no modifications" actually requires that's missing:
- Doom's **linedef special system** as a real dispatch table, not a
  door-shaped-geometry heuristic. Doors are one category of ~15+ special
  categories (floors, ceilings, lifts/platforms, stairs, teleporters,
  scrolling textures, light effects, exits) that a real level uses.
- **Sector special effects** (damage floors, light flicker/strobe/glow,
  secret sectors) — parsed already via `SECTORS` lump fields not yet fully
  consumed for gameplay.
- **Line activation types**: walk-over (crossing the line), switch
  (use/interact), shoot (hitscan/projectile trigger), and repeatable vs.
  one-time variants. Currently everything is proximity-only.
- **Keycards**: `THINGS` kind IDs for red/blue/yellow key + skull key
  pickups, a per-player inventory, and gating locked specials on possession.
- **Multi-player spawn points**: co-op start (`Thing` kind 4, one per
  player 1-4) and deathmatch starts (kind 11, many-per-map) need to resolve
  to N spawns assigned round-robin/by-slot, not a single global spawn.
- **BOOM/MBF extended specials** (generalized linedef types, extra sector
  effects) — very common in community WADs; vanilla-only special handling
  will silently no-op or misbehave on a large fraction of real-world maps.

## Action plan (Doom-style, ordered by dependency)

1. **Linedef special dispatch table.** Replace the ad-hoc
   `is_door_linedef`/`is_door_geometry_candidate` heuristic with a real
   `LinedefSpecial` enum decoded from `linedef.special` (Doom's numeric
   special IDs — see the DoomWiki "Linedef type" reference table), each
   carrying its activation type (walk/switch/shoot), repeatability, and
   effect category (door/floor/ceiling/lift/stair/teleport/exit/light).
   This is the foundation everything else hangs off — do this before
   keycards or triggers, since both depend on knowing *which* special a
   linedef has and how it's supposed to activate.

2. **Activation model.** Replace `DOOR_TRIGGER_DISTANCE` proximity-only
   triggering with real activation semantics:
   - Walk-over: player crossing the linedef (needs per-frame line-crossing
     test against player movement delta, not just distance).
   - Switch/use: player-facing interact (already have an interact-adjacent
     pattern for door triggering — extend it to check facing + specific
     switch texture rather than "any nearby line").
   - Shoot: hitscan/projectile impact on the linedef (ties into the
     existing projectile/hit-detection code in `scene.rs`).
   This generalizes to floors/ceilings/lifts, not just doors — the mover
   logic (`update_wad_doors`) should become a generic "sector mover" driven
   by special type, with door-open being one specific mover behavior.

3. **Keycard system** (depends on #1 for locked-special IDs):
   - Add key pickup `Thing` kinds (5=blue key, 6=yellow key, 13=red key,
     38/39/40=skull variants — confirm exact IDs against DoomWiki before
     implementing) to spawn logic alongside existing player-start handling.
   - Per-player inventory (bool set: has_red/has_blue/has_yellow, or a
     bitflag) — sync it the same way color/score are synced today
     (`RemoteState`/snapshot packets in `multiplayer.rs`/`steam_mp.rs`), so
     this is server-authoritative like everything else in this project's
     networking model, not client-trusted.
   - Gate locked-door specials (26-28, 32-34, and their switch/shoot
     variants) on inventory possession; on failed attempt, real Doom prints
     "You need a blue/red/yellow key" — worth matching for parity, at least
     as a log/HUD message.

4. **Multi-spawn resolution.** Extend `find_player_spawn` (or replace with
   a new function) to collect *all* matching spawn `Thing`s per priority
   tier, not just the first, and assign one per joining player by slot
   index. Needs to interact with the existing player-join flow in
   `multiplayer.rs`/`steam_mp.rs` — spawn assignment should happen
   server-side and be part of the authoritative join handshake, not decided
   independently per client.

5. **Sector specials** — damage floors (5/7/9/16 etc.), light effects
   (flicker/strobe/glow/blink), secret sector tracking. Lower priority than
   1-4 since most maps are still playable/recognizable without them, but
   "play as-is" isn't fully true until these exist. Do after the special
   dispatch table (#1) since sector specials share the same "decode a
   numeric ID into behavior" pattern.

6. **BOOM/MBF generalized specials.** A large fraction of community WADs
   (and nearly all "open source" ones you'd realistically load, since
   vanilla-only WADs are rarer in that category) use BOOM's generalized
   linedef types (special IDs in specific numeric ranges that encode
   trigger+speed+target via bit-packed fields rather than one ID per
   behavior). This should slot into the dispatch table from #1 as an
   additional decode branch once vanilla specials work — don't build #1's
   enum in a way that assumes only discrete named IDs, since generalized
   specials are numeric-range-based.

7. **Spatial partition for collision** (scaling concern from before,
   still applies here since real levels are large): grid-bucket
   `WallCollisionSegment`/`WadDoor` by cell so `resolve_position` stops
   doing an O(n) scan over the whole level every player every frame. Do
   this once real levels are actually loading via #1-#6, since that's when
   the segment count will get large enough to matter and you'll have real
   maps to profile against instead of guessing.

## Marathon-style maps — separate future extension

Marathon (Bungie, 1994-96, engine open-sourced as Aleph One / released
levels are open content) uses a **fundamentally different map format** from
Doom — not a variant, a different data model:
- Levels are stored in Macintosh resource-fork-derived `.sceA`/map-set
  files (or Aleph One's own container format), not WAD lumps.
- Geometry is polygon-based (arbitrary N-gon rooms with floor/ceiling
  height per polygon) rather than Doom's linedef/sector BSP model — no
  direct structural mapping from `ParsedMap`'s vertex/linedef/sector shape.
- Physics model differs: Marathon supports true room-over-room and
  variable gravity/liquid physics that Doom's 2.5D sector model can't
  represent at all.

**Do not try to shoehorn Marathon loading into the Doom parsing path.**
The right shape is a **separate loader function** (e.g.
`load_marathon_level_source(...)` or a new `marathon_wad.rs` module) that
produces the same `MapSource`/`ParsedMap`-equivalent output contract the
renderer/collision code already consumes, so the Bevy-side spawn/render
pipeline doesn't need to know which classic format it came from — only the
parsing front-end differs. This is a "when Doom support is solid" follow-up,
not something to start in parallel; the special-dispatch and activation-model
work in items 1-2 above is Doom-format-specific and won't transfer directly,
but the *pattern* (numeric type ID → behavior dispatch, server-authoritative
triggered state) should carry over conceptually.

Flagging now, not architecting yet: this file exists so "load open-source
maps from classic games" as a standing design goal doesn't get lost, and so
whoever picks up Marathon support later knows not to bolt it onto
`doom_wad.rs` directly.
