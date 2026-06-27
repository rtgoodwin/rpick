//! Persistent caching of video metadata for fast startup

use crate::video::CachedVideoInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Full cache structure stored on disk
#[derive(Debug, Serialize, Deserialize)]
pub struct VideoFileCache {
    pub files: HashMap<String, CachedVideoInfo>,
}

impl VideoFileCache {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    /// Load cache from a JSON file
    pub fn load(path: &str) -> Result<Self, String> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read cache: {}", e))?;
        serde_json::from_str(&data)
            .map_err(|e| format!("Failed to parse cache: {}", e))
    }

    /// Save cache to a JSON file
    pub fn save(&self, path: &str) -> Result<(), String> {
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize cache: {}", e))?;
        std::fs::write(path, &data)
            .map_err(|e| format!("Failed to write cache: {}", e))
    }

    /// Get cached info for a file path, checking staleness
    pub fn get(&self, path: &str) -> Option<&CachedVideoInfo> {
        self.files.get(path)
    }

    /// Check if a cached entry is stale (file changed on disk)
    pub fn is_stale(path: &str, info: &CachedVideoInfo) -> bool {
        match std::fs::metadata(path) {
            Ok(meta) => {
                let size = meta.len() as i64;
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                size != info.size || mtime != info.mod_time_secs
            }
            Err(_) => true,
        }
    }

    /// Insert or update a cached entry
    pub fn set(&mut self, path: String, info: CachedVideoInfo) {
        self.files.insert(path, info);
    }

    /// Remove a path from the cache
    pub fn remove(&mut self, path: &str) {
        self.files.remove(path);
    }

    /// Number of entries
    pub fn len(&self) -> usize {
        self.files.len()
    }
}

/// Deleted hashes store (simple text file, one hex-encoded hash per line)
#[derive(Debug)]
pub struct DeletedHashes {
    pub hashes: HashMap<[u8; 16], bool>,
}

impl DeletedHashes {
    pub fn new() -> Self {
        Self {
            hashes: HashMap::new(),
        }
    }

    /// Load from file
    pub fn load(path: &str) -> Result<Self, String> {
        let mut dh = Self::new();
        let content = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(dh);
            }
            Err(e) => return Err(format!("Failed to read deleted hashes: {}", e)),
        };

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut hash = [0u8; 16];
            match hex::decode_to_slice(line, &mut hash) {
                Ok(()) => {
                    dh.hashes.insert(hash, true);
                }
                Err(e) => {
                    eprintln!("Warning: invalid hash line '{}': {}", line, e);
                }
            }
        }

        Ok(dh)
    }

    /// Save to file
    pub fn save(&self, path: &str) -> Result<(), String> {
        let mut content = String::new();
        for hash in self.hashes.keys() {
            content.push_str(&hex::encode(hash));
            content.push('\n');
        }
        std::fs::write(path, &content)
            .map_err(|e| format!("Failed to write deleted hashes: {}", e))
    }

    /// Check if a hash is in the deleted set
    pub fn contains(&self, hash: &[u8; 16]) -> bool {
        self.hashes.contains_key(hash)
    }

    /// Insert a hash
    pub fn insert(&mut self, hash: [u8; 16]) {
        self.hashes.insert(hash, true);
    }

    /// Number of stored hashes
    pub fn len(&self) -> usize {
        self.hashes.len()
    }
}
