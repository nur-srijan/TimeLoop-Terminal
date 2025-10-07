TimeLoop Terminal — Roadmap

This document outlines recommended enhancements, cleanups, and potential future work for the TimeLoop Terminal project.

Goals and principles
- Keep the CLI fast, predictable and testable.
- Make the storage system deterministic and safe for both tests and real users.
- Provide a minimal GUI as an opt-in feature (behind a Cargo feature).
- Ensure event recording is complete and that replay is accurate, including timestamps.

Immediate enhancements (done in current patch)
- Per-instance persistent storage: `Storage::with_path(path)` creates an isolated on-disk state file. Useful for integration tests and running multiple independent instances.
- Atomic writes: persistence uses temporary file + rename to minimize risk of partial writes.
- Explicit `Storage::flush()` API to control when data is persisted.
- Storage helpers `with_read`/`with_write` to centralize locking logic.
- Tests converted to use temporary directories to avoid cross-test interference.
- Added integration tests verifying on-disk persistence roundtrips.
- Fixed initial terminal raw mode imports and ensured EventRecorder is always called behind a Mutex guard.
- GUI binary gated behind the `gui` feature to make the optional dependency truly optional.
- Linted test warnings (unused variables/imports) and fixed them.

Short-term (next 1–2 weeks)
- Add `Storage::backup(path)` and `Storage::restore(path)` helpers to manage snapshots.
- Implement a configurable autosave policy (e.g. time-based debounce or write coalescing) to avoid too-frequent disk writes.
- Add an explicit `Storage::open_or_create(path)` that validates file permissions and migration paths.
- Add an integration test that simulates concurrent access (two instances writing to different files) and verify isolation.
- Add `cargo clippy` as part of CI and fix all warnings to keep code quality high.

Medium-term (1–3 months)
- Provide optional write-ahead logging (WAL) for better durability and crash recovery for the global store.
- Add migration path and versioned state files so future storage format changes are safe and reversible.
- Add a compact binary persistence format (e.g., CBOR) as an alternative to JSON for large recordings.
- Implement optional encryption for persisted state files for privacy-sensitive use-cases.
- Improve file-watching by switching to a robust glob library and better rename/old-path detection (platform-specific logic).

Long-term (3+ months)
- Distributed timeline sharing: enable pushing/pulling sessions to/from remote registries.
- Real-time collaboration: multiple users working on a session (requires strong locking/mutation semantics).
- Rich GUI with editing and visual diffing of sessions and branches (EF GUI feature expansion).
- Provide a plugin system for custom event types and AI-based summarizers.

Caveats and considerations
- File-level atomicity depends on the underlying filesystem and rename semantics. Avoid using network-mounted paths unless you accept weaker atomicity guarantees.
- Concurrency: multiple processes writing to the same state file can cause race conditions. Prefer per-instance files or a single writer service.
- Data size: session recording can grow large for long sessions. Add compression and retention policies.
- Privacy: user command output may contain secrets. Provide opt-in redaction options and encrypted storage.

How to contribute and test
- Use the `gui` feature flag to build GUI: `cargo run -p timeloop-terminal --bin gui --features gui`.
- To run tests: `cargo test`.
- To run the CLI: `cargo run -- <command>`.
- When adding persistence-related code, include an integration test that runs on a temporary directory.

Open questions
- Should `with_path` accept a directory (creating `state.json` inside) or a full file path? Current implementation accepts a file path; consider accepting both.
- What retention policy should be default for recordings? (e.g., max size, time-based pruning)

The roadmap will be kept in `docs/ROADMAP.md`. If you'd like, I can open issues or create a project board tracking the above milestones.