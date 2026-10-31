//! Persistent state: stable numeric IDs for sites.
//!
//! IDs are assigned once per site key (`<file>#<domain>`) and never reused, so
//! `ngx show 4` keeps pointing at the same site as configs come and go.

use crate::env::Env;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default)]
struct State {
    next_id: u32,
    ids: BTreeMap<String, u32>,
}

fn path() -> PathBuf {
    let env = Env::get();
    env.host(&env.state_dir.join("state.json"))
}

fn load() -> State {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(st: &State) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(st) {
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

/// Return the ID for each key, assigning new ones in order. Persisted when
/// writable; otherwise the (deterministic) assignment lives in memory only.
pub fn assign_ids(keys: &[String]) -> Vec<u32> {
    let mut st = load();
    st.next_id = st.next_id.max(st.ids.values().copied().max().unwrap_or(0) + 1);
    let mut next = st.next_id;
    let ids = keys
        .iter()
        .map(|k| {
            *st.ids.entry(k.clone()).or_insert_with(|| {
                next += 1;
                next - 1
            })
        })
        .collect();
    if next != st.next_id {
        st.next_id = next;
        save(&st);
    }
    ids
}

/// Move an ID to a new key (config renamed / domain changed) so it stays stable.
pub fn rekey(old: &str, new: &str) {
    let mut st = load();
    if let Some(id) = st.ids.remove(old) {
        st.ids.insert(new.to_string(), id);
        save(&st);
    }
}

/// Forget a removed site (its ID is not reused).
pub fn forget(key: &str) {
    let mut st = load();
    if st.ids.remove(key).is_some() {
        save(&st);
    }
}
