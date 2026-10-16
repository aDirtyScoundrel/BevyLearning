# Classic map implementation status

Checked on 2026-10-10. Chicken Gun is not yet multiplayer-ready with original-engine map compatibility.
This file supersedes implementation claims in `CLASSIC_MAP_SUPPORT.md`.
That file remains a historical plan, not a compatibility report.

## Scope

Doom-family and Marathon/Aleph One maps are both in scope.
The importers remain separate. Marathon data must retain native polygon ownership.
The selected movement model uses local prediction. The server controls collisions, hits and final positions.
This architecture is the target, not a claim that all authority and reconciliation work is complete.

## Tested implementation

- LAN identity, authentication retries, sender checks and snapshot size limits have regression tests.
- Input validation rejects non-finite movement axes. Menus clear movement intent.
- Firing queries select the local chicken hierarchy. Live firing feedback still needs acceptance tests.
- UDMF parsing handles namespaces, quoted delimiters and comments.
- Authentication binds the configured map name and contents to the session proof.
- The headless Doom server loads map collision before the network starts.
- Server movement uses wall sweeps, sector floor heights and separate co-op starts.
- The native Marathon parser has synthetic, malformed-input and real-map tests.

These checks do not establish full map-change synchronisation, reliable snapshot ordering or live Steam compatibility.
The special dispatch table still needs a complete original-engine reference audit.

## Sector surfaces

`src/map_surfaces.rs` makes bounded floor and ceiling meshes from unordered closed boundaries.
Horizontal strips preserve concave outlines, holes and separate islands.
The helper rejects open boundaries, crossings within strips, non-finite values and excessive work.
Floors face upwards. Ceilings face downwards. Texture coordinates retain world alignment.

`src/doom_wad.rs` prepares all sector meshes before it creates map entities.
The renderer uses each sector's floor and ceiling heights instead of a single flat floor.
Sky flats leave openings for the sky. WAD sector surfaces now use indexed flats with the base PLAYPAL palette.
Wall materials and unsupported flat resources still use placeholder colours. Sector lighting remains incomplete.
Floors and ceilings are static. Door, lift and other sector movers must update them in a later step.

Limits are explicit:

- Each sector permits at most 8,192 edges and 65,536 triangles.
- Each sector permits at most 4,000,000 edge and height-level scan pairs.
- Each map permits a conservative scan estimate of 16,000,000 and 1,500,000 surface vertices.

A map that exceeds these limits returns an error. Larger-map compatibility needs separate tests and profiling.

## Camera collision

`src/map_camera.rs` shortens the third-person camera offset against the current Doom collision world.
The helper checks wall capsules, door collision state, sector boundaries, floors and ceilings.
It clamps the pivot height and keeps clearance for the perspective camera near plane.
The camera restores its normal offset when space clears. Noclip bypasses camera collision.
Camera updates follow movement, mouse input, position corrections and door updates.

The helper bounds edge counts, boundary cuts and combined sector-query work.
Excessive work returns the pivot instead of placing the camera beyond unchecked geometry.
The current door collision model remains an approximation. This does not add dynamic sector surfaces or Marathon camera support.
Tight-space avatar occlusion and broader map coverage still need visual checks.

## Vulkan startup workaround

`src/render_startup.rs` adds a clear pass before normal view rendering.
It clears an acquired window surface only when no writing camera has a ready output pipeline.
The pipeline key matches the installed Bevy 0.19 upscaler, including later-camera blending and compositing space.
Windows with ready output pipelines bypass the clear pass. Headless apps need no render resources.
Vulkan validation and asynchronous pipeline compilation remain enabled. Dependencies did not change.

The default arena reproduced `VUID-VkPresentInfoKHR-pImageIndices-01430` and `VUID-vkAcquireNextImageKHR-semaphore-01286`.
Synchronous pipeline compilation did not fix either error. That trial setting was removed.
The installed render path and the default control match the failure described in Bevy issue 22733.
This application workaround does not constitute an upstream renderer fix.
Recheck it after a Bevy or wgpu upgrade.

Seven graphical sessions with the workaround reported no Vulkan validation errors.
Each session used two initial clear passes. The repeated arena and MAP01 checks covered both feature configurations.
All sessions used LAN transport. These checks do not establish live Steam compatibility.
The first repeated LAN launch lacked a display environment and failed before rendering.
Its record remains in the evidence directory. The corrected runner rejects a missing display and premature client exit.

## Executed checks

Both feature configurations passed:

- 54 library tests and 114 game tests.
- One library reference test and six game reference tests, explicitly selected with `--ignored`.
- Surface checks for all 68 Freedoom 0.13.0 maps.
- Native Marathon Infinity parsing through the real-map reference test.

