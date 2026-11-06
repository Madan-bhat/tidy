//! Hybrid MiniLM embeddings (BM25 + semantic).
//!
//! ONNX Runtime can `abort()` the whole process. The UI binary must never link
//! ort — inference runs in the `tidy-embed-worker` sidecar. If the worker is
//! missing or dies, ranking falls back to BM25 only.

use crate::subjects::SubjectDef;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const MAX_SUBJECT_CHARS: usize = 1800;
const MAX_KEYWORDS_IN_DOC: usize = 48;

static SUBJECT_CACHE: Lazy<Mutex<Option<SubjectEmbCache>>> = Lazy::new(|| Mutex::new(None));
static WORKER: Lazy<Mutex<Option<Worker>>> = Lazy::new(|| Mutex::new(None));
static DISABLED: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct SubjectEmbCache {
    fingerprint: u64,
    vectors: HashMap<String, Vec<f32>>,
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(Serialize, Deserialize)]
struct EmbedReq {
    id: u64,
    texts: Vec<String>,
}

#[derive(Deserialize)]
struct WorkerHello {
    ready: bool,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct EmbedResp {
    id: u64,
    ok: bool,
    #[serde(default)]
    vectors: Option<Vec<Vec<f32>>>,
    #[serde(default)]
    error: Option<String>,
}

fn embeddings_wanted() -> bool {
    if cfg!(test) {
        return false;
    }
    if DISABLED.load(Ordering::SeqCst) {
        return false;
    }
    std::env::var_os("TIDY_DISABLE_EMBED").is_none()
}

fn disable_permanently(reason: &str) {
    eprintln!("[tidy] embeddings disabled: {reason}");
    DISABLED.store(true, Ordering::SeqCst);
    let mut guard = WORKER.lock();
    if let Some(mut w) = guard.take() {
        let _ = w.child.kill();
        let _ = w.child.wait();
    }
    *SUBJECT_CACHE.lock() = None;
}

/// Resolve the sidecar next to the app binary (Tauri `externalBin`).
fn worker_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "no parent dir for current_exe".to_string())?;

    let names = if cfg!(windows) {
        ["tidy-embed-worker.exe", "tidy-embed-worker"]
    } else {
        ["tidy-embed-worker", "tidy-embed-worker.exe"]
    };

    for name in names {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    // Dev: cargo target next to a running debug/release tidy.
    for rel in [
        "tidy-embed-worker",
        "release/tidy-embed-worker",
        "debug/tidy-embed-worker",
    ] {
        let candidate = dir.join(rel);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(
        "tidy-embed-worker sidecar not found (rebuild the app so the embed worker is bundled)"
            .into(),
    )
}

fn spawn_worker() -> Result<(), String> {
    if !embeddings_wanted() {
        return Err("embeddings disabled".into());
    }
    {
        let guard = WORKER.lock();
        if guard.is_some() {
            return Ok(());
        }
    }

    let path = worker_path()?;
    let mut child = Command::new(&path)
        .env("OMP_NUM_THREADS", "1")
        .env("ORT_NUM_THREADS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", path.display()))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "embed worker missing stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "embed worker missing stdout".to_string())?;
    let mut reader = BufReader::new(stdout);

    let mut hello = String::new();
    reader
        .read_line(&mut hello)
        .map_err(|e| format!("embed worker hello: {e}"))?;
    let hello: WorkerHello = serde_json::from_str(hello.trim())
        .map_err(|e| format!("embed worker hello json: {e} ({hello})"))?;
    if !hello.ready {
        let _ = child.kill();
        let _ = child.wait();
        return Err(hello
            .error
            .unwrap_or_else(|| "embed worker failed to load MiniLM".into()));
    }

    *WORKER.lock() = Some(Worker {
        child,
        stdin,
        stdout: reader,
    });
    Ok(())
}

