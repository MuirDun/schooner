# Schooner

A game engine written from scratch in Rust, targeting an open-world RPG with emergent living-world simulation.

The engine is built progressively through a series of small games (Games 0–5), each adding one major new dimension of complexity. See [`plans/plan.md`](plans/plan.md) for the full roadmap.

## Workspace layout

```
schooner/
├── Cargo.toml              # workspace root
├── crates/
│   ├── schooner-engine/    # engine library (lib crate)
│   └── game/               # active Game
└── plans/                  # design docs and phase plans
└── games/                  # previously done games
```

## Running Game

```sh
cargo run -p game
```

## Checking the engine without running

```sh
cargo check -p schooner-engine
```

## Platforms

Windows, Linux, macOS. wgpu selects the native graphics backend (DX12, Vulkan, Metal).
