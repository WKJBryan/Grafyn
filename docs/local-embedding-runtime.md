# Local embedding runtime boundary

As of September 5, 2026, the executable integration is local Ollama. Native Windows NPU inference is **not implemented or validated**. The adapter never selects an invented provider, substitutes another model, installs software, or falls back to a cloud endpoint.

## Observed machine and feasibility

Read-only inspection found Windows build 26200, an AMD Ryzen 7 7700 8-Core Processor (processor registry), and NVIDIA GeForce RTX 5090 with driver 591.86 (`nvidia-smi`). CIM and PnP enumeration were denied by the execution environment, so a complete NPU device inventory is **unknown**, not a successful negative enumeration. Ollama `/api/version` returned `0.33.2`; `/api/tags` contained no embedding model. Consequently, actual embedding execution, device placement, throughput, and semantic quality were not measured. No model was downloaded during this inspection.

## Implemented contract

`services/evidence/embedding.rs` owns local endpoint validation, exact model lookup, batch embedding, cache validation, and runtime telemetry. `discover_relationships(snapshot, cache_dir, ollama_url)` remains compatible with existing callers. Endpoint redirects and proxies are disabled; only explicitly loopback HTTP(S) endpoints are admitted.

The selected model is exactly `embeddinggemma:latest`, with the installed digest frozen before encoding and checked again before publishing vectors or writing the cache. The request requires 768 dimensions and disables truncation. The adapter prepends `task: sentence similarity | query: `; receipts retain their original exact text. The cache identity includes the model tag, digest, `sentence-similarity-v2`, and dimensions. Legacy raw-passage caches therefore cannot be mixed with this representation. Vector counts, finite nonzero norms, dimensions, model response names, and cached text identities are validated before comparison. A runtime failure produces pending status and no candidates; stale identities never publish under the original digest.

The model's 768-dimensional output and semantic-similarity prompt follow the [Google model card](https://ai.google.dev/gemma/docs/embeddinggemma/model_card). Request fields and response model identification follow the [Ollama embedding API](https://docs.ollama.com/api/embed).

Runtime observation remains separate from model identity. After encoding, `/api/ps` is matched by exact tag and digest. Positive `size_vram` is reported as GPU memory observed, **not** proof of operator placement or full GPU execution. Missing entries or telemetry remain unknown; zero memory does not imply proven CPU placement. A cache-only request explicitly reports that it observed no execution device. None of these states can claim NPU inference. See the [Ollama running-model API](https://docs.ollama.com/api/ps).

A ready new representation clears obsolete graph scores without discarding reviewed or assessed interpretations. Rediscovered pairs refresh their score/model metadata while preserving human corrections and scorer audit records. Pairs omitted by the new representation remain unscored; unassessed ones wait for candidate discovery rather than entering the scorer queue with an obsolete score. Rejected pairs are not revived, and a pending/unavailable runtime does not erase previously valid scores.

## Concrete native Windows acceptance path

Windows ML can execute ONNX models and exposes hardware execution providers. Its dynamic provider catalog requires Windows 11 24H2/build 26100 or later; compatible devices and drivers are still required. NPU providers include Intel OpenVINO, Qualcomm QNN and AMD VitisAI. Provider availability alone does not validate this embedding model. See [supported providers](https://learn.microsoft.com/en-us/windows/ai/new-windows-ml/supported-execution-providers).

The smallest supported native investigation is a Windows ML C++/WinRT integration with an explicitly installed, version-pinned ONNX embedding model and tokenizer. First run the ONNX graph on CPU, then register an already-installed compatible provider, enumerate actual devices, and select the candidate accelerator. Capture graph/operator placement and output parity on fixed synthetic texts before enabling automatic preference for that device. The app must offer an explicit setup action for model/provider acquisition, since Windows ML's catalog can download providers. See [Windows ML setup](https://learn.microsoft.com/en-us/windows/ai/new-windows-ml/get-started).

Remaining blockers are an unverified NPU inventory, no established compatible NPU driver/provider, no approved and tested ONNX artifact/tokenizer equivalent to the current representation, and no native runtime packaging integration in this Rust/Tauri app. The installed Ollama GGUF model format is not a drop-in ONNX artifact. Adding a provider enum that cannot execute would not address these blockers, so this change adds none. Once a native backend has demonstrated compatible outputs and real placement, its runtime identity can change without silently changing the embedding representation.

## Verification boundary

Focused tests cover invalid and overflowed vectors, mixed dimensions, response model mismatch, cache text/digest/dimension mismatch, unknown device observations, unavailable runtime, and replacement of the model digest during a real loopback HTTP exchange. The identity-race test verifies that neither vectors nor a cache file are published. These tests establish boundary behavior; they are not embedding quality, hardware execution, or performance evidence. Test execution is recorded by the implementation's consolidated verification report.
