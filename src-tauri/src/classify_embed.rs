//! Local MiniLM embeddings (fastembed / all-MiniLM-L6-v2) for hybrid ranking.
//!
//! Downloads ~23MB ONNX model on first use into the app cache. If the model
//! cannot load, callers fall back to BM25-only classification.

use crate::subjects::SubjectDef;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

const MAX_SUBJECT_CHARS: usize = 1800;
const MAX_KEYWORDS_IN_DOC: usize = 48;

static MODEL: Lazy<Mutex<Option<ModelState>>> = Lazy::new(|| Mutex::new(None));
static SUBJECT_CACHE: Lazy<Mutex<Option<SubjectEmbCache>>> = Lazy::new(|| Mutex::new(None));

struct ModelState {
    // fastembed's TextEmbedding is not Sync in all versions — keep behind mutex.
    inner: fastembed::TextEmbedding,
}

struct SubjectEmbCache {
    fingerprint: u64,
    vectors: HashMap<String, Vec<f32>>,
}

fn embeddings_enabled() -> bool {
    if cfg!(test) {
        return false;
    }
    std::env::var_os("TIDY_DISABLE_EMBED").is_none()
}

/// Load the MiniLM model (no-op if disabled / already loaded).
pub fn warm_up() -> Result<(), String> {
    if !embeddings_enabled() {
        return Err("Embeddings disabled".into());
    }
    let _ = model()?;
    Ok(())
}

pub fn is_ready() -> bool {
    if !embeddings_enabled() {
        return false;
    }
    MODEL.lock().is_some() || try_init_model().is_ok()
}

fn try_init_model() -> Result<(), String> {
    let mut guard = MODEL.lock();
    if guard.is_some() {
        return Ok(());
    }
    use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
    let cache = dirs::cache_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("Tidy")
        .join("fastembed");
    let _ = std::fs::create_dir_all(&cache);
    let model = TextEmbedding::try_new(
        TextInitOptions::new(EmbeddingModel::AllMiniLML6V2)
            .with_cache_dir(cache)
            .with_show_download_progress(false),
    )
    .map_err(|e| format!("Could not load MiniLM: {e}"))?;
    *guard = Some(ModelState { inner: model });
    Ok(())
}

fn model() -> Result<parking_lot::MutexGuard<'static, Option<ModelState>>, String> {
    try_init_model()?;
    Ok(MODEL.lock())
}

fn embed_texts(texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let mut guard = model()?;
    let state = guard.as_mut().ok_or("Embedding model missing")?;
    state
        .inner
        .embed(texts.to_vec(), None)
        .map_err(|e| format!("embed failed: {e}"))
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for i in 0..a.len() {
        let x = a[i] as f64;
        let y = b[i] as f64;
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom < 1e-12 {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
}

fn fingerprint(subjects: &[SubjectDef]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for s in subjects {
        if !s.enabled || s.name.eq_ignore_ascii_case("General") {
            continue;
        }
        s.name.hash(&mut h);
        s.keywords.len().hash(&mut h);
        for k in s.keywords.iter().take(8) {
            k.hash(&mut h);
        }
        if let Some(last) = s.keywords.last() {
            last.hash(&mut h);
        }
    }
    h.finish()
}

fn subject_passage(subject: &SubjectDef) -> String {
    let mut parts = Vec::new();
    parts.push(subject.name.clone());
    for kw in subject.keywords.iter().take(MAX_KEYWORDS_IN_DOC) {
        parts.push(kw.clone());
    }
    let body = parts.join(". ");
    let trimmed: String = body.chars().take(MAX_SUBJECT_CHARS).collect();
    format!("passage: {trimmed}")
}

fn ensure_subject_vectors(subjects: &[SubjectDef]) -> Result<HashMap<String, Vec<f32>>, String> {
    if !embeddings_enabled() {
        return Err("disabled".into());
    }
    let fp = fingerprint(subjects);
    {
        let cache = SUBJECT_CACHE.lock();
        if let Some(c) = cache.as_ref() {
            if c.fingerprint == fp {
                return Ok(c.vectors.clone());
            }
        }
    }

    let mut names = Vec::new();
    let mut docs = Vec::new();
    for s in subjects {
        if !s.enabled || s.name.eq_ignore_ascii_case("General") {
            continue;
        }
        names.push(s.name.clone());
        docs.push(subject_passage(s));
    }
    if docs.is_empty() {
        return Ok(HashMap::new());
    }

    let vectors = embed_texts(&docs)?;
    let mut map = HashMap::new();
    for (name, vec) in names.into_iter().zip(vectors.into_iter()) {
        map.insert(name, vec);
    }

    *SUBJECT_CACHE.lock() = Some(SubjectEmbCache {
        fingerprint: fp,
        vectors: map.clone(),
    });
    Ok(map)
}

/// Semantic similarity scores in roughly \[0, 1\] (cosine shifted from \[-1,1\]).
pub fn semantic_scores(query: &str, subjects: &[SubjectDef]) -> Option<HashMap<String, f64>> {
    if !embeddings_enabled() || query.trim().is_empty() {
        return None;
    }
    let subject_vecs = ensure_subject_vectors(subjects).ok()?;
    if subject_vecs.is_empty() {
        return None;
    }
    let q = format!("query: {}", query.chars().take(500).collect::<String>());
    let qvec = embed_texts(&[q]).ok()?.into_iter().next()?;
    let mut out = HashMap::new();
    for (name, vec) in &subject_vecs {
        let cos = cosine(&qvec, vec);
        // Map cosine [-1,1] → [0,1]
        out.insert(name.clone(), (cos + 1.0) * 0.5);
    }
    Some(out)
}

/// Fuse BM25 scores with MiniLM cosine when lexical ranking is uncertain.
/// Returns the winning subject name if the fused score clears the bar.
pub fn fuse_with_bm25(
    query: &str,
    bm25: &HashMap<String, f64>,
    subjects: &[SubjectDef],
) -> Option<String> {
    let semantic = semantic_scores(query, subjects)?;
    if semantic.is_empty() {
        return None;
    }

    let bm25_max = bm25.values().cloned().fold(0.0_f64, f64::max).max(1e-9);
    let mut fused: HashMap<String, f64> = HashMap::new();

    let names: HashSet<_> = bm25
        .keys()
        .chain(semantic.keys())
        .cloned()
        .collect();

    for name in names {
        let b = bm25.get(&name).copied().unwrap_or(0.0) / bm25_max;
        let s = semantic.get(&name).copied().unwrap_or(0.0);
        // Lexical still leads; semantics break ties / catch synonyms.
        let score = 0.58 * b + 0.42 * s;
        fused.insert(name, score);
    }

    let mut ranked: Vec<_> = fused.iter().collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });
    let (top, top_score) = ranked.first()?;
    let second = ranked.get(1).map(|(_, s)| **s).unwrap_or(0.0);
    // Cosine-space threshold: need a real semantic+lexical agreement.
    if **top_score < 0.48 {
        return None;
    }
    if **top_score - second < 0.04 && **top_score < 0.62 {
        return None;
    }
    Some((*top).clone())
}

use std::collections::HashSet;
