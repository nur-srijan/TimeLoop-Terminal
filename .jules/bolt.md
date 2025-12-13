## 2024-05-22 - [Optimized ignore pattern matching]
**Learning:** Rust's closures capture variables by reference by default, so explicit cloning inside a loop (like `let ignore_patterns = ignore_patterns.clone();`) is often unnecessary and wasteful, especially for heap-allocated types like `Vec`. Hoisting conversions like `to_string_lossy()` out of inner loops prevents redundant work.
**Action:** Always check if a captured variable needs to be cloned inside a loop or if a reference is sufficient. Look for repeated type conversions in nested loops.
