use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::sync::{RwLock, Arc};
use std::path::PathBuf;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use crate::Event;
use crate::session::Session;
use crate::branch::TimelineBranch;
use base64;
use chacha20poly1305;
use argon2::Argon2;
use zeroize::Zeroize;
use base64::engine::general_purpose;
use base64::Engine as _;
use rand::RngCore;

#[derive(Default, Clone, Serialize, Deserialize)]
struct StorageInner {
    events: HashMap<String, Vec<Event>>,      // session_id -> events
    sessions: HashMap<String, Session>,       // session_id -> session
    branches: HashMap<String, TimelineBranch>,// branch_id -> branch
}

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
            memory_kib: 65536, // 64 MiB
            iterations: 3,
            parallelism: 1,
        }
    }
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
}

impl Storage {
    pub fn new() -> crate::Result<Self> {
        // Best-effort load persisted state for the global storage
        let _ = Self::load_from_disk();
        Ok(Self { inner: None, persistence_path: None, encryption_key: None, encryption_salt: None, argon2_config: None })
    }

    // `with_path` creates an isolated storage instance whose state is stored in the
    // provided path. If the path exists it will be loaded into memory; mutations on
    // the Storage instance will be persisted to that path. This is useful for
    // integration tests that need on-disk isolation.
    pub fn with_path(path: &str) -> crate::Result<Self> {
        let pb = PathBuf::from(path);
        let inner = Arc::new(RwLock::new(StorageInner::default()));
        let storage = Self { inner: Some(inner.clone()), persistence_path: Some(pb.clone()), encryption_key: None, encryption_salt: None, argon2_config: None };

        // If the file exists, load it into the per-instance inner store
        if pb.exists() {
            if let Ok(data) = std::fs::read_to_string(&pb) {
                if let Ok(inner_data) = serde_json::from_str::<StorageInner>(&data) {
                    if let Ok(mut guard) = inner.write() {
                        *guard = inner_data;
                    }
                }
            }
        }

        Ok(storage)
    }

    /// Create or open a per-instance encrypted storage at `path` using `passphrase`.
    /// If the file exists it will be decrypted with the derived key. If not, a new
    /// salt is generated and used for subsequent writes.
    pub fn with_encryption(path: &str, passphrase: &str) -> crate::Result<Self> {
        let default_params = Argon2Config::default();
        Self::with_encryption_with_params(path, passphrase, &default_params)
    }

