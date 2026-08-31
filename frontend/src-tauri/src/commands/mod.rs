//! Canonical lock order: `knowledge_store` before `vault_optimizer`, always.
//!
//! The background vault-optimizer worker (`main.rs::start_vault_optimizer_worker`)
//! acquires both locks *twice per tick*, never holding either across LLM/network
//! work: first `state.knowledge_store.read()` then `state.vault_optimizer.write()`
//! around `VaultOptimizerService::prepare_next` (which resolves the common
//! `sidecar_first` write path and every no-op/error/cap-deferred case using only
//! read access to the vault), and — only when `prepare_next` returns a pending
//! non-`sidecar_first` write — a second, narrower pass with
//! `state.knowledge_store.write()` then `state.vault_optimizer.write()` around
//! `VaultOptimizerService::apply_pending`. Every other call site that needs both
//! locks (e.g. `commands::migration::rollback_vault_optimizer_change`) must
//! acquire `knowledge_store` before `vault_optimizer` in the same way (read or
//! write on `knowledge_store` doesn't matter for ordering — `vault_optimizer`
//! must simply always come second). Acquiring them in reverse order risks an
//! ABBA deadlock: the worker fires every 30s, so a caller that takes
//! `vault_optimizer` first and blocks on `knowledge_store` can cross with the
//! worker holding `knowledge_store` and blocking on `vault_optimizer`, wedging
//! both locks (and every command that touches either) until the app restarts.

pub mod boot;
pub mod canvas;
pub mod distill;
pub mod feedback;
pub mod graph;
pub mod import;
#[cfg(desktop)]
pub mod mcp;
pub mod memory;
pub mod migration;
pub mod notes;
pub mod priority;
pub mod retrieval;
pub mod search;
pub mod settings;
pub mod twin;
pub mod twin_state;
#[cfg(feature = "twin-eval-lab")]
pub mod twin_eval;
pub mod zettelkasten;

use crate::models::note::Note;
use crate::services::index_commit;
use crate::services::retrieval::RetrievalResult;
use crate::AppState;
use std::collections::HashSet;
use tauri::Emitter;

#[derive(Debug)]
pub(crate) struct RootReadTicket {
    _transition_guard: tokio::sync::OwnedRwLockReadGuard<()>,
    authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
    require_ready: bool,
}

impl RootReadTicket {
    pub(crate) fn authority(&self) -> &crate::services::vault_namespace::VaultAuthorityTokenV1 {
        &self.authority
    }

    pub(crate) async fn validate(&self, state: &AppState) -> Result<(), String> {
        let loaded = state.loaded_authority.read().await.clone();
        if self.require_ready && loaded.as_ref() != Some(&self.authority) {
            return Err("Grafyn derived state changed while this read was running".into());
        }
        state
            .mutation_coordinator
            .as_ref()
            .ok_or_else(|| "mutation coordinator is unavailable".to_string())?
            .validate_authority_token(&self.authority, self.require_ready)
            .map_err(|error| error.to_string())
    }

    pub(crate) async fn finish(self, state: &AppState) -> Result<(), String> {
        self.validate(state).await
    }
}

pub(crate) async fn acquire_root_epoch(state: &AppState) -> Result<RootReadTicket, String> {
    let guard = state.vault_transition.clone().read_owned().await;
    ensure_root_healthy(state).await?;
    let coordinator = state
        .mutation_coordinator
        .as_ref()
        .ok_or_else(|| "mutation coordinator is unavailable".to_string())?;
    let authority = coordinator
        .current_authority_token()
        .map_err(|error| error.to_string())?;
    Ok(RootReadTicket {
        _transition_guard: guard,
        authority,
        require_ready: false,
    })
}

pub(crate) async fn acquire_derived_root_epoch(state: &AppState) -> Result<RootReadTicket, String> {
    let mut ticket = acquire_root_epoch(state).await?;
    let coordinator = state
        .mutation_coordinator
        .as_ref()
        .ok_or_else(|| "mutation coordinator is unavailable".to_string())?;
    coordinator
        .validate_authority_token(&ticket.authority, true)
        .map_err(|error| error.to_string())?;
    if state.loaded_authority.read().await.as_ref() != Some(&ticket.authority) {
        return Err("Grafyn derived state is not loaded for the current authority".into());
    }
    ticket.require_ready = true;
    Ok(ticket)
}

pub(crate) async fn ensure_root_healthy(state: &AppState) -> Result<(), String> {
    if let Some(error) = state.mutation_startup_error.read().await.as_ref() {
        return Err(format!(
            "Grafyn storage is unavailable until restart: {error}"
        ));
    }
    Ok(())
}

pub(crate) fn capture_root_epoch(
    state: &AppState,
) -> Result<crate::services::vault_namespace::VaultAuthorityTokenV1, String> {
    state
        .mutation_coordinator
        .as_ref()
        .ok_or_else(|| "mutation coordinator is unavailable".to_string())?
        .current_authority_token()
        .map_err(|error| error.to_string())
}

pub(crate) async fn acquire_expected_root_epoch(
    state: &AppState,
    expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
) -> Result<RootReadTicket, String> {
    let guard = acquire_root_epoch(state).await?;
    state
        .mutation_coordinator
        .as_ref()
        .ok_or_else(|| "mutation coordinator is unavailable".to_string())?
        .validate_root_epoch(expected)
        .map_err(|error| error.to_string())?;
    Ok(guard)
}

pub(crate) async fn acquire_expected_derived_root_epoch(
    state: &AppState,
    expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
) -> Result<RootReadTicket, String> {
    let ticket = acquire_derived_root_epoch(state).await?;
    if ticket.authority() != expected {
        return Err("Grafyn authority changed while derived work was in flight".into());
    }
    Ok(ticket)
}

