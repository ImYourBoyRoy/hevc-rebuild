//! Bounded process-local profile cache. No persisted private paths or live forecasts.
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

const MAX_ENTRIES: usize = 128;
const MAX_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const POLICY_REVISION: u32 = 1;

#[derive(Default)]
struct Cache {
    entries: VecDeque<(String, Value, usize)>,
    bytes: usize,
}

impl Cache {
    fn get(&mut self, key: &str) -> Option<Value> {
        let index = self.entries.iter().position(|entry| entry.0 == key)?;
        let entry = self.entries.remove(index)?;
        let value = entry.1.clone();
        self.entries.push_back(entry);
        Some(value)
    }

    fn insert(&mut self, key: String, value: Value) {
        let Ok(encoded) = serde_json::to_vec(&value) else {
            return;
        };
        let bytes = key.len().saturating_add(encoded.len());
        if bytes > MAX_BYTES {
            return;
        }
        if let Some(index) = self.entries.iter().position(|entry| entry.0 == key) {
            if let Some(old) = self.entries.remove(index) {
                self.bytes -= old.2;
            }
        }
        while self.entries.len() >= MAX_ENTRIES || self.bytes.saturating_add(bytes) > MAX_BYTES {
            if let Some(old) = self.entries.pop_front() {
                self.bytes -= old.2;
            } else {
                break;
            }
        }
        self.bytes += bytes;
        self.entries.push_back((key, value, bytes));
    }
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}

pub(crate) fn get(key: &str) -> Option<Value> {
    cache().lock().ok()?.get(key)
}
pub(crate) fn insert(key: String, value: Value) {
    if let Ok(mut cache) = cache().lock() {
        cache.insert(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn eviction_replacement_and_oversize_are_bounded() {
        let mut cache = Cache::default();
        for index in 0..MAX_ENTRIES + 1 {
            cache.insert(index.to_string(), json!({"index": index}));
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(cache.get("0").is_none());
        cache.insert("1".into(), json!({"replacement": true}));
        assert_eq!(cache.get("1"), Some(json!({"replacement": true})));
        cache.insert("oversize".into(), json!("x".repeat(MAX_BYTES)));
        assert!(cache.get("oversize").is_none());
        assert!(cache.bytes <= MAX_BYTES);
    }
}
