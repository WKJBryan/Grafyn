use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const DIMENSIONS: usize = 768;
const MODEL: &str = "embeddinggemma:latest";

pub(super) struct EmbeddingOutput {
    pub vectors: Vec<Vec<f32>>,
    pub model_version: String,
    pub status: String,
}

#[derive(Serialize, Deserialize)]
struct CachedEmbedding {
    text: String,
    version: String,
    vector: Vec<f32>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    model: String,
    embeddings: Vec<Vec<f32>>,
}

fn cache_key(version: &str, text: &str) -> String {
    content_hash(&format!("{version}\n{text}"))
}

pub(super) fn valid_vector(vector: &[f32]) -> bool {
    let norm = vector.iter().map(|v| v * v).sum::<f32>();
    !vector.is_empty() && vector.iter().all(|n| n.is_finite()) && norm.is_finite() && norm > 0.0
}

fn reusable(entry: &CachedEmbedding, version: &str, text: &str) -> bool {
    entry.version == version
        && entry.text == text
        && entry.vector.len() == DIMENSIONS
        && valid_vector(&entry.vector)
}

fn model_digest(tags: &serde_json::Value) -> Option<&str> {
    tags["models"]
        .as_array()?
        .iter()
        .find(|m| m["name"].as_str() == Some(MODEL))?["digest"]
        .as_str()
        .filter(|digest| !digest.is_empty())
}

fn validate_response(response: &EmbedResponse, count: usize) -> Result<()> {
    ensure!(
        response.model == MODEL,
        "Embedding runtime returned a different model"
    );
    ensure!(
        response.embeddings.len() == count,
        "Embedding runtime returned the wrong vector count"
    );
    ensure!(
        response
            .embeddings
            .iter()
            .all(|v| v.len() == DIMENSIONS && valid_vector(v)),
        "Embedding runtime returned invalid vectors or incompatible dimensions"
    );
    Ok(())
}

fn device_observation(ps: &serde_json::Value, digest: &str) -> String {
    let Some(model) = ps["models"].as_array().and_then(|models| {
        models
            .iter()
            .find(|m| m["name"].as_str() == Some(MODEL) && m["digest"].as_str() == Some(digest))
    }) else {
        return "device unknown (no matching running model observation)".into();
    };
    match model["size_vram"].as_u64() {
        Some(bytes) if bytes > 0 => format!(
            "GPU memory observed: {bytes} bytes via Ollama /api/ps; execution placement unverified"
        ),
        Some(_) => "no GPU memory reported via Ollama /api/ps; execution device unknown".into(),
        None => "device unknown (runtime omitted memory observation)".into(),
    }
}

fn local_client(url: &str) -> Result<reqwest::Client> {
    let parsed = reqwest::Url::parse(url)?;
    ensure!(
        matches!(
            parsed.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        ) && matches!(parsed.scheme(), "http" | "https")
            && parsed.username().is_empty()
            && parsed.password().is_none(),
        "Embeddings require an explicitly local Ollama runtime"
    );
    Ok(reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()?)
}

/// The only implemented backend is local Ollama. Device telemetry never changes model identity.
/// No installation, runtime substitution, or remote fallback is performed here.
pub(super) async fn embed_passages(
    texts: &[&str],
    cache_dir: PathBuf,
    ollama_url: &str,
) -> Result<EmbeddingOutput> {
    let client = local_client(ollama_url)?;
    match encode(&client, texts, cache_dir, ollama_url.trim_end_matches('/')).await {
        Ok(output) => Ok(output),
        Err(error) => Ok(EmbeddingOutput {
            vectors: vec![],
            model_version: String::new(),
            status: format!("pending: {error}; no semantic relationships generated"),
        }),
    }
}