#[cfg(test)]
mod root_epoch_source_guards {
    fn function_body<'a>(source: &'a str, signature: &str, next_marker: &str) -> &'a str {
        let start = source
            .find(signature)
            .expect("command signature should exist");
        let rest = &source[start..];
        let end = rest.find(next_marker).unwrap_or(rest.len());
        &rest[..end]
    }

    fn tauri_command_body<'a>(source: &'a str, name: &str) -> &'a str {
        let signature = format!("pub async fn {name}");
        let start = source
            .find(&signature)
            .unwrap_or_else(|| panic!("command {name} should exist"));
        let rest = &source[start..];
        let end = rest.find("\n#[tauri::command]").unwrap_or(rest.len());
        &rest[..end]
    }

    #[test]
    fn every_derived_desktop_command_uses_an_exact_before_and_after_authority_ticket() {
        let inventories = [
            (
                include_str!("search.rs"),
                &["search_notes", "find_similar"][..],
            ),
            (
                include_str!("graph.rs"),
                &[
                    "get_backlinks",
                    "get_outgoing",
                    "get_neighbors",
                    "get_unlinked",
                    "get_full_graph",
                ][..],
            ),
            (include_str!("retrieval.rs"), &["retrieve_relevant"][..]),
            (
                include_str!("memory.rs"),
                &["recall_relevant", "find_contradictions"][..],
            ),
            (
                include_str!("migration.rs"),
                &[
                    "preview_markdown_migration",
                    "get_vault_optimizer_status",
                    "list_vault_optimizer_decisions",
                    "get_vault_optimizer_inbox",
                ][..],
            ),
            (
                include_str!("zettelkasten.rs"),
                &["list_link_suggestion_queue", "get_link_discovery_status"][..],
            ),
            (
                include_str!("twin.rs"),
                &[
                    "get_session_trace",
                    "get_twin_review",
                    "resolve_user_record_evidence",
                    "list_decision_episodes",
                    "get_decision_mirror_config",
                    "list_constitution_items",
                    "list_action_gaps",
                    "get_constitution_setup",
                ][..],
            ),
        ];

        for (source, commands) in inventories {
            for name in commands {
                let body = tauri_command_body(source, name);
                assert!(
                    body.contains("acquire_derived_root_epoch"),
                    "derived command {name} must validate exact ready authority before reading"
                );
                assert!(
                    body.contains(".finish(state.inner()).await?"),
                    "derived command {name} must validate authority after releasing service guards"
                );
            }
        }

        let migration = include_str!("migration.rs");
        let status = tauri_command_body(migration, "get_markdown_migration_status");
        assert!(status.contains("acquire_root_epoch"));
        assert!(status.contains("status_scoped"));
        assert!(status.contains("authority()"));
        assert!(!status.contains("acquire_derived_root_epoch"));
    }

    #[test]
    fn authoritative_note_and_twin_reads_use_before_and_after_authority_tickets() {
        for (source, commands) in [
            (include_str!("notes.rs"), &["list_notes", "get_note"][..]),
            (
                include_str!("twin.rs"),
                &["list_user_records", "get_user_record"][..],
            ),
        ] {
            for name in commands {
                let body = tauri_command_body(source, name);
                assert!(
                    body.contains("acquire_root_epoch"),
                    "authoritative command {name} must capture authority before reading"
                );
                assert!(
                    !body.contains("acquire_derived_root_epoch"),
                    "authoritative command {name} must not depend on derived readiness"
                );
                assert!(
                    body.contains(".finish(state.inner()).await?"),
                    "authoritative command {name} must revalidate authority after reading"
                );
                let reload = if *name == "list_notes" || *name == "get_note" {
                    "reload_authoritative_state"
                } else {
                    "rebuild_mutation_caches"
                };
                assert!(
                    body.contains(reload),
                    "authoritative command {name} must refresh durable state after capturing authority"
                );
            }
        }
    }

    #[test]
    fn root_dependent_read_commands_take_the_transition_gate_before_service_locks() {
        let retrieval = include_str!("retrieval.rs");
        for (signature, next_marker) in [
            (
                "pub async fn retrieve_relevant",
                "/// Get current retrieval configuration",
            ),
            (
                "pub async fn get_retrieval_config",
                "/// Update retrieval configuration",
            ),
            ("pub async fn update_retrieval_config", "\n}"),
        ] {
            let body = function_body(retrieval, signature, next_marker);
            let gate = body
                .find("acquire_root_epoch")
                .or_else(|| body.find("acquire_derived_root_epoch"))
                .expect("retrieval command must acquire the root transition gate");
            let service = body
                .find("retrieval_service")
                .or_else(|| body.find("run_retrieval"))
                .expect("retrieval command must access its root-dependent service");
            assert!(
                gate < service,
                "root gate must precede retrieval service access"
            );
        }

        let mcp = include_str!("mcp.rs");
        for (signature, next_marker) in [
            (
                "pub async fn get_mcp_status",
                "/// Get the Claude Desktop config snippet",
            ),
            (
                "pub async fn get_mcp_config_snippet",
                "/// Find the grafyn-mcp binary",
            ),
        ] {
            let body = function_body(mcp, signature, next_marker);
            let gate = body
                .find("acquire_root_epoch")
                .expect("MCP settings read must acquire the root transition gate");
            let settings = body
                .find("settings_service.read")
                .expect("MCP command must read settings");
            assert!(gate < settings, "root gate must precede MCP settings read");
        }

        let settings = include_str!("settings.rs");
        let ollama = function_body(
            settings,
            "async fn capture_ollama_request_authority",
            "async fn finish_ollama_request_authority",
        );
        let gate = ollama
            .find("acquire_root_epoch")
            .expect("Ollama status settings read must acquire the root transition gate");
        let settings_read = ollama
            .find("settings_service.read")
            .expect("Ollama status must read settings");
        assert!(gate < settings_read);
    }

    fn assert_short_commands_are_gated(source: &str, family: &str, exempt: &[&str]) {
        for command in source.split("#[tauri::command]").skip(1) {
            let Some(signature) = command.find("pub async fn ") else {
                continue;
            };
            let name = command[signature + "pub async fn ".len()..]
                .split(|character: char| character == '(' || character.is_whitespace())
                .next()
                .expect("command name");
            if exempt.contains(&name) {
                continue;
            }
            let body = command.split("#[tauri::command]").next().unwrap_or(command);
            assert!(
                body.contains("acquire_root_epoch")
                    || body.contains("acquire_derived_root_epoch")
                    || body.contains("acquire_expected_root_epoch")
                    || body.contains("acquire_expected_derived_root_epoch")
                    || body.contains("run_twin_mutation")
                    || body.contains("persist_canvas_mutation"),
                "root-dependent {family} command {name} must acquire the transition gate"
            );
        }
    }

    #[test]
    fn root_dependent_command_and_worker_inventory_uses_short_gates_or_epoch_revalidation() {
        for (source, family, exempt) in [
            (include_str!("twin.rs"), "Twin", &[][..]),
            (include_str!("memory.rs"), "memory", &[][..]),
            (include_str!("distill.rs"), "distill", &[][..]),
            (include_str!("graph.rs"), "graph", &[][..]),
            (include_str!("notes.rs"), "notes", &[][..]),
            (include_str!("search.rs"), "search", &[][..]),
            (
                include_str!("migration.rs"),
                "migration",
                &["update_vault_optimizer_settings"][..],
            ),
            (include_str!("retrieval.rs"), "retrieval", &[][..]),
            (
                include_str!("canvas/session.rs"),
                "Canvas session",
                &["get_available_models"][..],
            ),
        ] {
            assert_short_commands_are_gated(source, family, exempt);
        }

        let migration = include_str!("migration.rs");
        let optimizer_settings = function_body(
            migration,
            "pub async fn update_vault_optimizer_settings",
            "#[tauri::command]\npub async fn list_vault_optimizer_decisions",
        );
        assert!(
            optimizer_settings.contains("settings::apply_settings_update"),
            "optimizer settings must delegate to the central write-gated settings boundary"
        );

        for (source, family, minimum_pairs) in [
            (include_str!("canvas/streaming.rs"), "Canvas streaming", 3),
            (include_str!("canvas/debate.rs"), "Canvas debate", 2),
            (include_str!("zettelkasten.rs"), "link application", 1),
            (include_str!("twin_eval.rs"), "Twin evaluation", 2),
        ] {
            let captures = source.matches("capture_root_epoch").count()
                + source.matches("authority().clone()").count();
            assert!(
                captures >= minimum_pairs,
                "long {family} workflows must capture the starting root epoch"
            );
            assert!(
                source.matches("acquire_expected_root_epoch").count() >= minimum_pairs,
                "long {family} workflows must revalidate before root-dependent publication"
            );
        }

        let context = include_str!("canvas/context.rs");
        assert!(
            context.matches("acquire_expected_root_epoch").count()
                + context
                    .matches("acquire_expected_derived_root_epoch")
                    .count()
                >= 2,
            "sealed Twin prediction must validate before context reads and persisted results"
        );

        let discovery = include_str!("../services/link_discovery.rs");
        assert!(discovery.contains("discover_for_note_at_epoch"));
        assert!(discovery.contains("acquire_derived_root_epoch"));
        assert!(discovery.contains("acquire_expected_derived_root_epoch"));

        let runtime = include_str!("../lib.rs");
        for required in [
            "acquire_warm_start_root_gate",
            "discover_for_note_at_epoch",
            "acquire_expected_derived_root_epoch",
            "start_vault_optimizer_worker",
        ] {
            assert!(
                runtime.contains(required),
                "root-dependent runtime worker is missing {required}"
            );
        }
    }

    #[test]
    fn long_workflows_chain_exact_authority_tokens_across_their_own_mutations() {
        let streaming = include_str!("canvas/streaming.rs");
        for required in [
            "add_tile_expecting_authority",
            "add_decision_tile_expecting_authority",
            "batch_update_tile_responses_expecting_authority",
            "append_canvas_trace_expecting_authority",
            ".authority_token",
        ] {
            assert!(
                streaming.contains(required),
                "Canvas streaming must carry its exact post-mutation authority via {required}"
            );
        }

        let debate = include_str!("canvas/debate.rs");
        for required in [
            "append_canvas_trace_expecting_authority",
            "update_debate_expecting_authority",
            "mut root_epoch",
            ".authority_token",
        ] {
            assert!(
                debate.contains(required),
                "Canvas debate must carry its exact post-mutation authority via {required}"
            );
        }

        let twin_eval = include_str!("twin_eval.rs");
        assert!(
            twin_eval.matches("acquire_derived_root_epoch").count() >= 2,
            "Twin evaluation must validate ready derived input before network work"
        );
        assert!(
            twin_eval.matches(".finish(state.inner()).await?").count() >= 2,
            "Twin evaluation must discard stale derived input before network work"
        );

        let distill = include_str!("distill.rs");
        for required in [
            "create_note_expecting_authority",
            "update_note_expecting_authority",
            "complete_distill_note_mutation",
            "latest_commit.as_ref()",
            "repair_after_authority_mutation",
        ] {
            assert!(
                distill.contains(required),
                "distillation must chain governed note mutations via {required}"
            );
        }
        assert!(!distill.contains("validate_authority_token(&root_epoch"));

        let links = include_str!("zettelkasten.rs");
        for required in [
            "update_note_expecting_authority",
            "complete_link_note_mutation",
            "latest_commit.as_ref()",
            "repair_after_authority_mutation",
        ] {
            assert!(
                links.contains(required),
                "link application must chain governed note mutations via {required}"
            );
        }
        assert!(!links.contains("validate_authority_token(&root_epoch"));

        assert!(
            streaming.contains("sealed_epoch_receiver.await")
                && streaming.contains("sealed_epoch_sender.send(published_epoch)"),
            "sealed prediction must start from the visible-response publication token"
        );
    }

    #[test]
    fn every_rebuild_captures_then_reloads_authoritative_inputs_before_publication() {
        let runtime = include_str!("../lib.rs");
        let warm_start = function_body(
            runtime,
            "async fn warm_start_services_inner",
            "async fn acquire_warm_start_root_gate",
        );
        let normalize = warm_start.find("sync_topic_hubs(state)").unwrap();
        let capture = warm_start.find("capture_authority_token").unwrap();
        let rebuild = warm_start
            .find("rebuild_authority_with_retained_guard")
            .unwrap();
        assert!(normalize < capture && capture < rebuild);

        let commands = include_str!("mod.rs");
        let rebuild = function_body(
            &commands[commands
                .rfind("pub(crate) fn rebuild_authority_with_retained_guard")
                .expect("shared rebuild helper should exist")..],
            "pub(crate) fn rebuild_authority_with_retained_guard",
            "async fn rebuild_and_publish_current_authority_inner",
        );
        let capture = rebuild.find("capture_authority_token").unwrap();
        let knowledge = rebuild.find("reload_authoritative_state").unwrap();
        let twin = rebuild.find("rebuild_mutation_caches").unwrap();
        let derive = rebuild.find(".search").unwrap();
        let publish = rebuild.find("publish_namespace_ready").unwrap();
        assert!(capture < knowledge && knowledge < twin);
        assert!(twin < derive && derive < publish);

        let repair = function_body(
            &commands[commands
                .rfind("async fn rebuild_and_publish_current_authority_inner")
                .expect("repair coordinator should exist")..],
            "async fn rebuild_and_publish_current_authority_inner",
            "pub(crate) async fn rebuild_and_publish_current_authority",
        );
        let normalize = repair.find("normalize_topic_hubs_only").unwrap();
        let guard = repair.find("begin_root_transition").unwrap();
        let rebuild = repair
            .find("rebuild_authority_with_retained_guard")
            .unwrap();
        assert!(normalize < guard && guard < rebuild);

        let settings = include_str!("settings.rs");
        for capture in settings.match_indices("capture_authority_token") {
            let rest = &settings[capture.0..];
            let derive = rest
                .find("rebuild_indexes_from_notes")
                .expect("root transition capture must be followed by a rebuild");
            let before_derive = &rest[..derive];
            assert!(before_derive.contains("reload_authoritative_state"));
            assert!(before_derive.contains("rebuild_mutation_caches"));
        }
    }

    #[test]
    fn authoritative_mutation_families_use_the_central_post_commit_repair_seam() {
        for (family, source) in [
            ("notes", include_str!("notes.rs")),
            ("imports", include_str!("import.rs")),
            ("zettelkasten", include_str!("zettelkasten.rs")),
            ("distill", include_str!("distill.rs")),
            ("Twin", include_str!("twin.rs")),
            ("Canvas sessions", include_str!("canvas/session.rs")),
            ("Canvas streaming", include_str!("canvas/streaming.rs")),
            ("Canvas debate", include_str!("canvas/debate.rs")),
            ("migration", include_str!("migration.rs")),
            ("optimizer worker", include_str!("../lib.rs")),
        ] {
            assert!(
                source.contains("repair_after_authority_"),
                "{family} must route committed authority through the central repair seam"
            );
            assert!(
                !source.contains("commit_note_writes("),
                "{family} must not end at the legacy partial note-index refresh"
            );
        }
    }

    #[test]
    fn canvas_inputs_and_ollama_network_results_are_exact_authority_fenced() {
        let sessions = include_str!("canvas/session.rs");
        for name in ["list_sessions", "get_session"] {
            let body = tauri_command_body(sessions, name);
            assert!(body.contains("reload_authoritative_state"));
            assert!(body.contains(".finish(state.inner()).await?"));
        }

        let streaming = include_str!("canvas/streaming.rs");
        assert!(streaming.matches("reload_authoritative_state").count() >= 3);
        assert!(streaming.matches("root_ticket.validate").count() >= 3);
        let twin = include_str!("twin.rs");
        for name in ["update_decision_outcome", "record_canvas_feedback"] {
            let body = tauri_command_body(twin, name);
            assert!(body.contains("reload_authoritative_state"));
            assert!(body.contains("root_ticket.validate"));
        }

        let settings = include_str!("settings.rs");
        for name in ["get_ollama_status", "list_ollama_models"] {
            let body = tauri_command_body(settings, name);
            assert!(body.contains("capture_ollama_request_authority"));
            assert!(body.contains("finish_ollama_request_authority"));
        }
    }

    #[test]
    fn every_prediction_abandonment_uses_idempotent_same_root_terminalization() {
        let context = include_str!("canvas/context.rs");
        let terminality = include_str!("canvas/prediction_terminality.rs");
        let streaming = include_str!("canvas/streaming.rs");
        assert!(terminality.contains("is_same_prediction_root"));
        assert!(
            context
                .matches("fail_requested_prediction_if_same_root")
                .count()
                >= 7
        );
        assert!(
            streaming
                .matches("fail_requested_prediction_if_same_root")
                .count()
                >= 8
        );
        assert!(streaming.contains("visible response channel closed"));
        assert!(streaming.contains("visible response authority repair"));
        assert!(context.contains("post-network authority validation"));
        assert!(context.contains("provider failure"));
    }
}

