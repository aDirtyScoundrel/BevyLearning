# Patch Notes

## v0.1.2 - 2026-07-10

- Updated the chicken beak to use a cone mesh and adjusted its transform for clearer visibility.
- Added vertical centerline alignment so the beak stays inline with the head framing.
- Increased forward offset so the beak protrudes consistently in gameplay camera views.

## v0.1.1 - 2026-07-10

- Shared the LAN and Steam packet codec logic in a single module so both sync paths stay aligned.
- Added inbound validation for networked transforms: reject non-finite floats and normalize decoded quaternions.
- Pruned projectile dedup state so remote projectile tracking no longer grows without bound.
- Hardened GameInfo serialization to reject player counts above 15 instead of silently truncating them.
- Added end-to-end codec tests for state, freeze, and projectile packet roundtrips plus malformed input cases.

## v0.1.0

- Initial packaged Linux release.