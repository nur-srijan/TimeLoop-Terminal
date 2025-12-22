## 2024-05-22 - [Optimized ignore pattern matching]
**Learning:** Rust's closures capture variables by reference by default, so explicit cloning inside a loop (like `let ignore_patterns = ignore_patterns.clone();`) is often unnecessary and wasteful, especially for heap-allocated types like `Vec`. Hoisting conversions like `to_string_lossy()` out of inner loops prevents redundant work.
**Action:** Always check if a captured variable needs to be cloned inside a loop or if a reference is sufficient. Look for repeated type conversions in nested loops.

## 2024-05-22 - [Streamed File Hashing]
**Learning:** `fs::read(path)` loads the entire file into memory. For potentially large files (like those monitored by a file watcher), this can cause huge memory spikes or OOM crashes. Using `File::open` with a buffered read loop keeps memory usage constant (e.g., 8KB) regardless of file size.
**Action:** When reading files where the size is unknown or potentially large (e.g., user content, logs), always use buffered/streaming reads instead of `fs::read` or `fs::read_to_string`.