/// Shared retrieval helper — acquires the 4 retrieval-pipeline read locks, calls
/// `retrieval.retrieve()`, and releases all locks before returning. Used by
/// `retrieve_relevant`, `recall_relevant`, and the canvas context resolvers to
/// avoid duplicating the same lock sequence in each caller.
pub(crate) async fn run_retrieval(
    state: &AppState,
    query: &str,
    limit: usize,
    context_ids: &[String],
) -> Result<Vec<RetrievalResult>, String> {
    let search = state.search_service.read().await;
    let graph = state.graph_index.read().await;
    let priority = state.priority_service.read().await;
    let retrieval = state.retrieval_service.read().await;
    retrieval.retrieve(&search, &graph, &priority, query, limit, context_ids)
}

pub(crate) async fn sync_chunk_index_for_note(state: &AppState, note: &Note) {
    sync_chunk_index_for_notes(state, std::slice::from_ref(note)).await;
}

pub(crate) async fn sync_chunk_index_for_notes(state: &AppState, notes: &[Note]) {
    if notes.is_empty() {
        return;
    }

    let mut chunks = state.chunk_index.write().await;
    for note in notes {
        if let Err(error) = chunks.index_note_chunks(note) {
            log::error!("Failed to index chunks for note '{}': {}", note.id, error);
        }
    }
    if let Err(error) = chunks.commit() {
        log::error!("Failed to commit chunk index: {}", error);
    }
}

