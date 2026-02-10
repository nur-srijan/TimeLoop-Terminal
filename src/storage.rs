use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, Read, Seek, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use argon2::Argon2;
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use once_cell::sync::{Lazy, OnceCell};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::branch::TimelineBranch;
use crate::session::Session;
use crate::Event;

#[derive(Default, Clone, Serialize, Deserialize)]
struct StorageInner {
    events: HashMap<String, Vec<Event>>,       // session_id -> events
    sessions: HashMap<String, Session>,        // session_id -> session
    branches: HashMap<String, TimelineBranch>, // branch_id -> branch
}

impl Drop for StorageInner {
    fn drop(&mut self) {
        // Manually clear hash maps to zeroize contents
        for events in self.events.values_mut() {
            for event in events {
                event.zeroize();
            }
        }
        self.events.clear();

        for session in self.sessions.values_mut() {
            session.zeroize();
        }
        self.sessions.clear();

        for branch in self.branches.values_mut() {
            branch.zeroize();
        }
        self.branches.clear();
    }
}

// Atomic counter for tracking pending write operations to reduce lock contention
static PENDING_WRITES: AtomicU32 = AtomicU32::new(0);

static GLOBAL_STORAGE: Lazy<RwLock<StorageInner>> = Lazy::new(|| RwLock::new(StorageInner::default()));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Argon2Config {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for Argon2Config {
    fn default() -> Self {
        Self {
            memory_kib: 64 * 1024, // 64 MiB
            iterations: 3,
            parallelism: 4,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PersistenceFormat {
    Json,
    Cbor,
}

pub struct Storage {
    // When `inner` is None, operations go to the global singleton (and persist to the global location).
    // When `inner` is Some(...), this Storage instance operates on an independent in-memory store and
    // may optionally persist to the specified `persistence_path`.
    inner: Option<Arc<RwLock<StorageInner>>>,
    persistence_path: Option<PathBuf>,
    // Encryption support
    encryption_key: Option<[u8; 32]>,
    encryption_salt: Option<Vec<u8>>,
    // Argon2 params used to derive keys for this storage instance
    argon2_config: Option<Argon2Config>,
    // Persistence format for this instance
    persistence_format: PersistenceFormat,
    // Append-only event log support
    append_only: bool,
    events_log_path: Option<PathBuf>,
    // Rotation/compaction policy (per-instance overrides global policy when set)
    max_log_size_bytes: Option<u64>,
    max_events: Option<usize>,
    retention_count: usize,
    compaction_interval_secs: Option<u64>,
    // Background compaction control
    background_running: Option<Arc<AtomicBool>>,
    background_handle: Option<thread::JoinHandle<()>>,
    // Pending writes counter for this instance (when not using global storage)
    pending_writes: Option<Arc<AtomicU32>>,
}

impl Storage {
    /// Set per-instance max log size in bytes (overrides global policy for this instance)
    pub fn set_max_log_size_bytes(&mut self, v: Option<u64>) {
        self.max_log_size_bytes = v;
    }

    /// Set per-instance max events threshold (overrides global policy for this instance)
    pub fn set_max_events(&mut self, v: Option<usize>) {
        self.max_events = v;
    }

    /// Set per-instance retention count for rotated logs
    pub fn set_retention_count(&mut self, v: usize) {
        self.retention_count = v;
    }

    /// Set per-instance compaction interval in seconds
    pub fn set_compaction_interval_secs(&mut self, v: Option<u64>) {
        self.compaction_interval_secs = v;
    }

    /// Get the current number of pending write operations
    pub fn get_pending_writes(&self) -> u32 {
        if let Some(ref counter) = self.pending_writes {
            counter.load(Ordering::Relaxed)
        } else {
            PENDING_WRITES.load(Ordering::Relaxed)
        }
    }

    /// Increment pending writes counter
    fn increment_pending_writes(&self) {
        if let Some(ref counter) = self.pending_writes {
            counter.fetch_add(1, Ordering::Relaxed);
        } else {
            PENDING_WRITES.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Decrement pending writes counter
    fn decrement_pending_writes(&self) {
        if let Some(ref counter) = self.pending_writes {
            counter.fetch_sub(1, Ordering::Relaxed);
        } else {
            PENDING_WRITES.fetch_sub(1, Ordering::Relaxed);
        }
    }


    /// Replace the compaction policy for this instance
    pub fn set_compaction_policy(&mut self, p: CompactionPolicy) {
        self.max_log_size_bytes = p.max_log_size_bytes;
        self.max_events = p.max_events;
        self.retention_count = p.retention_count;
        self.compaction_interval_secs = p.compaction_interval_secs;
    }

    /// Get the per-instance retention_count
    pub fn retention_count(&self) -> usize {
        self.retention_count
    }

    pub fn new() -> crate::Result<Self> {
        // Best-effort load persisted state for the global storage
        let _ = Self::load_from_disk();
        // adopt global config
        let fmt = global_persistence_format();
        let append = global_append_only();
        // load global compaction defaults
        let gp = global_compaction_policy();
        let mut s = Self {
            inner: None,
            persistence_path: None,
            encryption_key: None,
            encryption_salt: None,
            argon2_config: None,
            persistence_format: fmt,
            append_only: append,
            events_log_path: None,
            max_log_size_bytes: gp.max_log_size_bytes,
            max_events: gp.max_events,
            retention_count: gp.retention_count,
            compaction_interval_secs: gp.compaction_interval_secs,
            background_running: None,
            background_handle: None,
            pending_writes: None,
        };
        if append {
            // compute events log path for default global persistence file
            let p = Self::persistence_file();
            s.events_log_path = Some(Self::events_log_for(&p, fmt));
            // try to load events from log
            let _ = s.load_events_from_log();
        }
        Ok(s)
    }

    // `with_path` creates an isolated storage instance whose state is stored in the
    // provided path. If the path exists it will be loaded into memory; mutations on
    // the Storage instance will be persisted to that path. This is useful for
    // integration tests that need on-disk isolation.
    pub fn with_path(path: &str) -> crate::Result<Self> {
        // If file extension indicates .cbor or .bin treat it as CBOR, else JSON
        let format = if path.ends_with(".cbor") || path.ends_with(".bin") {
            PersistenceFormat::Cbor
        } else {
            PersistenceFormat::Json
        };
        Self::with_path_and_format(path, format)
    }

    pub fn with_path_and_format(path: &str, format: PersistenceFormat) -> crate::Result<Self> {
        // Resolve relative paths against the current working directory so tests
        // that pass plain filenames write/read to the same location.
        let input_pb = PathBuf::from(path);
        let pb = if input_pb.is_absolute() {
            input_pb
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(input_pb)
        };
        let inner = Arc::new(RwLock::new(StorageInner::default()));

        let gp = global_compaction_policy();
        let pending_writes = Arc::new(AtomicU32::new(0));
        let mut storage = Self {
            inner: Some(inner.clone()),
            persistence_path: Some(pb.clone()),
            encryption_key: None,
            encryption_salt: None,
            argon2_config: None,
            persistence_format: format,
            append_only: false,
            events_log_path: None,
            max_log_size_bytes: gp.max_log_size_bytes,
            max_events: gp.max_events,
            retention_count: gp.retention_count,
            compaction_interval_secs: gp.compaction_interval_secs,
            background_running: None,
            background_handle: None,
            pending_writes: Some(pending_writes),
        };

        // If the file exists, load it into the per-instance inner store
        if pb.exists() {
            let bytes = std::fs::read(&pb).ok();
            if let Some(b) = bytes {
                match format {
                    PersistenceFormat::Json => {
                        if let Ok(inner_data) = serde_json::from_slice::<StorageInner>(&b) {
                            if let Ok(mut guard) = inner.write() {
                                *guard = inner_data;
                            }
                        }
                    }
                    PersistenceFormat::Cbor => {
                        if let Ok(inner_data) = serde_cbor::from_slice::<StorageInner>(&b) {
                            if let Ok(mut guard) = inner.write() {
                                *guard = inner_data;
                            }
                        }
                    }
                }
            }
        }

        // If global append_only is enabled, set events log path accordingly
        if global_append_only() {
            let events_path = Self::events_log_for(&pb, format);
            storage.events_log_path = Some(events_path);
            storage.append_only = true;
            let _ = storage.load_events_from_log();
        }
        Ok(storage)
    }

    /// Create or open a per-instance encrypted storage at `path` using `passphrase`.
    /// If the file exists it will be decrypted with the derived key. If not, a new
    /// salt is generated and used for subsequent writes.
    pub fn with_encryption(path: &str, passphrase: &str) -> crate::Result<Self> {
        let default_params = Argon2Config::default();
        // detect format from extension
        let format = if path.ends_with(".cbor") || path.ends_with(".bin") {
            PersistenceFormat::Cbor
        } else {
            PersistenceFormat::Json
        };
        Self::with_encryption_with_params_and_format(path, passphrase, &default_params, format)
    }

    pub fn with_encryption_with_params_and_format(
        path: &str,
        passphrase: &str,
        params: &Argon2Config,
        format: PersistenceFormat,
    ) -> crate::Result<Self> {
        // Reuse the earlier with_encryption_with_params body but set persistence_format
        let pb = PathBuf::from(path);
        let inner = Arc::new(RwLock::new(StorageInner::default()));

        let mut encryption_key: Option<[u8; 32]> = None;
        let mut encryption_salt: Option<Vec<u8>> = None;
        if pb.exists() {
            if let Ok(bytes) = std::fs::read(&pb) {
                // First, try the encrypted JSON wrapper
                if let Ok(wrapper_str) = std::string::String::from_utf8(bytes.clone()) {
                    if let Ok(wrapper) = serde_json::from_str::<EncryptedFile>(&wrapper_str) {
                        if let Ok(salt_bytes) = general_purpose::STANDARD.decode(&wrapper.salt) {
                            let key =
                                Self::derive_key_with_params(passphrase, &salt_bytes, Some(params));
                            if let Ok(ciphertext) =
                                general_purpose::STANDARD.decode(&wrapper.ciphertext)
                            {
                                if let Ok(nonce_bytes) =
                                    general_purpose::STANDARD.decode(&wrapper.nonce)
                                {
                                    if let Ok(plain) =
                                        Self::try_decrypt(&key, &nonce_bytes, &ciphertext)
                                    {
                                        if let Ok(inner_data) =
                                            serde_json::from_slice::<StorageInner>(&plain)
                                        {
                                            if let Ok(mut guard) = inner.write() {
                                                *guard = inner_data;
                                            }
                                            encryption_key = Some(key);
                                            encryption_salt = Some(salt_bytes);
                                        }
                                    } else {
                                        return Err(crate::error::TimeLoopError::Configuration(
                                            "Unable to decrypt storage: invalid passphrase"
                                                .to_string(),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }

                // If JSON wrapper didn't match, try CBOR encrypted wrapper struct
                if encryption_key.is_none() {
                    if let Ok(wrapper_cbor) = serde_cbor::from_slice::<EncryptedFileCbor>(&bytes) {
                        let salt_bytes = wrapper_cbor.salt;
                        let key =
                            Self::derive_key_with_params(passphrase, &salt_bytes, Some(params));
                        if let Ok(plain) =
                            Self::try_decrypt(&key, &wrapper_cbor.nonce, &wrapper_cbor.ciphertext)
                        {
                            if let Ok(inner_data) = serde_cbor::from_slice::<StorageInner>(&plain) {
                                if let Ok(mut guard) = inner.write() {
                                    *guard = inner_data;
                                }
                                encryption_key = Some(key);
                                encryption_salt = Some(salt_bytes);
                            }
                        } else {
                            return Err(crate::error::TimeLoopError::Configuration(
                                "Unable to decrypt storage: invalid passphrase".to_string(),
                            ));
                        }
                    }
                }
            }
        }

        // If file didn't exist or wasn't encrypted, generate a salt now
        if encryption_salt.is_none() {
            let mut salt = vec![0u8; 16];
            let mut osrng = rand::rngs::OsRng;
            osrng.fill_bytes(&mut salt);
            let key = Self::derive_key_with_params(passphrase, &salt, Some(params));
            encryption_key = Some(key);
            encryption_salt = Some(salt);
        }

        let gp = global_compaction_policy();
        let pending_writes = Arc::new(AtomicU32::new(0));
        Ok(Self { inner: Some(inner), persistence_path: Some(pb), encryption_key, encryption_salt, argon2_config: Some(params.clone()), persistence_format: format, append_only: false, events_log_path: None, max_log_size_bytes: gp.max_log_size_bytes, max_events: gp.max_events, retention_count: gp.retention_count, compaction_interval_secs: gp.compaction_interval_secs, background_running: None, background_handle: None, pending_writes: Some(pending_writes) })
    }

    pub fn get_db_path() -> crate::Result<std::path::PathBuf> {
        Ok(std::path::PathBuf::from("/tmp/timeloop-memory"))
    }

    // Helper to run read-only closures against the correct storage instance
    fn with_read<F, R>(&self, f: F) -> crate::Result<R>
    where
        F: FnOnce(&StorageInner) -> R,
    {
        if let Some(inner) = &self.inner {
            let guard = inner.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
            Ok(f(&guard))
        } else {
            let guard = GLOBAL_STORAGE.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
            Ok(f(&guard))
        }
    }

    // Helper to run write closures against the correct storage instance
    fn with_write<F, R>(&self, f: F) -> crate::Result<R>
    where
        F: FnOnce(&mut StorageInner) -> R,
    {
        self.increment_pending_writes();
        let result = if let Some(inner) = &self.inner {
            let mut guard = inner.write().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
            Ok(f(&mut guard))
        } else {
            let mut guard = GLOBAL_STORAGE.write().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
            Ok(f(&mut guard))
        };
        self.decrement_pending_writes();
        result
    }

    pub fn store_event(&self, event: &Event) -> crate::Result<()> {
        // Always update in-memory storage
        self.with_write(|guard| {
            let session_events = guard.events.entry(event.session_id.clone()).or_default();
            session_events.push(event.clone());
        })?;
        // If append-only logging is enabled, append event to the log; otherwise
        // persist the full state as before.
        if self.append_only {
            let _ = self.append_event_to_log(event);
        } else if let Some(path) = &self.persistence_path {
            let _ = Self::save_to_path(path, self);
        } else if self.inner.is_none() {
            let _ = Self::save_to_disk();
        }
        Ok(())
    }

    pub fn get_events_for_session(&self, session_id: &str) -> crate::Result<Vec<Event>> {
        self.with_read(|guard| guard.events.get(session_id).cloned().unwrap_or_default())
    }

    pub fn get_events_in_range(
        &self,
        session_id: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> crate::Result<Vec<Event>> {
        let events = self.get_events_for_session(session_id)?;
        let filtered: Vec<Event> = events
            .into_iter()
            .filter(|e| e.timestamp >= start && e.timestamp <= end)
            .collect();
        Ok(filtered)
    }

    pub fn get_last_event(&self, session_id: &str) -> crate::Result<Option<Event>> {
        let events = self.get_events_for_session(session_id)?;
        Ok(events.last().cloned())
    }

    pub fn get_last_n_events(
        &self,
        session_id: &str,
        n: usize,
    ) -> crate::Result<Vec<Event>> {
        self.with_read(|guard| {
            guard
                .events
                .get(session_id)
                .map(|events| {
                    let len = events.len();
                    if n == 0 {
                        return Vec::new();
                    }
                    if n >= len {
                        return events.clone();
                    }

                    let mut events = events.clone();
                    events.select_nth_unstable_by_key(len - n, |e| e.sequence_number);
                    events.split_off(len - n)
                })
                .unwrap_or_default()
        })
    }

    pub fn clear_session_events(&self, session_id: &str) -> crate::Result<()> {
        self.with_write(|guard| {
            guard.events.remove(session_id);
        })?;
        if let Some(path) = &self.persistence_path {
            let _ = Self::save_to_path(path, self);
        } else if self.inner.is_none() {
            let _ = Self::save_to_disk();
        }
        Ok(())
    }

    // Session management
    pub fn store_session(&self, session: &Session) -> crate::Result<()> {
        self.with_write(|guard| {
            guard.sessions.insert(session.id.clone(), session.clone());
        })?;
        if let Some(path) = &self.persistence_path {
            let _ = Self::save_to_path(path, self);
        } else if self.inner.is_none() {
            let _ = Self::save_to_disk();
        }
        Ok(())
    }

    pub fn get_session(&self, session_id: &str) -> crate::Result<Option<Session>> {
        self.with_read(|guard| guard.sessions.get(session_id).cloned())
    }

    pub fn list_sessions(&self) -> crate::Result<Vec<Session>> {
        self.with_read(|guard| {
            let mut sessions: Vec<Session> = guard.sessions.values().cloned().collect();
            sessions.sort_by_key(|s| s.created_at);
            sessions
        })
    }

    // Branch management
    pub fn store_branch(&self, branch: &TimelineBranch) -> crate::Result<()> {
        self.with_write(|guard| {
            guard.branches.insert(branch.id.clone(), branch.clone());
        })?;
        if let Some(path) = &self.persistence_path {
            let _ = Self::save_to_path(path, self);
        } else if self.inner.is_none() {
            let _ = Self::save_to_disk();
        }
        Ok(())
    }

    pub fn get_branch(&self, branch_id: &str) -> crate::Result<Option<TimelineBranch>> {
        self.with_read(|guard| guard.branches.get(branch_id).cloned())
    }

    pub fn list_branches(&self) -> crate::Result<Vec<TimelineBranch>> {
        self.with_read(|guard| {
            let mut branches: Vec<TimelineBranch> = guard.branches.values().cloned().collect();
            branches.sort_by_key(|b| b.created_at);
            branches
        })
    }

    pub fn delete_session(&self, session_id: &str) -> crate::Result<()> {
        self.with_write(|guard| {
            guard.events.remove(session_id);
            guard.sessions.remove(session_id);
        })?;
        if let Some(path) = &self.persistence_path {
            let _ = Self::save_to_path(path, self);
        } else if self.inner.is_none() {
            let _ = Self::save_to_disk();
        }
        Ok(())
    }

    pub fn delete_branch(&self, branch_id: &str) -> crate::Result<()> {
        self.with_write(|guard| {
            guard.events.remove(branch_id);
            guard.branches.remove(branch_id);
        })?;
        if let Some(path) = &self.persistence_path {
            let _ = Self::save_to_path(path, self);
        } else if self.inner.is_none() {
            let _ = Self::save_to_disk();
        }
        Ok(())
    }

    // Simple JSON export/import for sessions
    // Note: If the storage instance is encrypted, the backup will also be encrypted using the same key and salt.
    // This ensures that sensitive session data remains protected in backups.
    pub fn export_session_to_file(&self, session_id: &str, path: &str) -> crate::Result<()> {
        let session = self
            .get_session(session_id)?
            .ok_or_else(|| crate::error::TimeLoopError::SessionNotFound(session_id.to_string()))?;
        let events = self.get_events_for_session(session_id)?;
        let bundle = SessionExport { session, events };

        // Serialize the data
        let json = serde_json::to_string_pretty(&bundle)?;
        let mut data_bytes = json.as_bytes().to_vec();

        // If storage is encrypted, re-encrypt the backup data
        if let Some(key) = &self.encryption_key {
            // reuse salt if present
            let salt = self.encryption_salt.as_ref().ok_or_else(|| {
                crate::error::TimeLoopError::Configuration(
                    "Missing salt for encrypted storage".to_string(),
                )
            })?;
            let (nonce, ciphertext) = Self::encrypt_bytes(key, &data_bytes)?;

            // Use EncryptedFile wrapper to include salt, nonce, and ciphertext
            let wrapper = EncryptedFile {
                salt: general_purpose::STANDARD.encode(salt),
                nonce: general_purpose::STANDARD.encode(&nonce),
                ciphertext: general_purpose::STANDARD.encode(&ciphertext),
            };
            let wrapper_json = serde_json::to_string_pretty(&wrapper)?;

            // Write to file
            let mut file = fs::File::create(path).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
            file.write_all(wrapper_json.as_bytes()).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;

            // zeroize plaintext
            data_bytes.zeroize();
        } else {
            // Plaintext export
            let mut file = fs::File::create(path).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
            file.write_all(&data_bytes).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
            data_bytes.zeroize();
        }
        Ok(())
    }

    pub fn import_session_from_file(&self, path: &str) -> crate::Result<String> {
        let file_bytes = fs::read(path).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;

        // Attempt to parse as EncryptedFile JSON first
        let data = if let Ok(json_str) = String::from_utf8(file_bytes.clone()) {
             if let Ok(wrapper) = serde_json::from_str::<EncryptedFile>(&json_str) {
                 // It's an encrypted export
                 if let Some(key) = &self.encryption_key {
                     // Verify salts match if we are to use the current key
                     // Note: Since we are using the current derived key, we can only decrypt if the export was made
                     // with the same salt (same storage instance). If salts differ, we can't decrypt with current key.
                     let file_salt = general_purpose::STANDARD.decode(&wrapper.salt)
                        .map_err(|e| crate::error::TimeLoopError::Storage(format!("Invalid salt in export: {}", e)))?;

                     if let Some(current_salt) = &self.encryption_salt {
                         if &file_salt != current_salt {
                             return Err(crate::error::TimeLoopError::Storage(
                                 "Cannot import encrypted session: salt mismatch. Import is only supported on the same storage instance.".to_string()
                             ));
                         }
                     }

                     let nonce = general_purpose::STANDARD.decode(&wrapper.nonce)
                        .map_err(|e| crate::error::TimeLoopError::Storage(format!("Invalid nonce in export: {}", e)))?;
                     let ciphertext = general_purpose::STANDARD.decode(&wrapper.ciphertext)
                        .map_err(|e| crate::error::TimeLoopError::Storage(format!("Invalid ciphertext in export: {}", e)))?;

                     match Self::try_decrypt(key, &nonce, &ciphertext) {
                         Ok(plain) => plain,
                         Err(_) => return Err(crate::error::TimeLoopError::Storage("Decryption failed".to_string())),
                     }
                 } else {
                     return Err(crate::error::TimeLoopError::Configuration("Storage is not encrypted but import file is.".to_string()));
                 }
             } else {
                 // Not an encrypted wrapper, treat as plaintext
                 file_bytes
             }
        } else {
            // Not valid UTF-8, maybe binary? Treat as is (likely fail JSON parse later if not text)
            file_bytes
        };

        // Parse as JSON
        let json_str = String::from_utf8(data).map_err(|e| crate::error::TimeLoopError::Storage(format!("Invalid UTF-8 in import file: {}", e)))?;
        let bundle: SessionExport = serde_json::from_str(&json_str)?;
        let id = bundle.session.id.clone();
        self.store_session(&bundle.session)?;
        for event in &bundle.events {
            self.store_event(event)?;
        }
        Ok(id)
    }

    pub fn flush(&self) -> crate::Result<()> {
        if let Some(path) = &self.persistence_path {
            Self::save_to_path(path, self)
        } else {
            Self::save_to_disk()
        }
    }

    // Helper to atomically write bytes to a file path. Writes to a temporary file in
    // the same directory and then renames into place.
    fn atomic_write(path: &PathBuf, bytes: &[u8]) -> crate::Result<()> {
        // If path has no parent (e.g., filename in current dir) use current directory
        let parent = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let mut tmp = parent.join(".tmp_timeloop");
        // add a random suffix to avoid collisions
        let mut osrng = rand::rngs::OsRng;
        let suffix: u64 = osrng.next_u64();
        tmp = tmp.with_extension(format!("{}.tmp", suffix));

        // Write tmp file with secure permissions
        #[allow(unused_mut)]
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);

        #[cfg(unix)]
        {
            options.mode(0o600);
        }

        let mut file = options.open(&tmp)
            .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        file.write_all(bytes)
            .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;

        // Rename into place (atomic on most platforms when on same filesystem)
        std::fs::rename(&tmp, path)
            .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        Ok(())
    }

    fn save_to_disk() -> crate::Result<()> {
        let dir = Self::data_dir();
        fs::create_dir_all(&dir)
            .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        let path = Self::persistence_file();
        let guard = GLOBAL_STORAGE
            .read()
            .map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        let data = serde_json::to_string_pretty(&*guard)?;
        // atomic write
        Self::atomic_write(&path, data.as_bytes())?;
        Ok(())
    }

    /// Perform compaction: write a full snapshot atomically and rotate/truncate
    /// the append-only event log according to rotation/retention settings.
    pub fn compact(&self) -> crate::Result<()> {
        // Persist current snapshot
        if let Some(path) = &self.persistence_path {
            Self::save_to_path(path, self)?;
        } else if self.inner.is_none() {
            Self::save_to_disk()?;
        }

        // Rotate/truncate events log
        let log_path = match &self.events_log_path {
            Some(p) => p.clone(),
            None => return Ok(()),
        };

        if !log_path.exists() {
            return Ok(());
        }

        // Decide whether to rotate based on size or event count if configured
        let mut should_rotate = false;
        if let Some(max_size) = self.max_log_size_bytes {
            if let Ok(metadata) = std::fs::metadata(&log_path) {
                if metadata.len() > max_size {
                    should_rotate = true;
                }
            }
        }

        if !should_rotate {
            if let Some(max_ev) = self.max_events {
                // Count events (lines for JSON, records for CBOR)
                if self.persistence_format == PersistenceFormat::Json {
                    if let Ok(file) = std::fs::File::open(&log_path) {
                        let reader = std::io::BufReader::new(file);
                        let mut cnt = 0usize;
                        for _ in reader.lines() {
                            cnt += 1;
                            if cnt > max_ev {
                                should_rotate = true;
                                break;
                            }
                        }
                    }
                } else {
                    // For CBOR count records by iterating length-prefixed entries
                    if let Ok(mut file) = std::fs::File::open(&log_path) {
                        let mut cnt = 0usize;
                        loop {
                            let mut len_buf = [0u8; 4];
                            if file.read_exact(&mut len_buf).is_err() {
                                break;
                            }
                            let len = u32::from_le_bytes(len_buf) as usize;
                            if file.seek(std::io::SeekFrom::Current(len as i64)).is_err() {
                                break;
                            }
                            cnt += 1;
                            if cnt > max_ev {
                                should_rotate = true;
                                break;
                            }
                        }
                    }
                }
            }
        }

        if should_rotate {
            // create rotated name with timestamp
            let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
            // Append an extra .rot.<ts> suffix so rotated files keep the original
            // log filename as a prefix (e.g. state.json.events.jsonl.rot.20250101T...)
            let fname = log_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let rotated = log_path.with_file_name(format!("{}.rot.{}", fname, ts));
            std::fs::rename(&log_path, &rotated)
                .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
            // create new empty log file
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&log_path)
                .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;

            // Enforce retention: remove oldest rotated files beyond retention_count
            let retention = self.retention_count;
            if retention > 0 {
                if let Some(dir) = rotated.parent() {
                    if let Ok(entries) = std::fs::read_dir(dir) {
                        let prefix = log_path
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();
                        // collect rotated files matching prefix.rot.*
                        let mut rots: Vec<(std::time::SystemTime, PathBuf)> = vec![];
                        for e in entries.filter_map(|e| e.ok()) {
                            let p = e.path();
                            if p.file_name()
                                .and_then(|n| n.to_str())
                                .map(|n| n.starts_with(&prefix) && n.contains("rot."))
                                .unwrap_or(false)
                            {
                                if let Ok(meta) = p.metadata() {
                                    if let Ok(mtime) = meta.modified() {
                                        rots.push((mtime, p.clone()));
                                    }
                                }
                            }
                        }
                        // sort by modified time desc (newest first)
                        rots.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
                        if rots.len() > retention {
                            for (_, path) in rots.iter().skip(retention) {
                                let _ = std::fs::remove_file(path);
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Start a background compaction thread (opt-in). The thread periodically
    /// invokes `compact()` according to `compaction_interval_secs`. If no interval
    /// is configured this becomes a no-op.
    pub fn start_background_compaction(&mut self) -> crate::Result<()> {
        if self.background_handle.is_some() {
            return Ok(());
        }
        let interval = match self
            .compaction_interval_secs
            .or_else(|| global_compaction_policy().compaction_interval_secs)
        {
            Some(s) if s > 0 => s,
            _ => return Ok(()),
        };
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();
        // we need a weak clone of storage references: we will call compact on a cloned reference
        let this_path = self.persistence_path.clone();
        let this_events = self.events_log_path.clone();
        let fmt = self.persistence_format;
        let max_size = self.max_log_size_bytes;
        let max_events = self.max_events;
        let retention = self.retention_count;
        let interval_dur = Duration::from_secs(interval);

        // Spawn a thread that owns a minimal Storage-like control structure by closure
        let handle = thread::spawn(move || {
            while running_clone.load(Ordering::SeqCst) {
                thread::sleep(interval_dur);
                // Attempt compaction: recreate a Storage-like ephemeral writer
                if let Some(p) = &this_path {
                    // Try to open and perform a simple rotation snapshot: write the current global snapshot
                    // Note: background compaction uses global snapshot for global storage or file-backed instance snapshot is not accessible here safely.
                    // We'll perform a best-effort: if path exists, rename events log if it exceeds threshold.
                    let log_path = if let Some(e) = &this_events {
                        e.clone()
                    } else {
                        Storage::events_log_for(p, fmt)
                    };
                    if log_path.exists() {
                        // Check thresholds
                        let mut should_rotate = false;
                        if let Some(ms) = max_size {
                            if let Ok(meta) = std::fs::metadata(&log_path) {
                                if meta.len() > ms {
                                    should_rotate = true;
                                }
                            }
                        }
                        if !should_rotate {
                            if let Some(me) = max_events {
                                // count lines
                                if fmt == PersistenceFormat::Json {
                                    if let Ok(file) = std::fs::File::open(&log_path) {
                                        let reader = std::io::BufReader::new(file);
                                        let mut cnt = 0usize;
                                        for _ in reader.lines() {
                                            cnt += 1;
                                            if cnt > me {
                                                should_rotate = true;
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if should_rotate {
                            let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
                            let fname = log_path
                                .file_name()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_default();
                            let rotated = log_path.with_file_name(format!("{}.rot.{}", fname, ts));
                            let _ = std::fs::rename(&log_path, &rotated);
                            let _ = std::fs::OpenOptions::new()
                                .create(true)
                                .write(true)
                                .truncate(true)
                                .open(&log_path);
                            // retention enforcement best-effort
                            if retention > 0 {
                                if let Some(dir) = rotated.parent() {
                                    if let Ok(entries) = std::fs::read_dir(dir) {
                                        let prefix = log_path
                                            .file_name()
                                            .map(|s| s.to_string_lossy().to_string())
                                            .unwrap_or_default();
                                        let mut rots: Vec<(std::time::SystemTime, PathBuf)> =
                                            vec![];
                                        for e in entries.filter_map(|e| e.ok()) {
                                            let p = e.path();
                                            if p.file_name()
                                                .and_then(|n| n.to_str())
                                                .map(|n| {
                                                    n.starts_with(&prefix) && n.contains("rot.")
                                                })
                                                .unwrap_or(false)
                                            {
                                                if let Ok(meta) = p.metadata() {
                                                    if let Ok(mtime) = meta.modified() {
                                                        rots.push((mtime, p.clone()));
                                                    }
                                                }
                                            }
                                        }
                                        rots.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
                                        if rots.len() > retention {
                                            for (_, path) in rots.iter().skip(retention) {
                                                let _ = std::fs::remove_file(path);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        self.background_running = Some(running);
        self.background_handle = Some(handle);
        Ok(())
    }

    /// Stop background compaction if running and join the thread.
    pub fn stop_background_compaction(&mut self) -> crate::Result<()> {
        if let Some(running) = &self.background_running {
            running.store(false, Ordering::SeqCst);
        }
        if let Some(handle) = self.background_handle.take() {
            let _ = handle.join();
        }
        self.background_running = None;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct SessionExport {
    session: Session,
    events: Vec<Event>,
}

#[derive(Serialize, Deserialize)]
struct EncryptedFile {
    salt: String,
    nonce: String,
    ciphertext: String,
}

// Encrypted CBOR wrapper: binary fields stored as raw byte arrays
#[derive(Serialize, Deserialize)]
struct EncryptedFileCbor {
    salt: Vec<u8>,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

impl Storage {
    fn data_dir() -> std::path::PathBuf {
        if cfg!(target_os = "windows") {
            std::env::var("LOCALAPPDATA")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join("timeloop-terminal")
        } else if cfg!(target_os = "macos") {
            std::env::var("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join("Library/Application Support/timeloop-terminal")
        } else {
            // Linux and others
            std::env::var("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(".local/share/timeloop-terminal")
        }
    }

    fn persistence_file() -> std::path::PathBuf {
        Self::data_dir().join("state.json")
    }

    fn load_from_disk() -> crate::Result<()> {
        let path = Self::persistence_file();
        if !path.exists() {
            return Ok(());
        }
        let data = fs::read_to_string(&path)
            .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        let inner: StorageInner = serde_json::from_str(&data)?;
        let mut guard = GLOBAL_STORAGE
            .write()
            .map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        *guard = inner;
        Ok(())
    }

    // Save to a per-instance path. Serialize the current inner state (either global
    // or the instance's inner) and write it to the provided path.
    fn save_to_path(path: &PathBuf, storage: &Storage) -> crate::Result<()> {
        // Determine which inner to read from
        let data_inner = if let Some(inner) = &storage.inner {
            inner
                .read()
                .map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?
                .clone()
        } else {
            GLOBAL_STORAGE
                .read()
                .map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?
                .clone()
        };

        // Serialize according to the chosen persistence format
        let mut data_bytes = match storage.persistence_format {
            PersistenceFormat::Json => serde_json::to_vec_pretty(&data_inner)?,
            PersistenceFormat::Cbor => serde_cbor::to_vec(&data_inner)?,
        };

        // If encryption is enabled on this storage, encrypt the blob and write a wrapper
        if let Some(key) = &storage.encryption_key {
            // reuse salt if present
            let salt = storage.encryption_salt.as_ref().ok_or_else(|| {
                crate::error::TimeLoopError::Configuration(
                    "Missing salt for encrypted storage".to_string(),
                )
            })?;
            let (nonce, ciphertext) = Self::encrypt_bytes(key, data_bytes.as_slice())?;
            match storage.persistence_format {
                PersistenceFormat::Json => {
                    let wrapper = EncryptedFile {
                        salt: general_purpose::STANDARD.encode(salt),
                        nonce: general_purpose::STANDARD.encode(&nonce),
                        ciphertext: general_purpose::STANDARD.encode(&ciphertext),
                    };
                    let wrapper_json = serde_json::to_string_pretty(&wrapper)?;
                    Self::atomic_write(path, wrapper_json.as_bytes())?;
                }
                PersistenceFormat::Cbor => {
                    let wrapper_cbor = EncryptedFileCbor {
                        salt: salt.clone(),
                        nonce,
                        ciphertext,
                    };
                    let wrapper_bytes = serde_cbor::to_vec(&wrapper_cbor)?;
                    Self::atomic_write(path, &wrapper_bytes)?;
                }
            }
            // zeroize plaintext
            data_bytes.zeroize();
        } else {
            // Unencrypted path: write according to format directly
            Self::atomic_write(path, data_bytes.as_slice())?;
            data_bytes.zeroize();
        }
        Ok(())
    }

    // Encrypt given plaintext with the given 32-byte key using XChaCha20-Poly1305.
    fn encrypt_bytes(key: &[u8; 32], plaintext: &[u8]) -> crate::Result<(Vec<u8>, Vec<u8>)> {
        use chacha20poly1305::aead::{Aead, KeyInit};
        use chacha20poly1305::XChaCha20Poly1305;
        use chacha20poly1305::XNonce;
        let cipher = XChaCha20Poly1305::new(key.into());
        let mut nonce = vec![0u8; 24];
        let mut osrng = rand::rngs::OsRng;
        osrng.fill_bytes(&mut nonce[..]);
        let nonce_arr = XNonce::from_slice(&nonce);
        let ciphertext = cipher.encrypt(nonce_arr, plaintext).map_err(|e| {
            crate::error::TimeLoopError::FileSystem(format!("Encryption failed: {}", e))
        })?;
        Ok((nonce, ciphertext))
    }

    fn try_decrypt(key: &[u8; 32], nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, ()> {
        use chacha20poly1305::aead::{Aead, KeyInit};
        use chacha20poly1305::XChaCha20Poly1305;
        use chacha20poly1305::XNonce;
        let cipher = XChaCha20Poly1305::new(key.into());
        let nonce_arr = XNonce::from_slice(nonce);
        cipher.decrypt(nonce_arr, ciphertext).map_err(|_| ())
    }

    // Derive a 32-byte key from passphrase + salt using PBKDF2-HMAC-SHA256
    fn derive_key_with_params(
        passphrase: &str,
        salt: &[u8],
        params: Option<&Argon2Config>,
    ) -> [u8; 32] {
        let config = params.cloned().unwrap_or_default();
        let mut key = [0u8; 32];
        use argon2::{Algorithm, Params, Version};
        let params = Params::new(
            config.memory_kib,
            config.iterations,
            config.parallelism,
            None,
        )
        .expect("invalid argon2 params");
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        argon
            .hash_password_into(passphrase.as_bytes(), salt, &mut key)
            .expect("Argon2 key derivation failed");
        key
    }

    /// Change the passphrase used to encrypt the storage. When called, the current
    /// in-memory state is re-encrypted with a new salt derived from `new_passphrase`.
    /// The old key material is zeroized.
    pub fn change_passphrase(&mut self, new_passphrase: &str) -> crate::Result<()> {
        let path = self.persistence_path.as_ref().ok_or_else(|| {
            crate::error::TimeLoopError::Configuration(
                "change_passphrase requires a persisted storage path".to_string(),
            )
        })?;

        // Determine current inner state
        let data_inner = if let Some(inner) = &self.inner {
            inner
                .read()
                .map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?
                .clone()
        } else {
            GLOBAL_STORAGE
                .read()
                .map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?
                .clone()
        };

        // Serialize into bytes then encrypt with a newly-derived key
        let mut data_bytes = serde_json::to_vec_pretty(&data_inner)?;

        // Generate new salt and derive new key
        let mut salt = vec![0u8; 16];
        let mut osrng = rand::rngs::OsRng;
        osrng.fill_bytes(&mut salt);
        let new_key =
            Self::derive_key_with_params(new_passphrase, &salt, self.argon2_config.as_ref());

        // Encrypt
        let (nonce, ciphertext) = Self::encrypt_bytes(&new_key, &data_bytes)?;

        // Zeroize plaintext bytes now that we've encrypted
        data_bytes.zeroize();

        // Build wrapper and write atomically
        let wrapper = EncryptedFile {
            salt: general_purpose::STANDARD.encode(&salt),
            nonce: general_purpose::STANDARD.encode(&nonce),
            ciphertext: general_purpose::STANDARD.encode(&ciphertext),
        };
        let wrapper_json = serde_json::to_string_pretty(&wrapper)?;
        Self::atomic_write(path, wrapper_json.as_bytes())?;

        // Zeroize and replace old key material
        if let Some(mut old_key) = self.encryption_key.take() {
            old_key.zeroize();
        }
        if let Some(mut old_salt) = self.encryption_salt.take() {
            old_salt.zeroize();
        }

        self.encryption_key = Some(new_key);
        self.encryption_salt = Some(salt);
        Ok(())
    }

    pub fn set_global_persistence_format(fmt: PersistenceFormat) {
        let cell = GLOBAL_PERSISTENCE_FORMAT.get_or_init(|| RwLock::new(fmt));
        if let Ok(mut guard) = cell.write() {
            *guard = fmt;
        }
    }

    pub fn set_global_append_only(flag: bool) {
        let cell = GLOBAL_APPEND_ONLY.get_or_init(|| RwLock::new(flag));
        if let Ok(mut guard) = cell.write() {
            *guard = flag;
        }
    }

    fn events_log_for(path: &std::path::Path, format: PersistenceFormat) -> PathBuf {
        let fname = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "state".to_string());
        match format {
            PersistenceFormat::Json => path.with_file_name(format!("{}.events.jsonl", fname)),
            PersistenceFormat::Cbor => path.with_file_name(format!("{}.events.cborlog", fname)),
        }
    }

    /// Enable append-only event logging for this storage instance. This will create
    /// an events log sibling file and append events to it rather than serializing
    /// the full event list on every mutation.
    pub fn enable_append_only(&mut self) {
        if let Some(p) = &self.persistence_path {
            self.events_log_path = Some(Self::events_log_for(p, self.persistence_format));
            self.append_only = true;
        }
    }

    fn load_events_from_log(&self) -> crate::Result<()> {
        let path = match &self.events_log_path {
            Some(p) => p.clone(),
            None => return Ok(()),
        };

        if !path.exists() {
            return Ok(());
        }

        if self.persistence_format == PersistenceFormat::Json {
            let file = std::fs::File::open(&path)
                .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
            let reader = std::io::BufReader::new(file);
            for line in reader.lines() {
                let l = line.map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
                // Check if encrypted entry (JSON object with nonce/ciphertext) or plain event
                if let Ok(wrapper) = serde_json::from_str::<EncryptedEventJson>(&l) {
                    // encrypted
                    if let Some(key) = &self.encryption_key {
                        let nonce = general_purpose::STANDARD
                            .decode(&wrapper.nonce)
                            .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
                        let ciphertext = general_purpose::STANDARD
                            .decode(&wrapper.ciphertext)
                            .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
                        let plain = Self::try_decrypt(key, &nonce, &ciphertext).map_err(|_| {
                            crate::error::TimeLoopError::Storage("decryption failed".to_string())
                        })?;
                        let event: Event = serde_json::from_slice(&plain)?;
                        // insert event
                        self.with_write(|g| { g.events.entry(event.session_id.clone()).or_default().push(event); })?;
                    }
                } else {
                    let event: Event = serde_json::from_str(&l)?;
                    // insert event
                    self.with_write(|g| { g.events.entry(event.session_id.clone()).or_default().push(event); })?;
                }
            }
        } else {
            // CBOR log: length-prefixed records: u32 LE length followed by bytes. Or encrypted CBOR wrapper entries.
            let mut file = std::fs::File::open(&path)
                .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
            loop {
                let mut len_buf = [0u8; 4];
                if file.read_exact(&mut len_buf).is_err() { break; }
                let len = u32::from_le_bytes(len_buf) as usize;
                let mut buf = vec![0u8; len];
                file.read_exact(&mut buf)
                    .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
                // attempt to deserialize as EncryptedEventCbor first
                if let Ok(wrapper) = serde_cbor::from_slice::<EncryptedEventCbor>(&buf) {
                    if let Some(key) = &self.encryption_key {
                        let plain = Self::try_decrypt(key, &wrapper.nonce, &wrapper.ciphertext)
                            .map_err(|_| {
                                crate::error::TimeLoopError::Storage(
                                    "decryption failed".to_string(),
                                )
                            })?;
                        let event: Event = serde_cbor::from_slice(&plain)?;
                        self.with_write(|g| { g.events.entry(event.session_id.clone()).or_default().push(event); })?;
                    }
                } else {
                    // treat as raw CBOR Event
                    let event: Event = serde_cbor::from_slice(&buf)?;
                    self.with_write(|g| { g.events.entry(event.session_id.clone()).or_default().push(event); })?;
                }
            }
        }

        Ok(())
    }

    fn append_event_to_log(&self, event: &Event) -> crate::Result<()> {
        let path = match &self.events_log_path {
            Some(p) => p.clone(),
            None => return Ok(()),
        };

        if self.persistence_format == PersistenceFormat::Json {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
            if let Some(key) = &self.encryption_key {
                // encrypt event JSON bytes
                let plain = serde_json::to_vec(event)?;
                let (nonce, ciphertext) = Self::encrypt_bytes(key, &plain)?;
                let wrapper = EncryptedEventJson {
                    nonce: general_purpose::STANDARD.encode(&nonce),
                    ciphertext: general_purpose::STANDARD.encode(&ciphertext),
                };
                let line = serde_json::to_string(&wrapper)?;
                file.write_all(line.as_bytes())
                    .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
                file.write_all(b"\n")
                    .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
            } else {
                let line = serde_json::to_string(event)?;
                file.write_all(line.as_bytes())
                    .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
                file.write_all(b"\n")
                    .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
            }
            file.flush()
                .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        } else {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
            if let Some(key) = &self.encryption_key {
                let plain = serde_cbor::to_vec(event)?;
                let (nonce, ciphertext) = Self::encrypt_bytes(key, &plain)?;
                let wrapper = EncryptedEventCbor { nonce, ciphertext };
                let buf = serde_cbor::to_vec(&wrapper)?;
                let len = (buf.len() as u32).to_le_bytes();
                file.write_all(&len)
                    .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
                file.write_all(&buf)
                    .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
            } else {
                let buf = serde_cbor::to_vec(event)?;
                let len = (buf.len() as u32).to_le_bytes();
                file.write_all(&len)
                    .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
                file.write_all(&buf)
                    .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
            }
            file.flush()
                .map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        }
        Ok(())
    }
}

// Global config statics and accessors
static GLOBAL_PERSISTENCE_FORMAT: OnceCell<RwLock<PersistenceFormat>> = OnceCell::new();
static GLOBAL_APPEND_ONLY: OnceCell<RwLock<bool>> = OnceCell::new();
static GLOBAL_COMPACTION_POLICY: OnceCell<RwLock<CompactionPolicy>> = OnceCell::new();
static GLOBAL_ARGON2_CONFIG: OnceCell<RwLock<Argon2Config>> = OnceCell::new();

#[derive(Debug, Clone, Copy)]
pub struct CompactionPolicy {
    pub max_log_size_bytes: Option<u64>,
    pub max_events: Option<usize>,
    pub retention_count: usize,
    pub compaction_interval_secs: Option<u64>,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            max_log_size_bytes: Some(10 * 1024 * 1024),
            max_events: Some(100_000),
            retention_count: 5,
            compaction_interval_secs: Some(60 * 60),
        }
    }
}

fn global_persistence_format() -> PersistenceFormat {
    GLOBAL_PERSISTENCE_FORMAT
        .get_or_init(|| RwLock::new(PersistenceFormat::Json))
        .read()
        .unwrap()
        .clone()
}

fn global_append_only() -> bool {
    *GLOBAL_APPEND_ONLY
        .get_or_init(|| RwLock::new(false))
        .read()
        .unwrap()
}

fn global_compaction_policy() -> CompactionPolicy {
    GLOBAL_COMPACTION_POLICY
        .get_or_init(|| RwLock::new(CompactionPolicy::default()))
        .read()
        .unwrap()
        .clone()
}

#[allow(dead_code)]
fn global_argon2_config() -> Argon2Config {
    GLOBAL_ARGON2_CONFIG
        .get_or_init(|| RwLock::new(Argon2Config::default()))
        .read()
        .unwrap()
        .clone()
}

impl Storage {
    pub fn set_global_compaction_policy(policy: CompactionPolicy) {
        let cell = GLOBAL_COMPACTION_POLICY.get_or_init(|| RwLock::new(policy));
        if let Ok(mut guard) = cell.write() {
            *guard = policy;
        }
    }

    pub fn set_global_argon2_config(cfg: Argon2Config) {
        let cell = GLOBAL_ARGON2_CONFIG.get_or_init(|| RwLock::new(cfg.clone()));
        if let Ok(mut guard) = cell.write() {
            *guard = cfg;
        }
    }
}

// Encrypted event wrappers (module scope)
#[derive(Serialize, Deserialize)]
struct EncryptedEventJson {
    nonce: String,
    ciphertext: String,
}

#[derive(Serialize, Deserialize)]
struct EncryptedEventCbor {
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventType;
    use tempfile::TempDir;
    use uuid::Uuid;

    #[test]
    fn test_in_memory_storage() {
        let tmp_dir = TempDir::new().unwrap();
        let state_file = tmp_dir.path().join("state.json");
        let storage = Storage::with_path(state_file.to_str().unwrap()).unwrap();

        // Test session storage
        let session = Session {
            id: "test-session".to_string(),
            name: "Test Session".to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };

        storage.store_session(&session).unwrap();
        let retrieved = storage.get_session("test-session").unwrap().unwrap();
        assert_eq!(retrieved.id, "test-session");

        // Test event storage
        let event = Event {
            id: Uuid::new_v4().to_string(),
            session_id: "test-session".to_string(),
            event_type: EventType::KeyPress {
                key: "a".to_string(),
                timestamp: Utc::now(),
            },
            sequence_number: 1,
            timestamp: Utc::now(),
        };

        storage.store_event(&event).unwrap();
        let events = storage.get_events_for_session("test-session").unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let tmp_dir = TempDir::new().unwrap();
        let state_file = tmp_dir.path().join("state.json");
        let storage = Storage::with_path(state_file.to_str().unwrap()).unwrap();

        // Initial state: empty
        assert!(storage.list_sessions().unwrap().is_empty());
        assert!(storage.list_branches().unwrap().is_empty());

        // Create session and branch
        let session = Session {
            id: "roundtrip-session".to_string(),
            name: "Roundtrip Session".to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };
        storage.store_session(&session).unwrap();

        let branch = TimelineBranch {
            id: "roundtrip-branch".to_string(),
            name: "Roundtrip Branch".to_string(),
            parent_session_id: "roundtrip-session".to_string(),
            branch_point_event_id: "".to_string(),
            created_at: Utc::now(),
            description: None,
        };
        storage.store_branch(&branch).unwrap();

        // Verify stored state
        let sessions = storage.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "roundtrip-session");

        let branches = storage.list_branches().unwrap();
        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].id, "roundtrip-branch");

        // Write some events
        let event1 = Event {
            id: Uuid::new_v4().to_string(),
            session_id: "roundtrip-session".to_string(),
            event_type: EventType::KeyPress {
                key: "a".to_string(),
                timestamp: Utc::now(),
            },
            sequence_number: 1,
            timestamp: Utc::now(),
        };
        storage.store_event(&event1).unwrap();

        let event2 = Event {
            id: Uuid::new_v4().to_string(),
            session_id: "roundtrip-session".to_string(),
            event_type: EventType::KeyPress {
                key: "b".to_string(),
                timestamp: Utc::now(),
            },
            sequence_number: 2,
            timestamp: Utc::now(),
        };
        storage.store_event(&event2).unwrap();

        // Flush to persist
        storage.flush().unwrap();

        // Drop storage to close file handles
        drop(storage);

        // Reopen storage using the same temp file path
        let storage = Storage::with_path(state_file.to_str().unwrap()).unwrap();

        // Verify restored state
        let sessions = storage.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "roundtrip-session");

        let branches = storage.list_branches().unwrap();
        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].id, "roundtrip-branch");

        // Verify events
        let events = storage.get_events_for_session("roundtrip-session").unwrap();
        assert_eq!(events.len(), 2);
        // Compare key values to avoid asserting on timestamps directly
        if let EventType::KeyPress { key, .. } = &events[0].event_type {
            assert_eq!(key, "a");
        } else {
            panic!("expected key press event");
        }
        if let EventType::KeyPress { key, .. } = &events[1].event_type {
            assert_eq!(key, "b");
        } else {
            panic!("expected key press event");
        }
    }

    #[test]
    fn test_persistence_roundtrip_tempdir() {
        let tmp_dir = TempDir::new().unwrap();
        let state_file = tmp_dir.path().join("state.json");

        // Create first storage instance backed by the file
        let storage1 = Storage::with_path(state_file.to_str().unwrap()).unwrap();

        let session = Session {
            id: "persistence-test".to_string(),
            name: "Persistence Test".to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };
        storage1.store_session(&session).unwrap();
        storage1.flush().unwrap();

        // Create a second storage instance pointing at the same file and verify
        // data persisted.
        let storage2 = Storage::with_path(state_file.to_str().unwrap()).unwrap();
        let retrieved = storage2.get_session("persistence-test").unwrap().unwrap();
        assert_eq!(retrieved.id, "persistence-test");
    }

    #[test]
    fn test_change_passphrase() {
        let tmp_dir = TempDir::new().unwrap();
        let state_file = tmp_dir.path().join("state.json");

        // Create encrypted storage with default argon2 params
        let mut storage =
            Storage::with_encryption(state_file.to_str().unwrap(), "oldpass").unwrap();
        let session = Session {
            id: "cp-session".to_string(),
            name: "ChangePass Session".to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };
        storage.store_session(&session).unwrap();
        storage.flush().unwrap();

        // Change passphrase
        storage.change_passphrase("newpass").unwrap();

        // Reopen with new passphrase
        let storage2 = Storage::with_encryption(state_file.to_str().unwrap(), "newpass").unwrap();
        let retrieved = storage2.get_session("cp-session").unwrap().unwrap();
        assert_eq!(retrieved.id, "cp-session");

        // Opening with old passphrase should fail (return Err)
        let err = Storage::with_encryption(state_file.to_str().unwrap(), "oldpass");
        assert!(err.is_err());
    }

    #[test]
    fn test_cbor_roundtrip() {
        let tmp_dir = TempDir::new().unwrap();
        let state_file = tmp_dir.path().join("state.cbor");
        let storage = Storage::with_path(state_file.to_str().unwrap()).unwrap();

        // Create session and branch
        let session = Session {
            id: "cbor-session".to_string(),
            name: "CBOR Session".to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };
        storage.store_session(&session).unwrap();

        let branch = TimelineBranch {
            id: "cbor-branch".to_string(),
            name: "CBOR Branch".to_string(),
            parent_session_id: "cbor-session".to_string(),
            branch_point_event_id: "".to_string(),
            created_at: Utc::now(),
            description: None,
        };
        storage.store_branch(&branch).unwrap();

        // Write some events
        let event1 = Event {
            id: Uuid::new_v4().to_string(),
            session_id: "cbor-session".to_string(),
            event_type: EventType::KeyPress {
                key: "x".to_string(),
                timestamp: Utc::now(),
            },
            sequence_number: 1,
            timestamp: Utc::now(),
        };
        storage.store_event(&event1).unwrap();

        let event2 = Event {
            id: Uuid::new_v4().to_string(),
            session_id: "cbor-session".to_string(),
            event_type: EventType::KeyPress {
                key: "y".to_string(),
                timestamp: Utc::now(),
            },
            sequence_number: 2,
            timestamp: Utc::now(),
        };
        storage.store_event(&event2).unwrap();

        // Flush to persist
        storage.flush().unwrap();

        // Drop storage to close file handles
        drop(storage);

        // Reopen storage
        let storage = Storage::with_path(state_file.to_str().unwrap()).unwrap();

        // Verify restored state
        let sessions = storage.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "cbor-session");

        let branches = storage.list_branches().unwrap();
        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].id, "cbor-branch");

        // Verify events
        let events = storage.get_events_for_session("cbor-session").unwrap();
        assert_eq!(events.len(), 2);
        // Compare key values to avoid asserting on timestamps directly
        if let EventType::KeyPress { key, .. } = &events[0].event_type {
            assert_eq!(key, "x");
        } else {
            panic!("expected key press event");
        }
        if let EventType::KeyPress { key, .. } = &events[1].event_type {
            assert_eq!(key, "y");
        } else {
            panic!("expected key press event");
        }
    }

    #[test]
    fn test_cbor_encryption_roundtrip() {
        let tmp_dir = TempDir::new().unwrap();
        let state_file = tmp_dir.path().join("state-encrypted.cbor");

        // Create encrypted storage with default argon2 params
        let mut storage =
            Storage::with_encryption(state_file.to_str().unwrap(), "securepass").unwrap();
        let session = Session {
            id: "enc-session".to_string(),
            name: "Encrypted Session".to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };
        storage.store_session(&session).unwrap();
        storage.flush().unwrap();

        // Change passphrase
        storage.change_passphrase("newsecurepass").unwrap();

        // Reopen with new passphrase
        let storage2 =
            Storage::with_encryption(state_file.to_str().unwrap(), "newsecurepass").unwrap();
        let retrieved = storage2.get_session("enc-session").unwrap().unwrap();
        assert_eq!(retrieved.id, "enc-session");

        // Opening with old passphrase should fail (return Err)
        let err = Storage::with_encryption(state_file.to_str().unwrap(), "securepass");
        assert!(err.is_err());
    }

    #[test]
    fn test_append_only_json() {
        let tmp_dir = TempDir::new().unwrap();
        let state_file = tmp_dir.path().join("append_only.json");
        let mut storage =
            Storage::with_path_and_format(state_file.to_str().unwrap(), PersistenceFormat::Json)
                .unwrap();
        storage.enable_append_only();

        let session = Session {
            id: "aj-session".to_string(),
            name: "Append JSON".to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };
        storage.store_session(&session).unwrap();

        let ev = Event {
            id: Uuid::new_v4().to_string(),
            session_id: "aj-session".to_string(),
            event_type: EventType::KeyPress {
                key: "k".to_string(),
                timestamp: Utc::now(),
            },
            sequence_number: 1,
            timestamp: Utc::now(),
        };
        storage.store_event(&ev).unwrap();
        storage.flush().unwrap();

        drop(storage);
        let storage2 =
            Storage::with_path_and_format(state_file.to_str().unwrap(), PersistenceFormat::Json)
                .unwrap();
        let events = storage2.get_events_for_session("aj-session").unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_append_cbor_roundtrip() {
        let tmp_dir = TempDir::new().unwrap();
        let state_file = tmp_dir.path().join("state.cbor");
        let mut storage =
            Storage::with_path_and_format(state_file.to_str().unwrap(), PersistenceFormat::Cbor)
                .unwrap();
        storage.enable_append_only();

        let session = Session {
            id: "ac-session".to_string(),
            name: "Append CBOR".to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };
        storage.store_session(&session).unwrap();

        let ev = Event {
            id: Uuid::new_v4().to_string(),
            session_id: "ac-session".to_string(),
            event_type: EventType::KeyPress {
                key: "k".to_string(),
                timestamp: Utc::now(),
            },
            sequence_number: 1,
            timestamp: Utc::now(),
        };
        storage.store_event(&ev).unwrap();
        storage.flush().unwrap();

        drop(storage);
        let storage2 =
            Storage::with_path_and_format(state_file.to_str().unwrap(), PersistenceFormat::Cbor)
                .unwrap();
        let events = storage2.get_events_for_session("ac-session").unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_compact_rotates_on_size_and_enforces_retention() {
        use std::io::Write as _;
        let tmp_dir = TempDir::new().unwrap();
        let state_file = tmp_dir.path().join("state.json");
        let mut storage =
            Storage::with_path_and_format(state_file.to_str().unwrap(), PersistenceFormat::Json)
                .unwrap();
        storage.enable_append_only();
        // Lower thresholds for test via setters
        storage.set_max_log_size_bytes(Some(1024)); // 1 KiB
        storage.set_retention_count(2);

        // Append events until file exceeds threshold
        for i in 0..200 {
            let ev = Event {
                id: Uuid::new_v4().to_string(),
                session_id: "rt-session".to_string(),
                event_type: EventType::KeyPress {
                    key: format!("k{}", i),
                    timestamp: Utc::now(),
                },
                sequence_number: i,
                timestamp: Utc::now(),
            };
            storage.store_event(&ev).unwrap();
        }
        // Force compaction/rotation
        storage.compact().unwrap();

        // Check rotated files exist (at least one rot file)
        // clone to avoid moving out of `storage` so we can call `storage.compact()` later
        let events_path = storage.events_log_path.clone().unwrap();
        let dir = events_path.parent().unwrap();
        let mut rotated_count = 0usize;
        for entry in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
            let p = entry.path();
            if p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("state.json.events.jsonl") && n.contains("rot."))
                .unwrap_or(false)
            {
                rotated_count += 1;
            }
        }
        assert!(rotated_count >= 1);

        // Create additional rotated files to exceed retention and then run compact again
        for t in 0..4 {
            let fake =
                events_path.with_extension(format!("rot.test{}.{}", t, Utc::now().timestamp()));
            let mut f = std::fs::File::create(&fake).unwrap();
            f.write_all(b"x").unwrap();
        }

        storage.compact().unwrap();

        // After retention enforcement, rotated files should be <= retention_count
        let mut rots = vec![];
        for entry in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
            let p = entry.path();
            if p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("state.json.events.jsonl") && n.contains("rot."))
                .unwrap_or(false)
            {
                rots.push(p);
            }
        }
        assert!(rots.len() <= storage.retention_count as usize + 1); // +1 tolerant
    }

    #[test]
    fn test_export_import_encrypted() {
        let tmp_dir = TempDir::new().unwrap();
        let state_file = tmp_dir.path().join("state.json");
        let export_file = tmp_dir.path().join("export.json");

        // Create encrypted storage
        let storage = Storage::with_encryption(state_file.to_str().unwrap(), "password").unwrap();

        // Create a session
        let session = Session {
            id: "export-session".to_string(),
            name: "Export Session".to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };
        storage.store_session(&session).unwrap();

        // Export session
        storage.export_session_to_file("export-session", export_file.to_str().unwrap()).unwrap();

        // Verify file contains salt (by reading it manually)
        let export_content = std::fs::read_to_string(&export_file).unwrap();
        let wrapper: serde_json::Value = serde_json::from_str(&export_content).unwrap();
        assert!(wrapper.get("salt").is_some());
        assert!(wrapper.get("nonce").is_some());
        assert!(wrapper.get("ciphertext").is_some());

        // Import session back (simulate restore)
        let storage2 = Storage::with_encryption(state_file.to_str().unwrap(), "password").unwrap();
        // Delete session first so import actually does something (though store_session overwrites)
        storage2.delete_session("export-session").unwrap();
        assert!(storage2.get_session("export-session").unwrap().is_none());

        // Import
        let id = storage2.import_session_from_file(export_file.to_str().unwrap()).unwrap();
        assert_eq!(id, "export-session");

        let restored = storage2.get_session("export-session").unwrap().unwrap();
        assert_eq!(restored.name, "Export Session");
    }

    #[test]
    fn test_zeroize_memory() {
        // This is a functional test to ensure the Drop trait compiles and runs without panic.
        let storage = Storage::new().unwrap();
        let session = Session {
            id: "z-session".to_string(),
            name: "Zeroize".to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };
        storage.store_session(&session).unwrap();

        // When storage goes out of scope, it should zeroize inner
        drop(storage);
    }

    #[test]
    fn test_get_last_n_events() {
        let tmp_dir = TempDir::new().unwrap();
        let storage = Storage::with_path(tmp_dir.path().join("state.json").to_str().unwrap()).unwrap();
        let session_id = "test-last-n";

        // Insert events with mixed sequence numbers to test sorting
        // Let's insert: seq 5, 1, 9, 3, 7, 2, 8, 4, 0, 6
        let seqs = [5, 1, 9, 3, 7, 2, 8, 4, 0, 6];
        for &s in &seqs {
            let event = Event {
                id: Uuid::new_v4().to_string(),
                session_id: session_id.to_string(),
                event_type: EventType::KeyPress {
                    key: "a".to_string(),
                    timestamp: Utc::now(),
                },
                sequence_number: s,
                timestamp: Utc::now(),
            };
            storage.store_event(&event).unwrap();
        }

        // We want last 3 events. Should be sequence numbers 7, 8, 9.
        let result = storage.get_last_n_events(session_id, 3).unwrap();
        assert_eq!(result.len(), 3);

        // Verify we got 7, 8, 9
        let mut got_seqs: Vec<u64> = result.iter().map(|e| e.sequence_number).collect();
        got_seqs.sort();
        assert_eq!(got_seqs, vec![7, 8, 9]);

        // Test getting more than available
        let result = storage.get_last_n_events(session_id, 20).unwrap();
        assert_eq!(result.len(), 10);
        let mut got_seqs: Vec<u64> = result.iter().map(|e| e.sequence_number).collect();
        got_seqs.sort();
        assert_eq!(got_seqs, (0..10).map(|i| i as u64).collect::<Vec<_>>());
    }

    #[test]
    fn test_get_last_n_events_zero() {
        let tmp_dir = TempDir::new().unwrap();
        let storage = Storage::with_path(tmp_dir.path().join("state.json").to_str().unwrap()).unwrap();
        let session_id = "test-last-n-zero";

        let event = Event {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            event_type: EventType::KeyPress {
                key: "a".to_string(),
                timestamp: Utc::now(),
            },
            sequence_number: 1,
            timestamp: Utc::now(),
        };
        storage.store_event(&event).unwrap();

        // Getting 0 events should return empty list and NOT panic
        let result = storage.get_last_n_events(session_id, 0).unwrap();
        assert!(result.is_empty());
    }
}
