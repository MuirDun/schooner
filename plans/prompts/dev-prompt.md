# Current Game Development Prompt

Use this prompt for implementation sessions on the active game in Schooner. Durable project rules live in `AGENTS.md`; this file defines the development-session rhythm.

## Goal

Implement the active game's current Part, Phase, and Step in small, reviewable increments. The work should advance the game while preserving the engine direction described by the roadmap, architecture docs, and current code.

## Startup

At the start of a development session:

1. Read `AGENTS.md`.
2. Read `crates/game/plan.md`.
3. Read the current Part doc under `crates/game/implementation/` for the requested Phase.
4. Read `crates/game/development.md` if it exists.
5. Read only the architecture and design docs directly relevant to the requested Phase.
6. Inspect the relevant engine and game source files.

Do not load the entire project by default. If the requested Phase is ambiguous, first identify the likely Part doc and ask for confirmation.

After startup, report:

- current game, Part, Phase, and last completed Step
- the Part milestone question, if the Part doc defines one
- the next Step or the smallest sensible implementation slice
- files read for context
- any discrepancy between plan, design, architecture, and code
- whether enough context has been loaded to discuss implementation

Then stop and wait for confirmation before planning or editing, unless the user's prompt explicitly grants permission to continue.

## Before Implementation

Before editing code:

1. Explain the proposed implementation approach.
2. Name the files or modules likely to change.
3. Call out any API, data model, scheduling, rendering, serialization, or dependency decision that is hard to reverse.
4. Recommend a path when there are multiple reasonable options.
5. State the intended verification command or manual playtest.

Wait for approval if the user asked for a checkpointed session, if the plan is unclear, or if the approach changes project/design/architecture intent.

## Work Size

Work one Step at a time. If a documented Step is still too large, split it into smaller slices such as:

- one module
- one public type or resource
- one system and its tests
- one renderer pass
- one debug overlay/control
- one focused integration point

Do not batch an entire Phase without check-ins.

## Check-Ins

At each implementation check-in:

1. Say what slice is being changed and why it comes next.
2. Make the code/doc changes.
3. Ask the user to run the verification task, following `AGENTS.md` and the session prompt.
4. Summarize what changed, what was learned, and what remains.
5. If verification fails, diagnose from actual output before changing more code.

When a slice creates something visible or interactive, propose a short experiment or playtest before continuing. Skip this for pure plumbing, refactors, and invisible infrastructure.

## Phase Boundaries

When a Phase is complete:

1. Confirm every Step and Done criterion for that Phase.
2. Update Step or Phase checkboxes in the relevant Part doc.
3. Add a short implementation note only if it records completed reality useful to later work.
4. Summarize what was built and what affects later Phases or Parts.
5. Ask whether to close the Phase and move to the next one.

Updating progress markers is allowed. Rewriting Phase scope, milestone questions, design facts, or architecture direction is not allowed without a prior planning conversation and explicit approval.

## Part Boundaries

When a Part is complete:

1. Verify the Part Done Bar, including subjective criteria.
2. Run or request the Part's regression surface.
3. Confirm whether the Part's milestone question now has a confident answer.
4. Update Part status/checklists in `crates/game/plan.md` and the Part doc.
5. Summarize what was built and what affects later Parts or games.
6. If the work changes the engine's understood current state, propose an architecture or overview update as a separate planning step.
7. Ask whether to close the Part and move on.

Do not edit `plans/architecture/`, `plans/overview/`, or design canon as part of closing a Part unless the user has explicitly approved that documentation change after discussion.

## Decision Policy

Ask when:

- two or more reasonable options have different tradeoffs
- the decision is hard to reverse
- code reality conflicts with the plan
- a dependency is needed
- the requested change would alter design, architecture, roadmap, or phase scope

Do not ask about:

- routine Rust idiom
- local variable names
- doc-comment wording
- trivial formatting
- private implementation details behind an agreed public shape

When asking, state the options, recommendation, and reasoning first.

## Explanation Policy

Explain game-engine reasoning, not basic Rust mechanics. Go deeper for:

- rendering and GPU pipeline choices
- ECS data layout and scheduling tradeoffs
- simulation, world state, AI, and reactivity architecture
- scripting and language-binding boundaries
- input, physics, camera, and gameplay feel

Use industry terms when useful so the developer can research further. Mention alternatives when they clarify why the chosen path fits the current game.

## Things To Resist

- scope creep beyond the active Phase
- future-game infrastructure unless it is needed now and does not distort the current work
- refactoring working code just because it is nearby
- silent plan reinterpretation
- guessing at compiler errors
- changing frozen games
- treating architecture/design documents as implementation scratchpads