pub(crate) async fn remove_note_chunks_from_index(state: &AppState, note_id: &str) {
    let mut chunks = state.chunk_index.write().await;
    if let Err(error) = chunks.remove_note_chunks(note_id) {
        log::error!("Failed to remove chunks for note '{}': {}", note_id, error);
    }
    if let Err(error) = chunks.commit() {
        log::error!("Failed to commit chunk index: {}", error);
    }
}

pub(crate) async fn rebuild_link_discovery(state: &AppState, notes: &[Note]) -> Result<(), String> {
    let coordinator = state
        .mutation_coordinator
        .as_ref()
        .ok_or_else(|| "mutation coordinator is unavailable".to_string())?;
    let token = coordinator
        .current_authority_token()
        .map_err(|error| error.to_string())?;
    rebuild_link_discovery_at(state, notes, &token).await
}

async fn rebuild_link_discovery_at(
    state: &AppState,
    notes: &[Note],
    token: &crate::services::vault_namespace::VaultAuthorityTokenV1,
) -> Result<(), String> {
    let coordinator = state
        .mutation_coordinator
        .as_ref()
        .ok_or_else(|| "mutation coordinator is unavailable".to_string())?;
    let mut discovery = state.link_discovery.write().await;
    coordinator
        .with_locked_derived_state(token, false, || {
            discovery.reload_from_disk_checked().map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })?;
            discovery.bootstrap_checked(notes).map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })
        })
        .map_err(|error| error.to_string())
}

pub(crate) async fn bootstrap_vault_optimizer(
    state: &AppState,
    notes: &[Note],
) -> Result<(), String> {
    let mut optimizer = state.vault_optimizer.write().await;
    optimizer
        .with_locked_fresh_state(|optimizer| optimizer.bootstrap_checked(notes))
        .map_err(|error| error.to_string())
}

pub(crate) async fn enqueue_vault_optimizer_note(
    state: &AppState,
    note_id: &str,
    reason: &str,
) -> Result<(), String> {
    let mut optimizer = state.vault_optimizer.write().await;
    optimizer
        .with_locked_fresh_state(|optimizer| {
            optimizer.enqueue_note_checked(note_id, reason).map(|_| ())
        })
        .map_err(|error| error.to_string())
}

pub(crate) async fn remove_link_discovery_note(
    state: &AppState,
    note_id: &str,
) -> Result<(), String> {
    let coordinator = state
        .mutation_coordinator
        .as_ref()
        .ok_or_else(|| "mutation coordinator is unavailable".to_string())?;
    let token = coordinator
        .current_authority_token()
        .map_err(|error| error.to_string())?;
    let mut discovery = state.link_discovery.write().await;
    coordinator
        .with_locked_derived_state(&token, false, || {
            discovery.reload_from_disk_checked().map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })?;
            discovery.remove_note_checked(note_id).map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })
        })
        .map_err(|error| error.to_string())
}

async fn normalize_topic_hubs_only(
    state: &AppState,
    expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
) -> anyhow::Result<crate::services::topic_hub::TopicHubSyncResult> {
    let mut store = state.knowledge_store.write().await;
    crate::services::topic_hub::sync_topic_hubs_expecting_authority(&mut store, expected)
}

#[derive(Debug)]
pub(crate) struct TopicHubCommandSync {
    pub notes: Vec<Note>,
    pub latest_commit: Option<crate::services::twin_events::MutationCommit>,
    pub continuation_authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
}

pub(crate) async fn sync_topic_hubs_at_authority(
    state: &AppState,
    expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
) -> Result<TopicHubCommandSync, String> {
    let normalization = normalize_topic_hubs_only(state, expected.clone()).await;
    let (sync_result, latest_commit, continuation_authority) = match normalization {
        Ok(mut sync_result) => match sync_result.latest_commit.take() {
            Some(commit) => {
                let repair =
                    repair_after_authority_mutation(state, &commit, "topic hub normalization")
                        .await;
                let continuation_authority = match repair {
                    PostAuthorityRepair::Ready(authority) => authority,
                    PostAuthorityRepair::NotRequired | PostAuthorityRepair::Unavailable(_) => {
                        return Err(
                            "topic hub normalization committed and recovery is pending; do not retry"
                                .to_string(),
                        );
                    }
                };
                sync_result.all_notes = {
                    let mut store = state.knowledge_store.write().await;
                    store.reload_authoritative_state();
                    store.list_full_notes().map_err(|error| error.to_string())?
                };
                (sync_result, Some(commit), continuation_authority)
            }
            None => (sync_result, None, expected),
        },
        Err(error) => {
            if let Some(commit) = crate::services::topic_hub::topic_hub_partial_commit(&error) {
                let repair =
                    repair_after_authority_mutation(state, &commit, "topic hub normalization")
                        .await;
                return Err(match repair {
                    PostAuthorityRepair::Ready(_) => format!(
                        "topic hub normalization partially committed before a later failure: {error}; do not retry automatically"
                    ),
                    PostAuthorityRepair::NotRequired | PostAuthorityRepair::Unavailable(_) => {
                        format!(
                            "topic hub normalization partially committed and recovery is pending after a later failure: {error}; do not retry"
                        )
                    }
                });
            }
            let (outcome, continuation_authority) =
                finish_knowledge_authority_error(state, &error, "topic hub normalization").await?;
            let all_notes = {
                let mut store = state.knowledge_store.write().await;
                store.reload_authoritative_state();
                store.list_full_notes().map_err(|error| error.to_string())?
            };
            (
                crate::services::topic_hub::TopicHubSyncResult {
                    all_notes,
                    changed_note_ids: Vec::new(),
                    removed_note_ids: Vec::new(),
                    latest_commit: None,
                },
                Some(outcome.commit),
                continuation_authority,
            )
        }
    };

    if latest_commit.is_some() {
        return Ok(TopicHubCommandSync {
            notes: sync_result.all_notes,
            latest_commit,
            continuation_authority,
        });
    }

    let changed_ids = sync_result
        .changed_note_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let changed_notes = sync_result
        .all_notes
        .iter()
        .filter(|note| changed_ids.contains(&note.id))
        .cloned()
        .collect::<Vec<_>>();

    for removed_note_id in &sync_result.removed_note_ids {
        remove_note_chunks_from_index(state, removed_note_id).await;
        remove_link_discovery_note(state, removed_note_id).await?;

        let mut search = state.search_service.write().await;
        if let Err(error) = index_commit::remove_note_for_search(&mut search, removed_note_id) {
            log::error!(
                "Failed to remove topic-hub note '{}' from search index: {}",
                removed_note_id,
                error
            );
        }
        if let Err(error) = index_commit::commit_search(&mut search) {
            log::error!(
                "Failed to commit search index after topic-hub removal: {}",
                error
            );
        }
    }

    if !changed_notes.is_empty() {
        {
            let mut search = state.search_service.write().await;
            for note in &changed_notes {
                if let Err(error) = index_commit::index_note_for_search(&mut search, note) {
                    log::error!("Failed to index topic-hub note '{}': {}", note.id, error);
                }
            }
            if let Err(error) = index_commit::commit_search(&mut search) {
                log::error!("Failed to commit search index after topic sync: {}", error);
            }
        }

        sync_chunk_index_for_notes(state, &changed_notes).await;
    }

    {
        let mut graph = state.graph_index.write().await;
        graph.build_from_notes(&sync_result.all_notes);
    }

    rebuild_link_discovery(state, &sync_result.all_notes).await?;
    bootstrap_vault_optimizer(state, &sync_result.all_notes).await?;

    Ok(TopicHubCommandSync {
        notes: sync_result.all_notes,
        latest_commit,
        continuation_authority,
    })
}

