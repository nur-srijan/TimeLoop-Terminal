# Fix for SQLite/Sled Lock Error on Concurrent Access

## Issue Summary

[Issue #1](https://github.com/nur-srijan/TimeLoop-Terminal/issues/1) reported database lock errors when multiple instances of TimeLoop Terminal tried to access the same database simultaneously.

**Error Examples:**
- **Linux**: `Resource temporarily unavailable`
- **Windows**: `The process cannot access the file because another process has locked a portion of the file`

## Root Cause

The application uses a sled embedded database (not SQLite as originally reported), which uses file-based locking. When multiple instances try to open the same database file, the second instance fails with a lock error.

## Solution Implemented

### 1. Retry Logic with Exponential Backoff

Added a `open_with_retry()` method to the `Storage` struct that:
- Attempts to open the database up to 5 times
- Uses exponential backoff (100ms, 200ms, 400ms, 800ms, 1600ms)
- Logs retry attempts for debugging

### 2. Intelligent Error Detection

Implemented `is_lock_error()` function that detects lock-related errors by checking for:
- "lock"
- "resource temporarily unavailable" 
- "would block"
- "another process has locked"
- "database is locked"

### 3. User-Friendly Error Messages

When all retries fail, provides a clear error message:
```
Failed to acquire database lock after 5 attempts. 
Another instance of TimeLoop Terminal may be running. 
Please close other instances or wait for them to finish.
```

## Code Changes

### Modified Files:
- `src/storage.rs`: Added retry logic and better error handling
- Fixed imports and removed unused dependencies

### Key Methods Added:
```rust
fn open_with_retry(path: &str) -> crate::Result<sled::Db>
fn is_lock_error(error_msg: &str) -> bool
```

## Testing

Created tests to verify:
1. Lock error detection works for various error message formats
2. Retry mechanism handles concurrent access gracefully
3. Non-lock errors are handled immediately without retries

## Benefits

1. **Improved Reliability**: Multiple instances can now coexist more gracefully
2. **Better User Experience**: Clear error messages explain what's happening
3. **Debugging Support**: Logs show retry attempts and help diagnose issues
4. **Cross-Platform**: Works on both Linux and Windows

## Usage

The fix is transparent to users. When database lock contention occurs, the application will:
1. Automatically retry the connection
2. Log the retry attempts
3. Either succeed or fail with a helpful error message

No changes are required to existing code that uses the `Storage` struct.