## 2024-05-22 - [Optimized ignore pattern matching]
**Learning:** Rust's closures capture variables by reference by default, so explicit cloning inside a loop (like `let ignore_patterns = ignore_patterns.clone();`) is often unnecessary and wasteful, especially for heap-allocated types like `Vec`. Hoisting conversions like `to_string_lossy()` out of inner loops prevents redundant work.
**Action:** Always check if a captured variable needs to be cloned inside a loop or if a reference is sufficient. Look for repeated type conversions in nested loops.

## 2024-05-30 - [Streaming File Hashing]
**Learning:** Reading entire files into memory (`fs::read`) for hashing is a major memory bottleneck (O(N)) for large files. `std::io::BufReader` with a small fixed buffer (e.g., 8KB) allows for O(1) memory usage during hashing.
**Action:** Always prefer streaming or buffered reading when processing file contents, especially for potentially large files.

## 2024-05-30 - [Clippy Cleanliness]
**Learning:** `cargo clippy` is a powerful tool for catching performance anti-patterns (like unnecessary clones, redundant casts) and safety issues. However, when working on a legacy codebase, fixing all warnings might be a large task. It's best to fix them incrementally or focus on the ones related to current changes.
**Action:** Run `cargo clippy` early to catch low-hanging fruit optimizations.