pub(crate) async fn sync_topic_hubs(state: &AppState) -> Result<Vec<Note>, String> {
    let expected = capture_root_epoch(state)?;
    sync_topic_hubs_at_authority(state, expected)
        .await
        .map(|result| result.notes)
}

pub(crate) async fn rebuild_all_indexes(
    state: &AppState,
    expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
) -> Result<TopicHubCommandSync, String> {
    let sync = sync_topic_hubs_at_authority(state, expected).await?;
    let notes = &sync.notes;

    {
        let mut search = state.search_service.write().await;
        search
            .reindex_all(&notes)
            .map_err(|error| error.to_string())?;
    }

    {
        let mut graph = state.graph_index.write().await;
        graph.build_from_notes(&notes);
    }

    {
        let mut chunks = state.chunk_index.write().await;
        if let Err(error) = chunks.reindex_all(&notes) {
            log::error!("Failed to rebuild chunk index: {}", error);
        }
    }

    bootstrap_vault_optimizer(state, &notes).await?;
    Ok(sync)
}

/// Rebuild every derived reader against one exact authority generation and
/// publish readiness only if that generation is still current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorityRepairStep {
    Normalized,
    Captured,
    TwinCaches,
    Search,
    Chunk,
    Graph,
    LinkDiscovery,
    Optimizer,
}

/// Every local guard used by a complete rebuild. They are acquired before the
/// cross-process coordinator guard so a note/Twin mutation that already holds
/// its service lock while committing cannot form an ABBA deadlock with repair.
pub(crate) struct AuthorityRepairGuards<'a> {
    knowledge: tokio::sync::RwLockWriteGuard<'a, crate::services::knowledge_store::KnowledgeStore>,
    twin: tokio::sync::RwLockWriteGuard<'a, crate::services::twin::TwinStore>,
    search: tokio::sync::RwLockWriteGuard<'a, crate::services::search::SearchService>,
    chunks: tokio::sync::RwLockWriteGuard<'a, crate::services::chunk_index::ChunkIndex>,
    graph: tokio::sync::RwLockWriteGuard<'a, crate::services::graph_index::GraphIndex>,
    discovery:
        tokio::sync::RwLockWriteGuard<'a, crate::services::link_discovery::LinkDiscoveryService>,
    optimizer:
        tokio::sync::RwLockWriteGuard<'a, crate::services::vault_optimizer::VaultOptimizerService>,
    _optimizer_state_lock: crate::services::twin_events::AnchoredExclusiveLock,
    loaded: tokio::sync::RwLockWriteGuard<
        'a,
        Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    >,
}

pub(crate) async fn acquire_authority_repair_guards(
    state: &AppState,
) -> Result<AuthorityRepairGuards<'_>, String> {
    // Canonical order. In particular Knowledge always precedes Optimizer.
    let knowledge = state.knowledge_store.write().await;
    let twin = state.twin_store.write().await;
    let search = state.search_service.write().await;
    let chunks = state.chunk_index.write().await;
    let graph = state.graph_index.write().await;
    let discovery = state.link_discovery.write().await;
    let optimizer = state.vault_optimizer.write().await;
    let optimizer_state_lock = optimizer
        .acquire_state_lock()
        .map_err(|error| error.to_string())?;
    let loaded = state.loaded_authority.write().await;
    Ok(AuthorityRepairGuards {
        knowledge,
        twin,
        search,
        chunks,
        graph,
        discovery,
        optimizer,
        _optimizer_state_lock: optimizer_state_lock,
        loaded,
    })
}

/// Rebuilds and publishes while `guard` continuously owns the shared process
/// lock. This function is deliberately synchronous: all Tokio service guards
/// were acquired first, and no local lock await is permitted after this point.
pub(crate) fn rebuild_authority_with_retained_guard(
    guards: &mut AuthorityRepairGuards<'_>,
    guard: &crate::services::twin_events::MutationRootTransitionGuard<'_>,
    expected_root: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    mut checkpoint: impl FnMut(AuthorityRepairStep) -> Result<(), String>,
) -> Result<crate::services::vault_namespace::VaultAuthorityTokenV1, String> {
    let lease = guard.current_lease().map_err(|error| error.to_string())?;
    let token = guard
        .capture_authority_token(&lease)
        .map_err(|error| error.to_string())?;
    if token.root_scope != expected_root.root_scope
        || token.lease_epoch_uuid != expected_root.lease_epoch_uuid
    {
        return Err("vault root changed before authority repair".into());
    }
    if guards.loaded.as_ref().is_some_and(|loaded| {
        loaded.root_scope == token.root_scope
            && loaded.lease_epoch_uuid == token.lease_epoch_uuid
            && loaded.authority_generation > token.authority_generation
    }) {
        return Err("loaded authority is newer than the durable repair token".into());
    }
    *guards.loaded = None;
    guard
        .invalidate_namespace(&lease)
        .map_err(|error| error.to_string())?;
    checkpoint(AuthorityRepairStep::Captured)?;

    guards.knowledge.reload_authoritative_state();
    let notes = guards
        .knowledge
        .list_full_notes()
        .map_err(|error| error.to_string())?;
    guards
        .twin
        .rebuild_mutation_caches()
        .map_err(|error| error.to_string())?;
    checkpoint(AuthorityRepairStep::TwinCaches)?;
    guard
        .validate_authority_token(&token, false)
        .map_err(|error| error.to_string())?;

    guards
        .search
        .reindex_all(&notes)
        .map_err(|error| error.to_string())?;
    checkpoint(AuthorityRepairStep::Search)?;
    guards
        .chunks
        .reindex_all(&notes)
        .map_err(|error| error.to_string())?;
    checkpoint(AuthorityRepairStep::Chunk)?;
    guards.graph.build_from_notes(&notes);
    checkpoint(AuthorityRepairStep::Graph)?;
    guards
        .discovery
        .reload_from_disk_checked()
        .map_err(|error| error.to_string())?;
    guards
        .discovery
        .bootstrap_checked(&notes)
        .map_err(|error| error.to_string())?;
    checkpoint(AuthorityRepairStep::LinkDiscovery)?;
    guards
        .optimizer
        .reload_from_disk_checked()
        .map_err(|error| error.to_string())?;
    guards
        .optimizer
        .recover_pending_publications_locked(&guards.knowledge, guard)
        .map_err(|error| error.to_string())?;
    guards
        .optimizer
        .bootstrap_checked(&notes)
        .map_err(|error| error.to_string())?;
    checkpoint(AuthorityRepairStep::Optimizer)?;

    guard
        .validate_authority_token(&token, false)
        .map_err(|error| error.to_string())?;
    guard
        .publish_namespace_ready(&token)
        .map_err(|error| error.to_string())?;
    *guards.loaded = Some(token.clone());
    Ok(token)
}

