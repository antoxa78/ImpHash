use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

#[derive(Serialize, Deserialize, Clone)]
struct CacheEntry {
    mtime: u64,
    size: u64,
    hash: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    hash_pdq: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rot_hashes: Option<[u64; 4]>,
}

pub struct HashCache {
    inner: Mutex<HashMap<String, CacheEntry>>,
    path: PathBuf,
}

impl HashCache {
    pub fn new() -> anyhow::Result<Self> {
        let cache_dir = cache_dir_path();
        fs::create_dir_all(&cache_dir)?;
        let path = cache_dir.join("hashes.json");

        let data = if path.exists() {
            let content = fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            HashMap::new()
        };

        Ok(HashCache {
            inner: Mutex::new(data),
            path,
        })
    }

    pub fn lookup(&self, path: &Path, mtime: u64, size: u64) -> Option<u64> {
        let map = self.inner.lock().unwrap();
        let key = path_key(path);
        map.get(&key).and_then(|entry| {
            if entry.mtime == mtime && entry.size == size {
                Some(entry.hash)
            } else {
                None
            }
        })
    }

    pub fn insert(&self, path: &Path, mtime: u64, size: u64, hash: u64) {
        let mut map = self.inner.lock().unwrap();
        let entry = map.entry(path_key(path)).or_insert(CacheEntry {
            mtime: 0,
            size: 0,
            hash: 0,
            hash_pdq: None,
            rot_hashes: None,
        });
        entry.mtime = mtime;
        entry.size = size;
        entry.hash = hash;
    }

    #[cfg(feature = "pdq")]
    #[allow(dead_code)]
    pub fn lookup_pdq(&self, path: &Path, mtime: u64, size: u64) -> Option<String> {
        let map = self.inner.lock().unwrap();
        let key = path_key(path);
        map.get(&key).and_then(|entry| {
            if entry.mtime == mtime && entry.size == size {
                entry.hash_pdq.clone()
            } else {
                None
            }
        })
    }

    #[cfg(feature = "pdq")]
    #[allow(dead_code)]
    pub fn insert_pdq(&self, path: &Path, mtime: u64, size: u64, hash: u64, pdq_hex: &str) {
        let mut map = self.inner.lock().unwrap();
        let entry = map.entry(path_key(path)).or_insert(CacheEntry {
            mtime: 0,
            size: 0,
            hash: 0,
            hash_pdq: None,
            rot_hashes: None,
        });
        entry.mtime = mtime;
        entry.size = size;
        entry.hash = hash;
        entry.hash_pdq = Some(pdq_hex.to_owned());
    }

    pub fn lookup_rot(&self, path: &Path, mtime: u64, size: u64) -> Option<[u64; 4]> {
        let map = self.inner.lock().unwrap();
        let key = path_key(path);
        map.get(&key).and_then(|entry| {
            if entry.mtime == mtime && entry.size == size {
                entry.rot_hashes
            } else {
                None
            }
        })
    }

    pub fn insert_rot(&self, path: &Path, mtime: u64, size: u64, hashes: [u64; 4]) {
        let mut map = self.inner.lock().unwrap();
        let key = path_key(path);
        let entry = map.entry(key).or_insert(CacheEntry {
            mtime: 0,
            size: 0,
            hash: 0,
            hash_pdq: None,
            rot_hashes: None,
        });
        entry.mtime = mtime;
        entry.size = size;
        entry.rot_hashes = Some(hashes);
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let map = self.inner.lock().unwrap();
        let content = serde_json::to_string_pretty(&*map)?;
        fs::write(&self.path, content)?;
        Ok(())
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        let mut map = self.inner.lock().unwrap();
        map.clear();
        let content = serde_json::to_string_pretty(&*map)?;
        fs::write(&self.path, content)?;
        Ok(())
    }
}

fn cache_dir_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(dir)
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".cache")
    }
    .join("imphash")
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

#[allow(dead_code)]
pub fn file_mtime(path: &Path) -> u64 {
    fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
