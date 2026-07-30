# Schooner dev task runner. `just --list` to see recipes.
# Dev-only for now — CI does not call these.

# Default: show available recipes
default:
    @just --list

# Dev build with hot reload (dioxus subsecond hotpatching)
serve:
    dx serve --hotpatch --features "hot dev-tools" -p game --bin playground

# Run the playground without hot reload
run:
    cargo run -p game --bin playground

# Run playground with all dev features enabled
try:
    cargo run -p game --features "hot dev-tools" --bin playground

# Validate the code of the project
validate:
    cargo check --workspace

# Build the whole workspace
build:
    cargo build --workspace

# Format all crates
fmt:
    cargo fmt --all

# Lint, treating warnings as errors
lint:
    cargo clippy --workspace --all-targets -- -D warnings
lint-fix:
    cargo clippy --workspace --all-targets --fix --allow-dirty

# purify
purify: fmt lint-fix

# Run the test suite
test:
    cargo test --workspace

# Pre-push gate: format, lint, test
check: fmt lint test

# Run the ECS benchmarks
bench:
    cargo bench -p bench-ecs