async fn rebuild_and_publish_current_authority_inner(
    state: &AppState,
    expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    normalize_hubs: bool,
    mut checkpoint: impl FnMut(AuthorityRepairStep) -> Result<(), String>,
) -> Result<crate::services::vault_namespace::VaultAuthorityTokenV1, String> {
    let _repair = state.authority_repair.lock().await;
    let coordinator = state
        .mutation_coordinator
        .as_ref()
        .ok_or_else(|| "mutation coordinator is unavailable".to_string())?;

    // Fail closed before WAL replay and authoritative normalization. A replay
    // failure must never leave an old ready marker visible to another process.
    // Normalization may itself commit, so no process guard is retained across
    // it; unlike `sync_topic_hubs`, this path publishes no derived state.
    {
        let mut loaded = state.loaded_authority.write().await;
        *loaded = None;
        coordinator
            .invalidate_namespace_before_recovery(expected)
            .map_err(|error| error.to_string())?;
    }
    coordinator
        .recover_pending()
        .map_err(|error| error.to_string())?;
    // Replay can create, replace, rename, or delete note files after the
    // KnowledgeStore last refreshed its indexes. Normalization reads through
    // those indexes, so refresh the authoritative cache before it observes the
    // recovered target set.
    state
        .knowledge_store
        .write()
        .await
        .reload_authoritative_state();
    let mut rebuild_expected = expected.clone();
    if normalize_hubs {
        for attempt in 0..2 {
            match normalize_topic_hubs_only(state, rebuild_expected.clone()).await {
                Ok(mut result) => {
                    if let Some(commit) = result.latest_commit.take() {
                        rebuild_expected = commit.authority_token.ok_or_else(|| {
                            "topic hub normalization lost its authority token".to_string()
                        })?;
                    }
                    break;
                }
                Err(error) => {
                    if let Some(commit) =
                        crate::services::topic_hub::topic_hub_partial_commit(&error)
                    {
                        rebuild_expected = commit.authority_token.ok_or_else(|| {
                            "topic hub normalization lost its partial authority token".to_string()
                        })?;
                        state
                            .knowledge_store
                            .write()
                            .await
                            .reload_authoritative_state();
                        if attempt == 1 {
                            return Err(format!(
                                "topic hub normalization partially committed and could not converge: {error}"
                            ));
                        }
                        continue;
                    }
                    let outcome =
                        crate::services::knowledge_store::knowledge_authority_advanced_outcome(
                            &error,
                        )
                        .ok_or_else(|| error.to_string())?;
                    rebuild_expected = outcome.commit.authority_token.clone().ok_or_else(|| {
                        "topic hub normalization lost its recovery authority token".to_string()
                    })?;
                    coordinator
                        .recover_pending()
                        .map_err(|recovery_error| recovery_error.to_string())?;
                    state
                        .knowledge_store
                        .write()
                        .await
                        .reload_authoritative_state();
                    if outcome.target_aborted {
                        return Err(
                            "topic hub normalization target was not applied after authority changed"
                                .to_string(),
                        );
                    }
                    if attempt == 1 {
                        return Err(
                            "topic hub normalization committed and recovery remains pending"
                                .to_string(),
                        );
                    }
                }
            }
        }
    }
    checkpoint(AuthorityRepairStep::Normalized)?;
    let mut guards = acquire_authority_repair_guards(state).await?;
    let process_guard = coordinator
        .begin_root_transition()
        .map_err(|error| error.to_string())?;
    rebuild_authority_with_retained_guard(
        &mut guards,
        &process_guard,
        &rebuild_expected,
        checkpoint,
    )
}

pub(crate) async fn rebuild_and_publish_current_authority(
    state: &AppState,
    expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
) -> Result<crate::services::vault_namespace::VaultAuthorityTokenV1, String> {
    rebuild_and_publish_current_authority_inner(state, expected, true, |_| Ok(())).await
}

#[cfg(test)]
pub(crate) async fn rebuild_and_publish_current_authority_with_checkpoint(
    state: &AppState,
    expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    checkpoint: impl FnMut(AuthorityRepairStep) -> Result<(), String>,
) -> Result<crate::services::vault_namespace::VaultAuthorityTokenV1, String> {
    rebuild_and_publish_current_authority_inner(state, expected, true, checkpoint).await
}

async fn rebuild_and_publish_migration_authority(
    state: &AppState,
    expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
) -> Result<crate::services::vault_namespace::VaultAuthorityTokenV1, String> {
    // Migration manifests bind exact after-images. Topic-hub normalization is
    // itself authoritative and would invalidate those rollback proofs.
    rebuild_and_publish_current_authority_inner(state, expected, false, |_| Ok(())).await
}

#[must_use = "committed mutations must inspect and propagate repair readiness"]
#[derive(Debug)]
pub(crate) enum PostAuthorityRepair {
    NotRequired,
    Ready(crate::services::vault_namespace::VaultAuthorityTokenV1),
    Unavailable(crate::models::mutation::CommittedMutationWarningV1),
}

#[derive(Debug)]
pub(crate) struct CompletedKnowledgeNoteMutation {
    pub note: Note,
    pub commit: Option<crate::services::twin_events::MutationCommit>,
    pub continuation_authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
    pub repaired: bool,
}

pub(crate) async fn finish_knowledge_authority_error(
    state: &AppState,
    error: &anyhow::Error,
    operation: &str,
) -> Result<
    (
        crate::services::knowledge_store::KnowledgeAuthorityAdvancedOutcome,
        crate::services::vault_namespace::VaultAuthorityTokenV1,
    ),
    String,
> {
    let outcome = crate::services::knowledge_store::knowledge_authority_advanced_outcome(error)
        .ok_or_else(|| error.to_string())?;
    let repair = repair_after_authority_mutation(state, &outcome.commit, operation).await;
    if outcome.target_aborted {
        return Err(format!(
            "{operation} was not applied after vault authority changed; refresh state before deciding whether to retry"
        ));
    }
    match repair {
        PostAuthorityRepair::Ready(authority) => Ok((outcome, authority)),
        PostAuthorityRepair::NotRequired | PostAuthorityRepair::Unavailable(_) => Err(format!(
            "{operation} committed and recovery is pending; do not retry"
        )),
    }
}

pub(crate) async fn complete_knowledge_note_mutation(
    state: &AppState,
    expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    prior_commit: Option<&crate::services::twin_events::MutationCommit>,
    mutation: anyhow::Result<(Note, crate::services::twin_events::MutationCommit)>,
    operation: &str,
) -> Result<CompletedKnowledgeNoteMutation, String> {
    match mutation {
        Ok((note, commit)) => {
            let continuation_authority = commit
                .authority_token
                .clone()
                .unwrap_or_else(|| expected.clone());
            let commit = commit.authority_token.is_some().then_some(commit);
            Ok(CompletedKnowledgeNoteMutation {
                note,
                commit,
                continuation_authority,
                repaired: false,
            })
        }
        Err(error) => {
            let authority_outcome =
                crate::services::knowledge_store::knowledge_authority_advanced_outcome(&error);
            if let (Some(outcome), Some(_)) = (authority_outcome.as_ref(), prior_commit) {
                if outcome.target_aborted {
                    let repair =
                        repair_after_authority_mutation(state, &outcome.commit, operation).await;
                    return Err(match repair {
                        PostAuthorityRepair::Ready(_) => format!(
                            "{operation} partially committed before a later target was aborted after authority advanced; do not retry automatically"
                        ),
                        PostAuthorityRepair::NotRequired | PostAuthorityRepair::Unavailable(_) => {
                            format!(
                                "{operation} partially committed and recovery is pending after a later target was aborted; do not retry"
                            )
                        }
                    });
                }
            }
            if authority_outcome.is_none() {
                if let Some(prior_commit) = prior_commit {
                    let repair =
                        repair_after_authority_mutation(state, prior_commit, operation).await;
                    return Err(match repair {
                        PostAuthorityRepair::Ready(_) => format!(
                            "{operation} partially committed before a later failure: {error}; do not retry automatically"
                        ),
                        PostAuthorityRepair::NotRequired | PostAuthorityRepair::Unavailable(_) => {
                            format!(
                                "{operation} partially committed and recovery is pending after a later failure: {error}; do not retry"
                            )
                        }
                    });
                }
                return Err(error.to_string());
            }
            let (outcome, continuation_authority) =
                finish_knowledge_authority_error(state, &error, operation).await?;
            let note_id = outcome.note_ids.first().ok_or_else(|| {
                format!("{operation} committed without its result identity; do not retry")
            })?;
            let note = state
                .knowledge_store
                .read()
                .await
                .get_note(note_id)
                .map_err(|load_error| {
                    log::error!(
                        "Recovered {operation} result '{note_id}' could not be loaded: {load_error}"
                    );
                    format!("{operation} committed and recovered; do not retry")
                })?;
            Ok(CompletedKnowledgeNoteMutation {
                note,
                commit: Some(outcome.commit),
                continuation_authority,
                repaired: true,
            })
        }
    }
}