Surface tests check area, holes, separate islands, triangle direction, normals, heights, texture coordinates and invalid input.
The Freedoom check also verifies triangle centroids against sector containment.
The MAP01 entity test compares spawned surface attributes with the prepared meshes.
The camera reference test checks 96 rays across four MAP01 starts, eight yaw angles and three pitch angles.
All rays passed height and wall-clearance checks. Collision shortened 91 rays.
An ECS test verifies parent transforms, camera retraction, camera extension and noclip bypass.

The earlier sector-surface build also passed a four-client UDP probe on Freedoom MAP01.
Every client received all four distinct co-op spawn positions.
The probe stopped the server and confirmed port release.

## Live visual findings

A graphical LAN client rendered Freedoom MAP01 on the discrete Vulkan device.
The screenshot shows sector geometry, but this is not an original-engine visual comparison.
The earlier sector-surface run placed the camera above low ceilings and exposed the map exterior.
The camera-collision run shows the camera inside the room, behind the chicken.
Overlapping interface panels and placeholder materials remain visible.
The earlier startup logs contain Vulkan swapchain layout and semaphore validation errors.
The default-map control reproduced both errors without imported map geometry.
The startup workaround below removes these errors in the checked launches.
The final screenshot shows MAP01 geometry and the chicken after the initial black startup frame.
The inspection stopped both temporary processes and confirmed port release.

## Texture verification on 2026-10-10

Resumed the interrupted flat-texture inspection without changing game source.
Both full test configurations and their reference tests passed again.
The graphical MAP01 client decoded 28 of 28 requested flat textures on the discrete Vulkan device.
The screenshot shows the chicken and textured floor. Walls retain placeholder colours, and interface panels overlap.
The ceiling is outside this view. This check does not establish ceiling appearance or original-engine fidelity.
The client log contains no Vulkan validation errors. Both temporary processes stopped and released UDP port 34567.

Evidence: `.hermes/verification/doom-flats-resume-2026-10-10/`.
`manifest.json` records source hashes, the executable hash, reference results and process cleanup.
`test-results.json` records the full test results. `map01.png` records the inspected view.
The earlier premature-client-exit record remains in `.hermes/verification/doom-flats-2026-10-09/`.

## Next work

1. Check camera behaviour in more maps and very tight spaces.
2. Check ceiling textures visually. Add original wall textures and sector lighting.
3. Implement server-controlled sector movers, triggers, keys, projectiles and hits.
4. Connect the Marathon importer to native rendering, collision and session selection.
5. Complete map-change synchronisation and prediction reconciliation.
6. Test rendered maps against their original engines and run live Steam sessions.

## Reproduce

Run these commands from the project directory:

```sh
cargo test --locked --offline -j 2
cargo test --locked --offline --no-default-features -j 2
CHICKEN_REFERENCE_DIR="$PWD/.hermes/verification/reference-maps" cargo test --locked --offline -j 2 real_ -- --ignored --nocapture
CHICKEN_REFERENCE_DIR="$PWD/.hermes/verification/reference-maps" cargo test --locked --offline --no-default-features -j 2 real_ -- --ignored --nocapture
```

The local reference files and verification records are outside version control.
A new checkout must supply the reference files before it runs the ignored tests.

Local evidence directory:
`.hermes/verification/sector-surfaces-2026-10-09/`

- `results.json` records the executed commands, exit codes and test summaries.
- `sector-surfaces.tsv` contains one checked row per Freedoom map.
- `sector-render-map01.png` contains the game window from the visual inspection.
- `render-client.log` retains the Vulkan errors.
- `live-udp.log` contains the four-client result from the final build.
- `reference-maps/manifest.json` in the parent directory records reference-file hashes and source URLs.

Latest camera evidence:
`.hermes/verification/camera-collision-2026-10-09/`

- `results.json` records the final test, reference, check and build results.
- `manifest.json` records final source hashes, the executable hash and the graphical session state.
- `camera-map01.png` shows the full game window before the final query-work guard.
- `final-camera-visible-region.png` shows the visible part of the final build. Another desktop window obscured the rest.
- `render-client.log` retains the recurring Vulkan errors.
- `render-session.json` records process exit and port release.
- `before-work-limit/` preserves the earlier test and graphical records.

Latest Vulkan startup evidence:
`.hermes/verification/vulkan-startup-2026-10-09/`

- `manifest.json` records source hashes, the executable hash, validation IDs and final process cleanup.
- `test-results.json` records both feature configurations and the reference tests.
- `runtime-results.json` retains the failed display-less launch and four successful repeats.
- `baseline-default/`, `synchronous-default/` and `trace-default/` retain the negative controls.
- `final-map01-visual/map01.png` shows the complete game window after startup.
- `final-map01-visual/startup.png` retains the earlier black startup frame with visible interface panels.
- `local-draft/` retains the local model request and uncorrected answer.

The retained Python probes contain this workstation's paths. They are inspection tools, not release tooling.