    pub fn with_encryption_with_params(path: &str, passphrase: &str, params: &Argon2Config) -> crate::Result<Self> {
        let pb = PathBuf::from(path);
        let inner = Arc::new(RwLock::new(StorageInner::default()));

        // If file exists and appears encrypted, read salt from file wrapper
        let mut encryption_key: Option<[u8; 32]> = None;
        let mut encryption_salt: Option<Vec<u8>> = None;
        if pb.exists() {
            // Try to load wrapper
            if let Ok(wrapper_str) = std::fs::read_to_string(&pb) {
                if let Ok(wrapper) = serde_json::from_str::<EncryptedFile>(&wrapper_str) {
                    if let Ok(salt_bytes) = general_purpose::STANDARD.decode(&wrapper.salt) {
                        // derive key using passphrase and salt
                        let key = Self::derive_key_with_params(passphrase, &salt_bytes, Some(params));
                        // attempt decryption to ensure key is correct
                        if let Ok(ciphertext) = general_purpose::STANDARD.decode(&wrapper.ciphertext) {
                            if let Ok(nonce_bytes) = general_purpose::STANDARD.decode(&wrapper.nonce) {
                                if let Ok(plain) = Self::try_decrypt(&key, &nonce_bytes, &ciphertext) {
                                    // load inner
                                    if let Ok(inner_data) = serde_json::from_slice::<StorageInner>(&plain) {
                                        if let Ok(mut guard) = inner.write() {
                                            *guard = inner_data;
                                        }
                                        encryption_key = Some(key);
                                        encryption_salt = Some(salt_bytes);
                                    }
                                } else {
                                    return Err(crate::error::TimeLoopError::Configuration("Unable to decrypt storage: invalid passphrase".to_string()));
                                }
                            }
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

        Ok(Self { inner: Some(inner), persistence_path: Some(pb), encryption_key, encryption_salt, argon2_config: Some(params.clone()) })
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
            Ok(f(&*guard))
        } else {
            let guard = GLOBAL_STORAGE.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
            Ok(f(&*guard))
        }
    }

    // Helper to run write closures against the correct storage instance
    fn with_write<F, R>(&self, f: F) -> crate::Result<R>
    where
        F: FnOnce(&mut StorageInner) -> R,
    {
        if let Some(inner) = &self.inner {
            let mut guard = inner.write().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
            Ok(f(&mut *guard))
        } else {
            let mut guard = GLOBAL_STORAGE.write().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
            Ok(f(&mut *guard))
        }
    }

    pub fn store_event(&self, event: &Event) -> crate::Result<()> {
        self.with_write(|guard| {
            let session_events = guard.events.entry(event.session_id.clone()).or_insert_with(Vec::new);
            session_events.push(event.clone());
        })?;
        // Persist if global storage is used, or if this instance has a persistence path
        if let Some(path) = &self.persistence_path {
            let _ = Self::save_to_path(path, self);
        } else if self.inner.is_none() {
            let _ = Self::save_to_disk();
        }
        Ok(())
    }

    pub fn get_events_for_session(&self, session_id: &str) -> crate::Result<Vec<Event>> {
        self.with_read(|guard| guard.events.get(session_id).cloned().unwrap_or_default())
    }

    pub fn get_events_in_range(&self, session_id: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> crate::Result<Vec<Event>> {
        let events = self.get_events_for_session(session_id)?;
        let filtered: Vec<Event> = events.into_iter().filter(|e| e.timestamp >= start && e.timestamp <= end).collect();
        Ok(filtered)
    }

    pub fn get_last_event(&self, session_id: &str) -> crate::Result<Option<Event>> {
        let events = self.get_events_for_session(session_id)?;
        Ok(events.last().cloned())
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
    pub fn export_session_to_file(&self, session_id: &str, path: &str) -> crate::Result<()> {
        let session = self.get_session(session_id)?.ok_or_else(|| crate::error::TimeLoopError::SessionNotFound(session_id.to_string()))?;
        let events = self.get_events_for_session(session_id)?;
        let bundle = SessionExport { session, events };
        let json = serde_json::to_string_pretty(&bundle)?;
        let mut file = fs::File::create(path).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        file.write_all(json.as_bytes()).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        Ok(())
    }

    pub fn import_session_from_file(&self, path: &str) -> crate::Result<String> {
        let data = fs::read_to_string(path).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        let bundle: SessionExport = serde_json::from_str(&data)?;
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
        let parent = path.parent().ok_or_else(|| crate::error::TimeLoopError::FileSystem("Invalid path".to_string()))?;
        let mut tmp = parent.join(".tmp_timeloop");
        // add a random suffix to avoid collisions
        let mut osrng = rand::rngs::OsRng;
        let suffix: u64 = osrng.next_u64();
        tmp = tmp.with_extension(format!("{}.tmp", suffix));
 
         // Write tmp file
         fs::write(&tmp, bytes).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
         // Rename into place (atomic on most platforms when on same filesystem)
         std::fs::rename(&tmp, path).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
         Ok(())
     }

    fn save_to_disk() -> crate::Result<()> {
        let dir = Self::data_dir();
        fs::create_dir_all(&dir).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        let path = Self::persistence_file();
        let guard = GLOBAL_STORAGE.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        let data = serde_json::to_string_pretty(&*guard)?;
        // atomic write
        Self::atomic_write(&path, data.as_bytes())?;
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
        let data = fs::read_to_string(&path).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        let inner: StorageInner = serde_json::from_str(&data)?;
        let mut guard = GLOBAL_STORAGE.write().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        *guard = inner;
        Ok(())
    }

    // Save to a per-instance path. Serialize the current inner state (either global
    // or the instance's inner) and write it to the provided path.
    fn save_to_path(path: &PathBuf, storage: &Storage) -> crate::Result<()> {
        // Determine which inner to read from
        let data_inner = if let Some(inner) = &storage.inner {
            inner.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?.clone()
        } else {
            GLOBAL_STORAGE.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?.clone()
        };

        let mut data = serde_json::to_string_pretty(&data_inner)?;
        let mut data_bytes = data.clone().into_bytes();
         // If encryption is enabled on this storage, encrypt the blob and write a wrapper
         if let Some(key) = &storage.encryption_key {
            // reuse salt if present
            let salt = storage.encryption_salt.as_ref().ok_or_else(|| crate::error::TimeLoopError::Configuration("Missing salt for encrypted storage".to_string()))?;
            let (nonce, ciphertext) = Self::encrypt_bytes(key, data.as_bytes())?;
            let wrapper = EncryptedFile {
                salt: general_purpose::STANDARD.encode(salt),
                nonce: general_purpose::STANDARD.encode(&nonce),
                ciphertext: general_purpose::STANDARD.encode(&ciphertext),
            };
            let wrapper_json = serde_json::to_string_pretty(&wrapper)?;
            Self::atomic_write(path, wrapper_json.as_bytes())?;
            // zeroize plaintext
            data_bytes.zeroize();
         } else {
            // Perform an atomic write: write to a temp file and rename into place. This
            // reduces the chance of file corruption if the process is interrupted while
            // writing. Note: on some platforms rename over an existing file may not be
            // atomic across filesystems; avoid sharing the same file across mounts.
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
        let ciphertext = cipher.encrypt(nonce_arr, plaintext).map_err(|e| crate::error::TimeLoopError::FileSystem(format!("Encryption failed: {}", e)))?;
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
    fn derive_key_with_params(passphrase: &str, salt: &[u8], params: Option<&Argon2Config>) -> [u8; 32] {
        let config = params.cloned().unwrap_or_default();
        let mut key = [0u8; 32];
        use argon2::{Algorithm, Version, Params};
        let params = Params::new(config.memory_kib, config.iterations, config.parallelism, None).expect("invalid argon2 params");
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        argon.hash_password_into(passphrase.as_bytes(), salt, &mut key).expect("Argon2 key derivation failed");
        key
    }

    /// Change the passphrase used to encrypt the storage. When called, the current
    /// in-memory state is re-encrypted with a new salt derived from `new_passphrase`.
    /// The old key material is zeroized.
    pub fn change_passphrase(&mut self, new_passphrase: &str) -> crate::Result<()> {
        let path = self.persistence_path.as_ref().ok_or_else(|| crate::error::TimeLoopError::Configuration("change_passphrase requires a persisted storage path".to_string()))?;

        // Determine current inner state
        let data_inner = if let Some(inner) = &self.inner {
            inner.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?.clone()
        } else {
            GLOBAL_STORAGE.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?.clone()
        };

        // Serialize into bytes then encrypt with a newly-derived key
        let mut data_bytes = serde_json::to_vec_pretty(&data_inner)?;

        // Generate new salt and derive new key
        let mut salt = vec![0u8; 16];
        let mut osrng = rand::rngs::OsRng;
        osrng.fill_bytes(&mut salt);
        let new_key = Self::derive_key_with_params(new_passphrase, &salt, self.argon2_config.as_ref());

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventType;
    use uuid::Uuid;
    use tempfile::TempDir;

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

        // Reopen storage
        let storage = Storage::with_path("test-persistence-roundtrip").unwrap();

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
        } else { panic!("expected key press event"); }
        if let EventType::KeyPress { key, .. } = &events[1].event_type {
            assert_eq!(key, "b");
        } else { panic!("expected key press event"); }
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
        let mut storage = Storage::with_encryption(state_file.to_str().unwrap(), "oldpass").unwrap();
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
}