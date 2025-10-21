<!--
Purpose: Short, actionable instructions for AI coding agents working on TimeLoop-Terminal.
Keep entries precise and reference concrete files and APIs found in the repository.
-->

# Copilot instructions — TimeLoop Terminal (timeloop-terminal)

This file gives immediate, repository-specific guidance for automated coding agents.
Be concise: prefer small, verified edits and run the project's build/tests after changes.

1) Big picture (what to know first)
  - Core runtime is a Rust library + CLI in `src/main.rs` that wires components from `src/lib.rs`.
  - Major components and canonical files:
    - CLI & wiring: `src/main.rs` (clap-based). Global flags set Storage globals early.
    - Library entrypoints: `src/lib.rs` (re-exports core types such as `Storage`, `SessionManager`, `EventRecorder`).
    - Event model & recorder: `src/events.rs`.
    - Session lifecycle: `src/session.rs`.
    - Persistent storage & compaction: `src/storage.rs` (sled-backed, multiple helper APIs such as `Storage::new()`, `Storage::with_path()`, and global setters like `set_global_compaction_policy`).
    - Terminal emulator: `src/terminal.rs` (uses `crossterm`).
    - Replay & branching: `src/replay.rs`, `src/branch.rs`.
    - File watching: `src/file_watcher.rs` (uses `notify`).

2) Primary workflows (commands an agent should run)
  - Build (debug): `cargo build`
  - Build (release): `cargo build --release`
  - Run CLI (default behavior starts a session): `cargo run`
  - Run with features: `cargo run --features gui` or `cargo run --features ai`
  - Tests: `cargo test` (unit tests exist in `src/lib.rs`'s test module)
  - Lint/format: `cargo clippy` and `cargo fmt`
  - Debug logging: prefix with `RUST_LOG=debug cargo run`

3) Repository-specific conventions & patterns
  - Global configuration via CLI: `main.rs` applies persistence/policy flags at startup by calling `Storage::set_global_*` helpers — when changing config-related behavior, update both the CLI parsing and these global setters.
  - Persistence formats: Storage supports JSON and CBOR; `Storage::set_global_persistence_format` is used in `main.rs`.
  - Append-only mode and compaction: storage supports append-only event logs and a compaction policy; tests and tools use `Storage::with_path()` in temporary directories (see tests in `src/lib.rs`).
  - Error type alias and re-exports: the crate re-exports `TimeLoopError` and a `Result<T>` alias in `src/lib.rs`. Use these types for consistency.
  - Feature flags: `gui` and `ai` are optional features. Guard new AI/GUI code with Rust conditional compilation attributes; for example use cfg(feature = "ai") or cfg(feature = "gui") in code paths (the repo uses these patterns in `src/lib.rs` and `src/main.rs`).

4) Integration points & external deps to be careful with
  - sled (embedded DB): used by `src/storage.rs`. Be conservative with concurrent DB access; tests use TempDir+`Storage::with_path()` to isolate DBs.
  - crossterm and terminal I/O: `src/terminal.rs` drives live input; isolate changes behind `TerminalEmulator` when possible.
  - notify file watcher: `src/file_watcher.rs` triggers event recording; ignore patterns live in README/`file_watcher.rs`.
  - Optional network/A.I.: `reqwest` is optional and only used behind the `ai` feature — avoid adding network calls unless guarded by the feature.

5) Editing guidance (practical rules for small changes)
  - Prefer small, focused changes and run `cargo test` after. If you touch storage, run tests that use TempDir to avoid side effects.
  - When adding CLI flags, update `src/main.rs` (clap struct) and the code paths that read globals (see how persistence/argon2/compaction are applied at program start).
  - Use public APIs re-exported in `src/lib.rs` when possible (keeps imports consistent across binaries/tests).
  - If changing event shapes, update `src/events.rs` and every consumer: `storage.rs`, `replay.rs`, `session.rs`, and tests.

6) Examples (concrete pointers)
  - To change persistence format parsing: edit `src/main.rs` near the calls to `Storage::set_global_persistence_format(...)`.
  - To add a new session-related API, implement in `src/session.rs` and then re-export in `src/lib.rs`.
  - To add a background compaction test: copy the pattern used by `tests` in `src/lib.rs` (TempDir + Storage::with_path).

7) CI and repo hooks
  - CI runs are defined under `.github/workflows/` (clippy, CI, CodeQL). Keep to stable Rust 2021 edition and run `cargo clippy` locally before creating PRs.
  - Some workflows reference `.cursorrules` — do not remove or rename repository metadata without checking workflows.

8) Quick checklist for PRs created by an agent
  - Pass `cargo test` and `cargo clippy` locally.
  - If code touches the CLI or feature flags, add a short test or update existing tests.
  - Avoid network calls unless behind `--features ai` and a clear opt-in.
  - Keep changes minimal and document any storage schema changes in the PR description.

If anything above is unclear or you want more examples (small PR templates, focused tests to add, or a specific area to inspect), ask and I will expand this doc.
