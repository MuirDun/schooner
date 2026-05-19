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

## License

Copyright 2026 Maksim Iakovlev

Licensed under the Apache License, Version 2.0 (the "License"); you may not use
this project except in compliance with the License. You may obtain a copy of
the License at

    http://www.apache.org/licenses/LICENSE-2.0

See the [LICENSE](LICENSE) file for the full text. Unless required by applicable
law or agreed to in writing, software distributed under the License is
distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND,
either express or implied.