/// Consume a repair result at command boundaries whose public response cannot
/// carry a warning. `Unavailable` has already published the sanitized global
/// committed-warning event, so acknowledging it here preserves the successful
/// mutation response without inviting a duplicate retry.
pub(crate) fn acknowledge_reported_repair(repair: PostAuthorityRepair) {
    match repair {
        PostAuthorityRepair::NotRequired
        | PostAuthorityRepair::Ready(_)
        | PostAuthorityRepair::Unavailable(_) => {}
    }
}

pub(crate) fn publish_committed_warning(
    state: &AppState,
) -> crate::models::mutation::CommittedMutationWarningV1 {
    let warning = crate::models::mutation::CommittedMutationWarningV1::derived_state_unavailable();
    if let Some(app) = state.committed_warning_app.as_ref() {
        if let Err(error) = app.emit(
            crate::models::mutation::COMMITTED_WARNING_EVENT,
            warning.clone(),
        ) {
            log::warn!("Failed to emit committed mutation warning: {error}");
        }
    }
    warning
}

/// Repairs every desktop derived reader after a durable authority mutation.
/// The mutation is already committed, so repair failure is surfaced as a
/// warning and leaves readiness unavailable instead of inviting a duplicate
/// retry of the authoritative write.
pub(crate) async fn repair_after_authority_token(
    state: &AppState,
    expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    operation: &str,
) -> PostAuthorityRepair {
    match rebuild_and_publish_current_authority(state, expected).await {
        Ok(token) => PostAuthorityRepair::Ready(token),
        Err(error) => {
            // A prior repair may fail after a newer repair has already
            // published. Never clear that newer loaded token.
            let mut loaded = state.loaded_authority.write().await;
            if loaded.as_ref() == Some(expected) {
                *loaded = None;
            }
            drop(loaded);
            log::error!(
                "{operation} committed but authority repair failed; readiness remains unavailable: {error}"
            );
            PostAuthorityRepair::Unavailable(publish_committed_warning(state))
        }
    }
}

pub(crate) async fn repair_after_authority_mutation(
    state: &AppState,
    commit: &crate::services::twin_events::MutationCommit,
    operation: &str,
) -> PostAuthorityRepair {
    let Some(expected) = commit.authority_token.as_ref() else {
        return PostAuthorityRepair::NotRequired;
    };
    repair_after_authority_token(state, expected, operation).await
}

pub(crate) async fn repair_after_migration_authority_token(
    state: &AppState,
    expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    operation: &str,
) -> PostAuthorityRepair {
    match rebuild_and_publish_migration_authority(state, expected).await {
        Ok(token) => PostAuthorityRepair::Ready(token),
        Err(error) => {
            let mut loaded = state.loaded_authority.write().await;
            if loaded.as_ref() == Some(expected) {
                *loaded = None;
            }
            drop(loaded);
            log::error!(
                "{operation} committed but authority repair failed; readiness remains unavailable: {error}"
            );
            PostAuthorityRepair::Unavailable(publish_committed_warning(state))
        }
    }
}

/// Single chokepoint for "a note was just created or edited on disk and needs to
/// become visible everywhere else": the search index, the chunk index, topic hubs
/// (which also rebuilds the graph + link-discovery bootstrap), and the vault
/// optimizer queue.
///
/// Before this helper existed, five call sites (`notes::create_note`,
/// `notes::update_note`, `zettelkasten::apply_links`, `zettelkasten::create_link`,
/// plus ad-hoc copies in `canvas::export_to_note`, `import.rs`, `distill.rs`)
/// each hand-rolled a subset of this sequence. `notes.rs` and `zettelkasten.rs`
/// called only `sync_topic_hubs` + `enqueue_vault_optimizer_note` — but
/// `sync_topic_hubs` only reindexes notes whose *hub* metadata changed, so a
/// plain content edit was never reindexed into search or chunks until the next
/// full rebuild (app restart). This was the reindex regression. The other three
/// sites got search+chunk indexing right but never synced topic hubs or
/// enqueued the vault optimizer.
///
/// `reason` is forwarded to `enqueue_vault_optimizer_note` for its audit trail
/// (e.g. `"note_created"`, `"links_applied"`).
///
/// Returns the full up-to-date note list from `sync_topic_hubs` (the same
/// value `sync_topic_hubs` itself returns) so callers that need it — e.g.
/// `distill::distill_note`, which builds hub-update summaries from it — don't
/// have to call `sync_topic_hubs` a second time. Callers that don't need it
/// simply discard the return value.
///
/// Error handling: search/chunk indexing failures are logged and do not abort
/// the call, matching what all the previously-correct call sites already did —
/// the note is already durably written to disk by the time this runs, so
/// losing that fact over an index hiccup would be worse than a temporarily
/// stale index (which self-heals on the next `rebuild_all_indexes`).
/// `sync_topic_hubs` failures do propagate, matching the pre-existing behavior
/// of `notes::create_note` / `notes::update_note` / the zettelkasten commands.
#[cfg(test)]
pub(crate) async fn commit_note_write(
    state: &AppState,
    note_id: &str,
    reason: &str,
) -> Result<Vec<Note>, String> {
    commit_note_writes(state, std::slice::from_ref(&note_id.to_string()), reason).await
}

/// Batch form of [`commit_note_write`] — indexes every note individually but
/// runs the (expensive, full-vault) `sync_topic_hubs` pass only once. Used by
/// call sites that write several notes in one command (conversation/document
/// import, zettelkasten's bidirectional link updates, distillation) so the
/// cost doesn't scale with the number of notes touched.
///
/// `sync_topic_hubs` always runs, even if `note_ids` is empty — callers such
/// as `distill_note` rely on getting the full post-sync note list back
/// regardless of whether this particular call touched any notes.
#[cfg(test)]
pub(crate) async fn commit_note_writes(
    state: &AppState,
    note_ids: &[String],
    reason: &str,
) -> Result<Vec<Note>, String> {
    let mut notes = Vec::with_capacity(note_ids.len());
    {
        let store = state.knowledge_store.read().await;
        for note_id in note_ids {
            match store.get_note(note_id) {
                Ok(note) => notes.push(note),
                Err(error) => {
                    log::error!(
                        "commit_note_write: note '{}' could not be read for indexing: {}",
                        note_id,
                        error
                    );
                }
            }
        }
    }

    if !notes.is_empty() {
        let mut search = state.search_service.write().await;
        for note in &notes {
            if let Err(error) = index_commit::index_note_for_search(&mut search, note) {
                log::error!("Failed to index note '{}': {}", note.id, error);
            }
        }
        if let Err(error) = index_commit::commit_search(&mut search) {
            log::error!(
                "Failed to commit search index after note write(s): {}",
                error
            );
        }
    }

    sync_chunk_index_for_notes(state, &notes).await;
    sync_topic_hubs(state).await?;

    for note_id in note_ids {
        enqueue_vault_optimizer_note(state, note_id, reason).await?;
    }
    let coordinator = state
        .mutation_coordinator
        .as_ref()
        .ok_or_else(|| "mutation coordinator is unavailable".to_string())?;
    let expected = coordinator
        .current_authority_token()
        .map_err(|error| error.to_string())?;
    let published = rebuild_and_publish_current_authority(state, &expected).await?;
    let ticket = acquire_expected_root_epoch(state, &published).await?;
    let fresh_notes = {
        let mut store = state.knowledge_store.write().await;
        store.reload_authoritative_state();
        store.list_full_notes().map_err(|error| error.to_string())?
    };
    ticket.finish(state).await?;
    Ok(fresh_notes)
}

