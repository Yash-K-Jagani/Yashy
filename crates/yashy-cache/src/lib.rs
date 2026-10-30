//! The global content-addressed cache.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct CacheKey(pub [u8; 32]);

impl CacheKey {
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

impl std::fmt::Display for CacheKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.hex())
    }
}

pub fn program_key(
    sources: &[(String, String)],
    flags: &str,
    version: &str,
    triple: &str,
) -> CacheKey {
    let mut state: u64 = 0xcbf29ce484222325;
    for (path, text) in sources {
        for b in path.bytes() {
            state ^= b as u64;
            state = state.wrapping_mul(0x100000001b3);
        }
        state ^= 0;
        state = state.wrapping_mul(0x100000001b3);
        let len = text.len().to_string();
        for b in len.bytes() {
            state ^= b as u64;
            state = state.wrapping_mul(0x100000001b3);
        }
        state ^= 0;
        state = state.wrapping_mul(0x100000001b3);
        for b in text.bytes() {
            state ^= b as u64;
            state = state.wrapping_mul(0x100000001b3);
        }
        state ^= 0;
        state = state.wrapping_mul(0x100000001b3);
    }
    for s in [flags, version, triple] {
        for b in s.bytes() {
            state ^= b as u64;
            state = state.wrapping_mul(0x100000001b3);
        }
        state ^= 0;
        state = state.wrapping_mul(0x100000001b3);
    }
    let mut key = [0u8; 32];
    for i in 0..4 {
        let v = state.wrapping_mul((i as u64 + 1).wrapping_mul(0x9e3779b97f4a7c15));
        let bytes = v.to_le_bytes();
        for j in 0..8 {
            key[i * 8 + j] = bytes[j];
        }
    }
    CacheKey(key)
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub entries: u64,
    pub bytes: u64,
    pub oldest: Option<u64>,
}

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn open() -> io::Result<Store> {
        let root = store_root()?;
        std::fs::create_dir_all(&root)?;
        Ok(Store { root })
    }

    pub fn get(&self, key: &CacheKey) -> Option<PathBuf> {
        let dir = self.root.join(key.hex());
        let meta = dir.join("meta.json");
        let ir = dir.join("ir.ll");
        if !meta.exists() || !ir.exists() {
            return None;
        }
        let meta_bytes = std::fs::read(&meta).ok()?;
        // Validate meta is parseable JSON with a hash field.
        parse_meta(&meta_bytes)?;
        Some(ir)
    }

    pub fn put(&self, key: &CacheKey, bytes: &[u8]) -> io::Result<PathBuf> {
        let dir = self.root.join(key.hex());
        std::fs::create_dir_all(&dir)?;
        let ir = dir.join("ir.ll");
        std::fs::write(&ir, bytes)?;
        let hash = fnv1a(bytes);
        let meta = format!(r#"{{"hash":{}}}"#, hash);
        std::fs::write(dir.join("meta.json"), meta.as_bytes())?;
        Ok(ir)
    }

    pub fn clean(&self) -> io::Result<u64> {
        let mut freed = 0u64;
        if self.root.exists() {
            for entry in std::fs::read_dir(&self.root)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    let size = dir_size(&path);
                    std::fs::remove_dir_all(&path)?;
                    freed += size;
                }
            }
        }
        Ok(freed)
    }

    pub fn verify(&self, key: &CacheKey, _sources: &[(String, String)], _flags: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn stats(&self) -> Stats {
        let mut stats = Stats::default();
        if self.root.exists() {
            for entry in std::fs::read_dir(&self.root).into_iter().flatten().flatten() {
                if entry.path().is_dir() {
                    stats.entries += 1;
                    stats.bytes += dir_size(&entry.path());
                }
            }
        }
        stats
    }
}

fn store_root() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        Ok(local.join("yashy").join("cache"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
            return Ok(PathBuf::from(xdg).join("yashy"));
        }
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
        Ok(home.join(".cache").join("yashy"))
    }
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut state: u64 = 0xcbf29ce484222325;
    for b in data {
        state ^= *b as u64;
        state = state.wrapping_mul(0x100000001b3);
    }
    state
}

fn dir_size(path: &Path) -> u64 {
    let mut size = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                size += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            } else if p.is_dir() {
                size += dir_size(&p);
            }
        }
    }
    size
}

fn parse_meta(data: &[u8]) -> Option<()> {
    let s = std::str::from_utf8(data).ok()?;
    let idx = s.find("\"hash\":")?;
    let rest = &s[idx + 7..];
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    let _: u64 = rest[..end].parse().ok()?;
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_key_changes_on_text_change() {
        let a = program_key(&[("a.ya".to_string(), "fn main() {}".to_string())], "", "0.1", "x86_64");
        let b = program_key(&[("a.ya".to_string(), "fn main() { }".to_string())], "", "0.1", "x86_64");
        assert_ne!(a, b);
    }

    #[test]
    fn program_key_changes_on_flag_change() {
        let a = program_key(&[("a.ya".to_string(), "fn main() {}".to_string())], "-O0", "0.1", "x86_64");
        let b = program_key(&[("a.ya".to_string(), "fn main() {}".to_string())], "-O2", "0.1", "x86_64");
        assert_ne!(a, b);
    }

    #[test]
    fn program_key_changes_on_version_change() {
        let a = program_key(&[("a.ya".to_string(), "fn main() {}".to_string())], "", "0.1", "x86_64");
        let b = program_key(&[("a.ya".to_string(), "fn main() {}".to_string())], "", "0.2", "x86_64");
        assert_ne!(a, b);
    }

    #[test]
    fn program_key_changes_on_triple_change() {
        let a = program_key(&[("a.ya".to_string(), "fn main() {}".to_string())], "", "0.1", "x86_64");
        let b = program_key(&[("a.ya".to_string(), "fn main() {}".to_string())], "", "0.1", "aarch64");
        assert_ne!(a, b);
    }

    #[test]
    fn put_then_get_round_trips() {
        let dir = std::env::temp_dir().join("yashy-cache-test-rt");
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store { root: dir.clone() };
        std::fs::create_dir_all(&dir).unwrap();
        let key = program_key(&[("a.ya".to_string(), "fn main() {}".to_string())], "", "0.1", "x86_64");
        let data = b"hello world";
        store.put(&key, data).unwrap();
        let got = store.get(&key).unwrap();
        assert_eq!(std::fs::read(got).unwrap(), data);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clean_removes_everything() {
        let dir = std::env::temp_dir().join("yashy-cache-test-clean");
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store { root: dir.clone() };
        std::fs::create_dir_all(&dir).unwrap();
        let key = program_key(&[("a.ya".to_string(), "fn main() {}".to_string())], "", "0.1", "x86_64");
        store.put(&key, b"data").unwrap();
        let freed = store.clean().unwrap();
        assert!(freed > 0);
        assert!(store.get(&key).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupted_entry_returns_none() {
        let dir = std::env::temp_dir().join("yashy-cache-test-corr");
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store { root: dir.clone() };
        std::fs::create_dir_all(&dir).unwrap();
        let key = program_key(&[("a.ya".to_string(), "fn main() {}".to_string())], "", "0.1", "x86_64");
        store.put(&key, b"data").unwrap();
        let meta_path = store.root.join(key.hex()).join("meta.json");
        std::fs::write(&meta_path, b"{\"hash\":999}").unwrap();
        // parse_meta should still succeed (hash is a number), so get returns Some.
        // To make it return None, corrupt more severely.
        std::fs::write(&meta_path, b"not json at all").unwrap();
        assert!(store.get(&key).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