fn embed_texts(texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    if !embeddings_wanted() {
        return Err("disabled".into());
    }

    let mut guard = WORKER.lock();
    if guard.is_none() {
        drop(guard);
        spawn_worker().map_err(|e| {
            disable_permanently(&e);
            e
        })?;
        guard = WORKER.lock();
    }

    let worker = guard
        .as_mut()
        .ok_or_else(|| "embed worker missing".to_string())?;

    if let Ok(Some(status)) = worker.child.try_wait() {
        *guard = None;
        let msg = format!("embed worker exited ({status})");
        disable_permanently(&msg);
        return Err(msg);
    }

    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let req = EmbedReq {
        id,
        texts: texts.to_vec(),
    };
    let line = serde_json::to_string(&req).map_err(|e| e.to_string())?;
    if let Err(e) = writeln!(worker.stdin, "{line}").and_then(|_| worker.stdin.flush()) {
        *guard = None;
        let msg = format!("embed worker write failed: {e}");
        disable_permanently(&msg);
        return Err(msg);
    }

    let mut resp_line = String::new();
    if let Err(e) = worker.stdout.read_line(&mut resp_line) {
        *guard = None;
        let msg = format!("embed worker read failed: {e}");
        disable_permanently(&msg);
        return Err(msg);
    }
    if resp_line.trim().is_empty() {
        *guard = None;
        let msg = "embed worker returned empty response (crashed?)".to_string();
        disable_permanently(&msg);
        return Err(msg);
    }

    let resp: EmbedResp = serde_json::from_str(resp_line.trim()).map_err(|e| {
        let msg = format!("embed worker bad json: {e}");
        disable_permanently(&msg);
        msg
    })?;
    if resp.id != id {
        return Err(format!(
            "embed worker id mismatch (sent {id}, got {})",
            resp.id
        ));
    }
    if !resp.ok {
        return Err(resp.error.unwrap_or_else(|| "embed failed".into()));
    }
    resp.vectors
        .ok_or_else(|| "embed worker missing vectors".to_string())
}

pub fn warm_up() -> Result<(), String> {
    if !embeddings_wanted() {
        return Err("embeddings disabled (TIDY_DISABLE_EMBED)".into());
    }
    spawn_worker()?;
    let _ = embed_texts(&["query: tidy warm-up".into()])?;
    Ok(())
}

pub fn is_ready() -> bool {
    embeddings_wanted() && WORKER.lock().is_some()
}

pub fn is_enabled() -> bool {
    embeddings_wanted()
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

pub fn semantic_scores(query: &str, subjects: &[SubjectDef]) -> Option<HashMap<String, f64>> {
    if !embeddings_wanted() || query.trim().is_empty() {
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
        out.insert(name.clone(), (cos + 1.0) * 0.5);
    }
    Some(out)
}

/// Semantic-only pass for many leftovers at once (one worker round-trip per chunk).
/// Conservative: needs a real cosine signal and a margin over the runner-up.
pub fn classify_batch(
    queries: &[String],
    subjects: &[SubjectDef],
) -> Vec<Option<(String, f64)>> {
    let mut out: Vec<Option<(String, f64)>> = vec![None; queries.len()];
    if queries.is_empty() || !embeddings_wanted() {
        return out;
    }
    let Ok(subject_vecs) = ensure_subject_vectors(subjects) else {
        return out;
    };
    if subject_vecs.is_empty() {
        return out;
    }

    for (chunk_idx, chunk) in queries.chunks(48).enumerate() {
        let texts: Vec<String> = chunk
            .iter()
            .map(|q| format!("query: {}", q.chars().take(400).collect::<String>()))
            .collect();
        let Ok(vecs) = embed_texts(&texts) else {
            return out;
        };
        for (i, qvec) in vecs.iter().enumerate() {
            let mut ranked: Vec<(&String, f64)> = subject_vecs
                .iter()
                .map(|(name, v)| (name, cosine(qvec, v)))
                .collect();
            ranked.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(b.0))
            });
            let Some((top, top_cos)) = ranked.first() else {
                continue;
            };
            let second = ranked.get(1).map(|r| r.1).unwrap_or(0.0);
            if *top_cos >= 0.30 && *top_cos - second >= 0.05 {
                out[chunk_idx * 48 + i] = Some(((*top).clone(), *top_cos));
            }
        }
    }
    out
}

pub fn fuse_with_bm25(
    query: &str,
    bm25: &HashMap<String, f64>,
    subjects: &[SubjectDef],
) -> Option<String> {
    if !embeddings_wanted() {
        return None;
    }
    let semantic = semantic_scores(query, subjects)?;
    if semantic.is_empty() {
        return None;
    }

    let bm25_max = bm25.values().cloned().fold(0.0_f64, f64::max).max(1e-9);
    let mut fused: HashMap<String, f64> = HashMap::new();

    let names: HashSet<_> = bm25.keys().chain(semantic.keys()).cloned().collect();

    for name in names {
        let b = bm25.get(&name).copied().unwrap_or(0.0) / bm25_max;
        let s = semantic.get(&name).copied().unwrap_or(0.0);
        fused.insert(name, 0.58 * b + 0.42 * s);
    }

    let mut ranked: Vec<_> = fused.iter().collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });
    let (top, top_score) = ranked.first()?;
    let second = ranked.get(1).map(|(_, s)| **s).unwrap_or(0.0);
    if **top_score < 0.48 {
        return None;
    }
    if **top_score - second < 0.04 && **top_score < 0.62 {
        return None;
    }
    Some((*top).clone())
}
