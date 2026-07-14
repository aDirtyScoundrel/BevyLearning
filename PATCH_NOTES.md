# Patch Notes

## v0.3.1 - 2026-07-14

- Split authoritative networking into explicit auth-service and game-service packet paths for both LAN and Steam transport runtimes.
- Added Steamworks transport intent routing: reliable auth control packets and low-latency unreliable gameplay packets.
- Added Steam packet-level observability counters for auth, token validation, replay rejection, and peer mismatch rejection paths.
- Added in-game Steam metrics overlay with `F5` toggle and periodic `[steam-metrics]` terminal summaries.
- Expanded multiplayer docs with beginner-friendly setup flow, 60-second sanity checks, and single-machine two-terminal quick start.

## v0.2.2 - 2026-07-13

- Added headless server mode: run with `CUBE_AUTH_SERVER=1` to start authoritative server without rendering or UI.
- Made `HudState` and `PlayerInputIntent` optional in multiplayer network systems so server mode avoids UI resource panics.
- Split `main()` into `run_headless_server()` (MinimalPlugins) and `run_client()` (full game app) to isolate server-only dependencies.

## v0.2.1 - 2026-07-13

- Fixed conflicting `Query` filters in `animate_walk_cycle` by adding `Without<ChickenLeg>` and `Without<ChickenBody>` constraints to disambiguate the body and leg queries.
- Rewrote README to reflect current game state: chicken player, 3D movement, seed projectiles, freeze mechanic, authoritative server architecture, and in-game Steam server browser.

## v0.2.0 - 2026-07-11

- Added explicit auth-server and untrusted-client networking flow for LAN and Steam paths with token-gated input ingress.
- Added shared auth/input/snapshot codec module with packet roundtrip tests to keep transport packet layouts aligned.
- Added dedicated authoritative server tick integration with config-driven movement tuning and client-side reconciliation toward snapshots.
- Added in-game Steam lobby-backed server browser with refresh, selection, and join controls plus host lobby advertisement metadata.

## v0.1.3 - 2026-07-11

- Removed transport-specific send branching from gameplay systems by routing projectile and freeze broadcast through shared runtime helpers.
- Extracted shared remote runtime maintenance for projectile draining, departure cleanup, and stale-state pruning across LAN and Steam transports.
- Added packet contract matrix tests for malformed freeze/projectile headers, leave-packet trailing data rejection, and state color clamping.
- Added deterministic LAN ingress harness tests for join/state/leave lifecycle and projectile dedup plus freeze delivery behavior.

## v0.1.2 - 2026-07-10

- Updated the chicken beak to use a cone mesh and adjusted its transform for clearer visibility.
- Added vertical centerline alignment so the beak stays inline with the head framing.
- Increased forward offset so the beak protrudes consistently in gameplay camera views.
- Fixed state packet leave-message encoding to match strict header-only leave decode behavior across shared/LAN/Steam paths.

## v0.1.1 - 2026-07-10

- Shared the LAN and Steam packet codec logic in a single module so both sync paths stay aligned.
- Added inbound validation for networked transforms: reject non-finite floats and normalize decoded quaternions.
- Pruned projectile dedup state so remote projectile tracking no longer grows without bound.
- Hardened GameInfo serialization to reject player counts above 15 instead of silently truncating them.
- Added end-to-end codec tests for state, freeze, and projectile packet roundtrips plus malformed input cases.

## v0.1.0

- Initial packaged Linux release.