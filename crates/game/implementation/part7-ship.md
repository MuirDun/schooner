# Part 7 — Ship

**Kind:** Polish, performance, mix, release
**Status:** Not started
**Depends on:** Part 6 (Second Half) complete

---

## Goal

Kinesis ships. By end of Part 7, the game runs at target frame rate on all three target platforms (macOS, Linux, Windows), the audio mix carries the design's atmospheric load across reference playback chains, hunger and attitude curves are tuned against real play-test data rather than developer feel, no shippable-bug list remains, and the release artifacts exist on the storefronts the game launches to. The playground binary is dropped from the shipped build. The `crates/game/` tree is ready to freeze to `games/1-kinesis/` per the engine roadmap's archival pattern.

## The question this Part answers

**Is it shippable?**

Not "is it done" — done is Part 6. Shippable is the harder bar: does it perform, does the mix translate, does the pacing survive contact with players who didn't write it, do the cross-platform builds actually run on machines we don't own. Part 7 exists because pretending shipping is a final week is how solo games slip into infinite delay.

## In scope

- Performance pass: profiling on representative scenes (the densest flight chamber, the labyrinth, the food-storage stealth sequence with multiple tentacles), targeted optimizations until target frame rate holds on the minimum-spec configuration
- Audio mix pass: ambient bed levels, vocalization presence, positional attenuation curves, the instrument's musical weight in Act 4 vs Epilogue A1, the death sequence's audio muffling timing
- Play-test tuning: hunger curves per act, attitude input weights, death thresholds — adjusted against outside play-test data rather than the developer's calibration
- Cross-platform build verification: real runs on macOS, Linux, Windows (the Game-0 CI matrix has been running `cargo check`; Part 7 is where actual playable builds get tested per-platform)
- Bug fix pass on the shippable-bug list accumulated across Parts 5–6 and play-tests
- Playground binary removal from the shipped build (drop the `--bin playground` target or feature-flag it out at release)
- Release artifact preparation: storefront pages, screenshots and trailers, store-page copy, build-uploading workflow, ratings submissions where required
- Final cross-check that no design-violation drift crept into the late Parts (no Mahli writing in the world, no companionship subverting the loneliness rule, no music outside the two instrument scenes)

## Out of scope

- Any new gameplay system, any new tech, any new content. Cuts are allowed; additions are not.
- Localization beyond the in-scope strings already authored (menu replacement and ending texts in RU + EN per `design/acts/endings.md`). Additional language localization is a post-launch decision.
- Post-launch content / DLC planning — Kinesis is a complete short novelette; a sequel or expansion is not on this project's roadmap
- Migration of `crates/game/` to `games/1-kinesis/` — that archival step happens *after* ship, when Game 2A's planning begins
