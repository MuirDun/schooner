# Game 0 — Cross-Platform Verification Protocol

> Run on each target OS once. Marks the Phase I "Verify `cargo run -p game-void` on Windows, Linux, macOS" item complete. macOS has been verified continuously throughout development; the remaining work is Linux + Windows.

---

## Per-OS prerequisites

### macOS
- Recent Xcode command-line tools (`xcode-select --install`).
- No further setup — Metal is the wgpu backend and is provided by the OS.

### Linux (Ubuntu / Debian)
- `sudo apt install libxkbcommon-dev libwayland-dev libxkbcommon-x11-dev libx11-dev` — winit's keyboard layer + both backends. The CI workflow installs the same set.
- A Vulkan loader (`mesa-vulkan-drivers`, or vendor's). `vulkaninfo` should print at least one device.
- An active display session (X11 or Wayland) — the binary opens a real window; headless servers need an `Xvfb` setup that is out of scope here.

### Windows
- Visual Studio Build Tools (the C++ workload) — Rust's MSVC toolchain links against it. `rustup-init` will warn if absent.
- A recent GPU driver. wgpu picks DX12.

---

## The protocol

Run on the OS being verified, from a clean clone or a fresh `cargo clean`:

```sh
cargo run -p game-void
```

A 1280×720 window titled **"Schooner — The Void"** opens. Verify each item below; tick on success, capture a screenshot or log line on failure.

### Visual checklist

- [ ] Window opens at 1280×720 with the expected title.
- [ ] A grey floor fills the lower half; a 3×3 grid of nine cubes stands on it.
- [ ] Lighting is visible — the cubes have a lit top face and shaded sides (Blinn–Phong, single directional light).
- [ ] The egui **Debug** overlay appears in the upper-left, showing FPS, frame ms, entity count = 12, camera position ≈ `(0.00, 1.70, 8.00)`.

### Input checklist

- [ ] Mouse-look turns the camera smoothly. Vertical look clamps before flipping.
- [ ] **W A S D** moves; **Space** ascends, **Ctrl** descends.
- [ ] **Esc** releases the cursor (visible again, free to move). Pressing again or clicking back into the window re-grabs.
- [ ] Alt-tab releases the cursor; refocusing re-grabs.
- [ ] **F1** hides the debug overlay; **F1** again shows it.

### Profiler checklist

- [ ] Tick the **Profiler** checkbox in the overlay. A scope table appears.
- [ ] Top-level row is `frame`; indented children include `update_stage`, `render_stage`, `render_frame`, etc.
- [ ] Numbers update about twice per second (not every frame). They're readable, not jittering.
- [ ] Untick **Profiler** — the table disappears, the rest of the overlay stays.

### Graceful behavior checklist

- [ ] Resize the window (drag a corner). The scene reflows to the new aspect ratio. No crash, no flicker beyond the resize itself.
- [ ] Minimize and restore the window. No crash. The render resumes.
- [ ] Close the window via the OS close button. The process exits cleanly (no panic in the log).

### Log sanity

- [ ] Run with `RUST_LOG=schooner_engine=info cargo run -p game-void`. The log shows `starting event loop`, `window created: 1280x720`, `selected adapter: …`, `render context ready: 1280x720 …`. No panics, no warnings beyond the expected `surface lost/outdated; reconfiguring` on resize (and only on macOS).

---

## On failure

For any failed item, capture:
1. The OS + GPU + driver version (`wgpu_info` if you have it; otherwise vendor + chip + driver from the OS's display settings).
2. The full stderr log with `RUST_LOG=info`.
3. A screenshot if it's visual.

File against the repo issue tracker. Do not paper over by `cargo run --release` or by changing the wgpu backend selection — Game 0's done bar is "default `cargo run` on a clean clone works on all three desktop OSes," and any divergence is a real bug.