async fn encode(
    client: &reqwest::Client,
    texts: &[&str],
    cache_dir: PathBuf,
    base: &str,
) -> Result<EmbeddingOutput> {
    let tags: serde_json::Value = client
        .get(format!("{base}/api/tags"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let digest = model_digest(&tags).ok_or_else(|| anyhow::anyhow!(
        "embeddinggemma:latest is not installed or its digest is unavailable; no model was downloaded"))?.to_string();
    let version = format!("{MODEL}@{digest}:sentence-similarity-v2:dim768");
    let path = cache_dir.join("embeddings.json");
    let mut cache: BTreeMap<String, CachedEmbedding> = if path.exists() {
        serde_json::from_slice(&std::fs::read(&path)?).unwrap_or_default()
    } else {
        BTreeMap::new()
    };
    let missing: Vec<_> = texts
        .iter()
        .filter(|text| {
            !cache
                .get(&cache_key(&version, text))
                .is_some_and(|entry| reusable(entry, &version, text))
        })
        .collect();
    let mut encoded = 0;
    for batch in missing.chunks(16) {
        let inputs: Vec<_> = batch
            .iter()
            .map(|text| format!("task: sentence similarity | query: {text}"))
            .collect();
        let response: EmbedResponse = client.post(format!("{base}/api/embed"))
            .json(&serde_json::json!({"model": MODEL, "input": inputs, "truncate": false, "dimensions": DIMENSIONS}))
            .send().await?.error_for_status()?.json().await?;
        validate_response(&response, batch.len())?;
        for (text, vector) in batch.iter().zip(response.embeddings) {
            cache.insert(
                cache_key(&version, text),
                CachedEmbedding {
                    text: (***text).to_string(),
                    version: version.clone(),
                    vector,
                },
            );
        }
        encoded += batch.len();
    }
    // A tag can be replaced while inference runs. Never publish or cache under a stale identity.
    let current: serde_json::Value = client
        .get(format!("{base}/api/tags"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        model_digest(&current) == Some(digest.as_str()),
        "Embedding model identity changed during encoding; retry available"
    );
    if encoded > 0 {
        std::fs::create_dir_all(&cache_dir)?;
        write_atomic(&path, &serde_json::to_vec(&cache)?)?;
    }
    let observation = if encoded == 0 {
        "cache only; no execution device observed".into()
    } else {
        match client.get(format!("{base}/api/ps")).send().await {
            Ok(r) if r.status().is_success() => {
                device_observation(&r.json().await.unwrap_or_default(), &digest)
            }
            _ => "device unknown (runtime observation unavailable)".into(),
        }
    };
    let vectors = texts
        .iter()
        .map(|text| {
            cache
                .get(&cache_key(&version, text))
                .unwrap()
                .vector
                .clone()
        })
        .collect();
    Ok(EmbeddingOutput {
        vectors,
        model_version: version.clone(),
        status: format!(
            "ready: {version}; runtime Ollama; {observation}; {encoded} passages encoded"
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_vectors_and_mixed_dimensions_fail_closed() {
        for vector in [
            vec![],
            vec![0.0],
            vec![f32::NAN],
            vec![f32::INFINITY],
            vec![f32::MAX],
        ] {
            assert!(!valid_vector(&vector));
        }
        let response = EmbedResponse {
            model: MODEL.into(),
            embeddings: vec![vec![1.0; DIMENSIONS], vec![1.0; 2]],
        };
        assert!(validate_response(&response, 2).is_err());
        assert!(validate_response(
            &EmbedResponse {
                model: "other".into(),
                embeddings: vec![vec![1.0; DIMENSIONS]]
            },
            1
        )
        .is_err());
    }
    #[test]
    fn cache_identity_and_dimensions_must_match() {
        let mut entry = CachedEmbedding {
            text: "text".into(),
            version: "digest-one".into(),
            vector: vec![1.0; DIMENSIONS],
        };
        assert!(reusable(&entry, "digest-one", "text"));
        assert!(!reusable(&entry, "digest-two", "text"));
        assert!(!reusable(&entry, "digest-one", "edited"));
        entry.vector.pop();
        assert!(!reusable(&entry, "digest-one", "text"));
        assert_ne!(
            cache_key("digest-one", "text"),
            cache_key("digest-two", "text")
        );
    }
    #[test]
    fn device_observation_never_invents_npu_or_cpu_placement() {
        assert!(device_observation(&serde_json::json!({}), "digest").contains("unknown"));
        for bytes in [
            serde_json::Value::Null,
            serde_json::json!(0),
            serde_json::json!(1024),
        ] {
            let ps = serde_json::json!({"models": [{"name": MODEL, "digest": "digest", "size_vram": bytes}]});
            let observed = device_observation(&ps, "digest");
            assert!(!observed.contains("NPU"));
            assert!(!observed.contains("CPU"));
            assert!(device_observation(&ps, "changed").contains("unknown"));
        }
    }
    #[tokio::test]
    async fn unavailable_runtime_produces_no_vectors() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let dir = tempfile::tempdir().unwrap();
        let output = embed_passages(
            &["synthetic text"],
            dir.path().to_path_buf(),
            &format!("http://{address}"),
        )
        .await
        .unwrap();
        assert!(output.vectors.is_empty());
        assert!(output.status.starts_with("pending:"));
        assert!(!dir.path().join("embeddings.json").exists());
        assert!(local_client("https://example.com").is_err());
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;
    use std::io::{Read, Write};

    #[tokio::test]
    async fn changed_digest_during_inference_discards_vectors_and_cache() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let responses = [
                serde_json::json!({"models": [{"name": MODEL, "digest": "before"}]}),
                serde_json::json!({"model": MODEL, "embeddings": [vec![1.0; DIMENSIONS]]}),
                serde_json::json!({"models": [{"name": MODEL, "digest": "after"}]}),
            ];
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buffer = [0; 4096];
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("content-length:")
                                    .and_then(|n| n.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let body = response.to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let output = embed_passages(
            &["synthetic sentence"],
            dir.path().to_path_buf(),
            &format!("http://{address}"),
        )
        .await
        .unwrap();
        server.join().unwrap();
        assert!(output.vectors.is_empty());
        assert!(output.status.contains("identity changed"));
        assert!(!dir.path().join("embeddings.json").exists());
    }
}
