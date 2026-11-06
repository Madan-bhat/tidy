use crate::organize_existing::PreviewItem;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;

static PENDING: Lazy<Mutex<HashMap<String, PreviewItem>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn upsert(item: PreviewItem) {
    let mut map = PENDING.lock();
    map.insert(item.source.clone(), item);
}

pub fn list() -> Vec<PreviewItem> {
    let mut items: Vec<_> = PENDING.lock().values().cloned().collect();
    items.sort_by(|a, b| {
        a.subject
            .cmp(&b.subject)
            .then(a.file_type.cmp(&b.file_type))
            .then(a.filename.to_lowercase().cmp(&b.filename.to_lowercase()))
    });
    items
}

pub fn clear() {
    PENDING.lock().clear();
}

pub fn remove_sources(sources: &[String]) {
    let mut map = PENDING.lock();
    for s in sources {
        map.remove(s);
    }
}
