# Schooner

A game engine written from scratch in Rust, targeting an open-world RPG with emergent living-world simulation.

The engine is built progressively through a series of small games (Games 0–5), each adding one major new dimension of complexity. See [`plans/plan.md`](plans/plan.md) for the full roadmap.

## Current milestone — Game 0: The Void

Engine bootstrap. A walkable 3D scene with a sparse-set ECS, wgpu forward renderer, first-person camera, egui debug overlay, and puffin profiling. Design in [`plans/game0-plan.md`](plans/game0-plan.md).

## Workspace layout

```
schooner/
├── Cargo.toml              # workspace root
├── crates/
│   ├── schooner-engine/    # engine library (lib crate)
│   └── game-void/          # Game 0 binary
└── plans/                  # design docs and phase plans
```

## Running Game 0

```sh
cargo run -p game-void
```

## Checking the engine without running

```sh
cargo check -p schooner-engine
```

## Platforms

Windows, Linux, macOS. wgpu selects the native graphics backend (DX12, Vulkan, Metal).