/// Refreshes search + chunk + topic-hub/graph state for a note that the
/// background vault optimizer worker (`main.rs::start_vault_optimizer_worker`)
/// just wrote to disk directly — bypassing `commit_note_write`/
/// `commit_note_writes` on purpose. See the design note below.
///
/// ## Why this exists (Part A of the 2026-07 optimizer-reindex fix)
///
/// The optimizer's two write paths — the `sidecar_first` overlay write
/// inside `VaultOptimizerService::prepare_next`, and the `full_rewrite`
/// `KnowledgeStore::update_note` inside `apply_pending` — change
/// tags/aliases/properties. Those changes ARE visible through
/// `KnowledgeStore::get_note` (overlays merge on read), and they DO matter to
/// search ranking, chunk content, and topic-hub membership — but neither
/// write path ever touched an index. The change was invisible to
/// `search_notes`/`get_backlinks` until the next full `rebuild_all_indexes`
/// (i.e. an app restart). That is the bug this function fixes.
///
/// ## Why a narrower helper instead of just calling `commit_note_write`
///
/// The obvious first idea is: call `commit_note_write(state, note_id, reason)`
/// from the worker like every other write site does. That does not work
/// cleanly, because `commit_note_write` unconditionally calls
/// `enqueue_vault_optimizer_note` — which would re-enqueue the exact note the
/// optimizer just finished processing (`finalize_applied_change` already
/// removed it from the queue before this runs; `enqueue_note` has no memory
/// of "we were the one who just wrote this").
///
/// That re-enqueue is NOT a runaway loop: on the next worker tick,
/// `build_optimizer_proposal` computes its proposal from the note's CURRENT
/// (already-updated) tags/aliases/properties, so the delta is empty,
/// `prepare_next` takes its `proposal.is_empty()` branch, and
/// `complete_noop_job` dequeues the job again. Critically, `complete_noop_job`
/// does NOT call `record_daily_write` — only `finalize_applied_change` (a real
/// write) does — so this extra pass costs nothing against
/// `background_vault_optimizer_max_daily_writes`. It also cannot enqueue the
/// rest of the vault: `sync_topic_hubs` (which this function still calls,
/// see below) only re-bootstraps the WHOLE queue when the queue is fully
/// empty, and a re-enqueued single note keeps the queue non-empty.
///
/// So the loop is real but self-correcting and free. It was still rejected
/// in favor of this narrower helper for two reasons: (1) it's a wasted extra
/// 30-second worker tick per optimizer write for no benefit, and (2) it
/// would record a spurious no-op "decision"-shaped pass through the pipeline
/// that has no meaningful audit story (the note wasn't touched by that
/// second pass; there's nothing to explain). Skipping the enqueue avoids
/// both for the price of duplicating the search+chunk+topic-hub sequence
/// here instead of getting it for free from `commit_note_write` — an
/// acceptable tradeoff since the vault optimizer worker is the only caller
/// of this pattern.
///
/// ## Lock ordering
///
/// Must be called only after the caller (`main.rs::start_vault_optimizer_worker`)
/// has released BOTH `knowledge_store` and `vault_optimizer` — the locks it
/// held to drive `prepare_next`/`apply_pending`. This function reacquires
/// `knowledge_store` itself (a read, then — via `sync_topic_hubs` — a write),
/// so calling it while either lock from the optimizer tick is still held
/// would at best double-acquire `knowledge_store` (a real risk of deadlock
/// depending on lock implementation) and at worst invert the canonical
/// `knowledge_store`-before-`vault_optimizer` order documented at the top of
/// this file. The worker enforces this structurally: the block that computes
/// `tick`/`applied_note_id` ends (dropping its guards) before this function
/// is ever called.
///
/// Errors reading/indexing the note are logged and swallowed (matching
/// `commit_note_writes`'s precedent) rather than propagated, since a
/// background worker has no caller to report an error to and the note is
/// already durably written — a stale index self-heals on the next
/// `rebuild_all_indexes`. `sync_topic_hubs` failures DO propagate, matching
/// `commit_note_writes`.
#[cfg(test)]
pub(crate) async fn commit_note_index_refresh(
    state: &AppState,
    note_id: &str,
) -> Result<(), String> {
    let note = {
        let store = state.knowledge_store.read().await;
        store.get_note(note_id)
    };

    match note {
        Ok(note) => {
            {
                let mut search = state.search_service.write().await;
                if let Err(error) = index_commit::index_note_for_search(&mut search, &note) {
                    log::error!(
                        "Failed to index optimizer-updated note '{}' into search: {}",
                        note.id,
                        error
                    );
                }
                if let Err(error) = index_commit::commit_search(&mut search) {
                    log::error!(
                        "Failed to commit search index after optimizer update to '{}': {}",
                        note.id,
                        error
                    );
                }
            }
            sync_chunk_index_for_note(state, &note).await;
        }
        Err(error) => {
            log::warn!(
                "commit_note_index_refresh: note '{}' could not be read for indexing: {}",
                note_id,
                error
            );
        }
    }

    // Deliberately no `enqueue_vault_optimizer_note` call here — see the doc
    // comment above.
    sync_topic_hubs(state).await?;
    Ok(())
}

/// Single chokepoint for "a note was just deleted from the vault and needs
/// to disappear everywhere else": the search index, the chunk index, link
/// discovery, topic hubs (which also rebuilds the graph), and the vault
/// optimizer queue. Symmetric counterpart to [`commit_note_write`]/
/// [`commit_note_writes`] — those handle "a note was created or edited and
/// needs to become visible"; this handles the opposite direction.
///
/// Callers must have already removed the note from the vault itself (e.g.
/// via `KnowledgeStore::delete_note`) before calling this — it only cleans
/// up the derived indexes/queues, it does not touch the vault.
///
/// `reason` is forwarded to `enqueue_vault_optimizer_note` for its audit
/// trail (e.g. `"note_deleted"`), mirroring `commit_note_write`. This looks
/// odd at first — why enqueue a note that no longer exists? — but it's
/// harmless and matches the pre-existing behavior this function replaces
/// (`notes::delete_note` already did this): `VaultOptimizerService::
/// prepare_next` handles a missing note by logging a warning and dequeuing
/// it again on the very next tick (see its `store.get_note` error branch),
/// so this is a no-op fast-path, not a real optimizer run.
///
/// Error handling mirrors `commit_note_write`: search/chunk/link-discovery
/// failures are logged and swallowed (the note is already gone from the
/// vault; a stale index self-heals on the next `rebuild_all_indexes`),
/// while `sync_topic_hubs` failures propagate.
#[cfg(test)]
pub(crate) async fn commit_note_delete(
    state: &AppState,
    note_id: &str,
    reason: &str,
) -> Result<(), String> {
    {
        let mut search = state.search_service.write().await;
        if let Err(error) = index_commit::remove_note_for_search(&mut search, note_id) {
            log::error!(
                "Failed to remove note '{}' from search index: {}",
                note_id,
                error
            );
        }
        if let Err(error) = index_commit::commit_search(&mut search) {
            log::error!(
                "Failed to commit search index after deleting note '{}': {}",
                note_id,
                error
            );
        }
    }

    remove_note_chunks_from_index(state, note_id).await;
    remove_link_discovery_note(state, note_id).await?;
    sync_topic_hubs(state).await?;
    enqueue_vault_optimizer_note(state, note_id, reason).await?;

    Ok(())
}

#[cfg(test)]
#[path = "commit_note_write_tests.rs"]
pub(crate) mod commit_note_write_tests;
