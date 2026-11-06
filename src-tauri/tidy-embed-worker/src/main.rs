//! Standalone MiniLM/ONNX worker. Kept in a separate crate so the UI binary
//! never links ort (which can abort() the process on macOS).

use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

#[derive(Deserialize)]
struct EmbedReq {
    id: u64,
    texts: Vec<String>,
}

#[derive(Serialize)]
struct EmbedResp {
    id: u64,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    vectors: Option<Vec<Vec<f32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn main() {
    use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

    std::env::set_var("OMP_NUM_THREADS", "1");
    std::env::set_var("ORT_NUM_THREADS", "1");

    let cache = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Tidy")
        .join("fastembed");
    let _ = std::fs::create_dir_all(&cache);

    let mut model = match TextEmbedding::try_new(
        TextInitOptions::new(EmbeddingModel::AllMiniLML6V2)
            .with_cache_dir(cache)
            .with_show_download_progress(false)
            .with_intra_threads(1),
    ) {
        Ok(m) => m,
        Err(e) => {
            let _ = writeln!(
                io::stdout(),
                "{}",
                serde_json::json!({ "ready": false, "error": e.to_string() })
            );
            let _ = io::stdout().flush();
            return;
        }
    };

    let _ = writeln!(io::stdout(), "{}", serde_json::json!({ "ready": true }));
    let _ = io::stdout().flush();

    for line in io::stdin().lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let req: EmbedReq = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let _ = writeln!(
                    io::stdout(),
                    "{}",
                    serde_json::to_string(&EmbedResp {
                        id: 0,
                        ok: false,
                        vectors: None,
                        error: Some(format!("bad request: {e}")),
                    })
                    .unwrap_or_default()
                );
                let _ = io::stdout().flush();
                continue;
            }
        };

        let resp = match model.embed(req.texts, None) {
            Ok(vectors) => EmbedResp {
                id: req.id,
                ok: true,
                vectors: Some(vectors),
                error: None,
            },
            Err(e) => EmbedResp {
                id: req.id,
                ok: false,
                vectors: None,
                error: Some(e.to_string()),
            },
        };

        if writeln!(
            io::stdout(),
            "{}",
            serde_json::to_string(&resp).unwrap_or_else(|_| {
                format!(r#"{{"id":{},"ok":false,"error":"serialize"}}"#, req.id)
            })
        )
        .is_err()
        {
            break;
        }
        let _ = io::stdout().flush();
    }
}
