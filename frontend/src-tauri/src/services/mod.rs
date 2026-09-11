#[cfg(any(target_os = "android", test))]
pub mod android_bridge;
pub mod atomic_io;
pub mod attachment_store;
pub mod canvas_store;
pub mod chunk_index;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod evidence;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod evidence_bridge;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod evidence_prediction;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod evidence_repair;
pub mod feedback;
pub mod graph_index;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod import;
pub mod index_commit;
pub mod knowledge_store;
pub mod link_discovery;
pub mod markdown_migration;
pub mod memory;
pub mod ollama;
pub mod openrouter;
pub mod priority;
pub mod retrieval;
pub mod root_transition;
pub mod search;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod source_content;
pub mod settings;
pub mod similarity;
pub mod sync;
pub mod texttiling;
pub mod topic_hub;
pub mod twin;
#[cfg(feature = "twin-eval-lab")]
pub mod twin_eval;
pub mod twin_events;
pub mod utf8_chunk;
pub mod vault_namespace;
pub mod vault_optimizer;
pub mod yake;
