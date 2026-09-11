use super::recovery::RecoveredMigrationStep;
use super::validation::{
    preview_authority_token, require_exact_fields, require_unique_bounded, validate_canonical_uuid,
    validate_current_preview, validate_manifest_authority, validate_operation_sequence,
    MAX_MIGRATION_AUTHORITY_ONLY_ADVANCES, MAX_MIGRATION_OPERATIONS,
};
use super::*;

pub(super) const MAX_MIGRATION_BLOB_BYTES: usize = 512 * 1024 * 1024;

#[must_use = "migration outcomes carry committed authority and must be repaired explicitly"]
#[derive(Debug)]
pub(crate) enum MigrationMutationOutcome<T> {
    NoWrite {
        value: T,
    },
    Committed {
        value: T,
        commit: crate::services::twin_events::MutationCommit,
        authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
        warning: Option<crate::models::mutation::CommittedMutationWarningV1>,
    },
    Partial {
        value: T,
        commit: Option<crate::services::twin_events::MutationCommit>,
        authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
        warning: crate::models::mutation::CommittedMutationWarningV1,
    },
}

impl<T> MigrationMutationOutcome<T> {
    pub(crate) fn into_parts(
        self,
    ) -> (
        T,
        Option<crate::services::twin_events::MutationCommit>,
        Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
        Option<crate::models::mutation::CommittedMutationWarningV1>,
        bool,
    ) {
        match self {
            Self::NoWrite { value } => (value, None, None, None, false),
            Self::Committed {
                value,
                commit,
                authority,
                warning,
            } => (value, Some(commit), Some(authority), warning, false),
            Self::Partial {
                value,
                commit,
                authority,
                warning,
            } => (value, commit, Some(authority), Some(warning), true),
        }
    }
}

#[derive(Debug)]
struct PreparedMigrationPlan {
    manifest: StoredManifest,
    blobs: Vec<(String, Vec<u8>)>,
}

impl MarkdownMigrationService {
    pub(crate) fn apply_transaction(
        &self,
        preview_id: &str,
        request: MarkdownMigrationRequest,
        store: &mut KnowledgeStore,
        current_authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<MigrationMutationOutcome<MarkdownMigrationApplyResult>> {
        let migration_root = self.strict_root()?;
        let _transaction_lock = migration_root
            .lock_exclusive("runs/transaction.lock")
            .map_err(anyhow::Error::new)?;
        let preview = self.load_strict_preview(preview_id)?;
        let preview_authority = preview_authority_token(&preview)?;
        require_current_preview_scope(&preview, &current_authority.root_scope, store.vault_path())?;
        let requested_hub =
            canonical_hub_folder(request.hub_folder.as_deref().unwrap_or("_grafyn/hubs"))?;
        let requested_program = canonical_program_path(
            request
                .program_path
                .as_deref()
                .unwrap_or("_grafyn/program.md"),
        )?;
        let canonical_request =
            canonical_migration_request(&request, &requested_hub, &requested_program);
        if preview.request.as_ref() != Some(&canonical_request) {
            anyhow::bail!("migration apply request differs from its preview");
        }

        if let Some(mut manifest) = self.load_strict_manifest_optional(preview_id)? {
            if manifest.preview_id != preview.preview_id
                || manifest.request.as_ref() != preview.request.as_ref()
                || manifest.root_scope != preview.root_scope
            {
                anyhow::bail!("prepared migration manifest does not match its preview");
            }
            if manifest.status == "applied" {
                if manifest.final_authority.as_ref() != Some(&store.current_migration_authority()?)
                {
                    anyhow::bail!("applied migration authority is no longer current");
                }
                let value = apply_result_from_manifest(&manifest);
                return Ok(match commit_from_manifest(&manifest) {
                    Some(commit) => MigrationMutationOutcome::Committed {
                        authority: commit
                            .authority_token
                            .clone()
                            .expect("manifest commit authority validated"),
                        value,
                        commit,
                        warning: None,
                    },
                    None => MigrationMutationOutcome::NoWrite { value },
                });
            }
            if manifest.status == "prepared" || manifest.status == "apply_partial" {
                if manifest.apply_next == 0 && manifest.active_step.is_none() {
                    let preview_at = preview
                        .created_at
                        .ok_or_else(|| anyhow::anyhow!("migration preview timestamp is missing"))?;
                    let (snapshots, overlays) =
                        store.migration_source_snapshot(&preview.program_path, preview_at)?;
                    let inventory = snapshots
                        .iter()
                        .map(|snapshot| snapshot.source.clone())
                        .collect::<Vec<_>>();
                    if inventory != preview.source_inventory
                        || overlays != preview.overlay_inventory
                    {
                        anyhow::bail!(
                            "migration physical inventory changed outside the transaction"
                        );
                    }
                    let derived = super::semantics::derive_preview_semantics(
                        &snapshots,
                        &canonical_request,
                        &preview.hub_folder,
                        &preview.program_path,
                    )?;
                    super::semantics::require_exact_preview_semantics(&preview, &derived)?;
                    let expected =
                        self.prepare_apply_plan(&preview, &canonical_request, snapshots, store)?;
                    self.require_exact_prepared_plan(&manifest, &expected)?;
                }
                let current = store.current_migration_authority()?;
                if manifest.active_step.is_none()
                    && manifest.final_authority.as_ref() != Some(&current)
                {
                    anyhow::bail!("prepared migration authority is stale");
                }
                if current.root_scope != current_authority.root_scope
                    || current.lease_epoch_uuid != current_authority.lease_epoch_uuid
                {
                    anyhow::bail!("prepared migration authority lease changed");
                }
                return self.execute_apply_manifest(store, &mut manifest);
            }
            anyhow::bail!("migration run is not eligible for apply");
        }

        if current_authority != preview_authority
            || store.current_migration_authority()? != preview_authority
        {
            anyhow::bail!("migration preview authority is stale");
        }

        let preview_at = preview
            .created_at
            .ok_or_else(|| anyhow::anyhow!("migration preview timestamp is missing"))?;
        let (snapshots, overlays) =
            store.migration_source_snapshot(&preview.program_path, preview_at)?;
        let inventory = snapshots
            .iter()
            .map(|snapshot| snapshot.source.clone())
            .collect::<Vec<_>>();
        if inventory != preview.source_inventory || overlays != preview.overlay_inventory {
            anyhow::bail!("migration sources changed after preview");
        }
        let derived = super::semantics::derive_preview_semantics(
            &snapshots,
            &canonical_request,
            &preview.hub_folder,
            &preview.program_path,
        )?;
        super::semantics::require_exact_preview_semantics(&preview, &derived)?;
        if store.current_migration_authority()? != preview_authority {
            anyhow::bail!("migration authority changed during apply preflight");
        }

        let prepared = self.prepare_apply_plan(&preview, &canonical_request, snapshots, store)?;
        let (finish_snapshots, finish_overlays) =
            store.migration_source_snapshot(&preview.program_path, preview_at)?;
        let finish_inventory = finish_snapshots
            .into_iter()
            .map(|snapshot| snapshot.source)
            .collect::<Vec<_>>();
        if finish_inventory != preview.source_inventory
            || finish_overlays != preview.overlay_inventory
            || store.current_migration_authority()? != preview_authority
        {
            anyhow::bail!("migration sources changed while the apply plan was prepared");
        }
        self.persist_prepared_plan(&prepared)?;
        let mut manifest = prepared.manifest;
        self.execute_apply_manifest(store, &mut manifest)
    }

    pub(crate) fn rollback_transaction(
        &self,
        run_id: &str,
        store: &mut KnowledgeStore,
        current_authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<MigrationMutationOutcome<crate::models::migration::MarkdownMigrationRollbackResult>>
    {
        let migration_root = self.strict_root()?;
        let _transaction_lock = migration_root
            .lock_exclusive("runs/transaction.lock")
            .map_err(anyhow::Error::new)?;
        let mut manifest = self.load_strict_manifest(run_id)?;
        if manifest.root_scope.as_ref() != Some(&current_authority.root_scope) {
            anyhow::bail!("legacy or cross-vault migration manifests are audit-only");
        }
        if store.current_migration_authority()? != current_authority {
            anyhow::bail!("migration authority changed before rollback");
        }
        if manifest.status == "rolled_back" {
            return Ok(MigrationMutationOutcome::NoWrite {
                value: rollback_result_from_manifest(&manifest, true),
            });
        }
        if manifest.status == "rollback_prepared" || manifest.status == "rollback_partial" {
            return self.execute_rollback_manifest(store, &mut manifest);
        }
        if manifest.status != "applied" && manifest.status != "apply_partial" {
            anyhow::bail!("migration run has no committed apply prefix to roll back");
        }
        if manifest.active_step.is_some() {
            if let RecoveredMigrationStep::AuthorityOnly(commit) =
                self.recover_active_step(&mut manifest, store)?
            {
                let authority = commit
                    .authority_token
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("authority-only abort lost its authority"))?;
                let mut value = rollback_result_from_manifest(&manifest, false);
                value.status = "partial".into();
                value.warning = Some(fixed_warning());
                return Ok(MigrationMutationOutcome::Partial {
                    value,
                    authority,
                    commit: Some(commit),
                    warning: fixed_warning(),
                });
            }
        }
        if manifest.final_authority.as_ref() != Some(&store.current_migration_authority()?) {
            anyhow::bail!("migration rollback authority is stale");
        }
        if manifest.apply_next == 0 {
            anyhow::bail!("migration run has no committed apply prefix to roll back");
        }

        manifest.rollback_total = manifest.apply_next;
        self.verify_manifest_inventory_and_blobs(&manifest, store)?;
        self.prevalidate_remaining_rollback(&manifest, store)?;
        manifest.status = "rollback_prepared".into();
        manifest.applied_at.get_or_insert_with(Utc::now);
        manifest.rollback_next = 0;
        manifest.active_step = None;
        self.write_strict_manifest(&manifest)?;
        self.execute_rollback_manifest(store, &mut manifest)
    }

    pub(super) fn strict_root(
        &self,
    ) -> Result<&std::sync::Arc<crate::services::twin_events::AnchoredRoot>> {
        self.migration_root
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("retained migration capability is unavailable"))
    }

    fn prepare_apply_plan(
        &self,
        preview: &MarkdownMigrationPreview,
        request: &MarkdownMigrationRequest,
        snapshots: Vec<crate::services::knowledge_store::MigrationMarkdownSnapshot>,
        store: &KnowledgeStore,
    ) -> Result<PreparedMigrationPlan> {
        let apply_at = preview
            .created_at
            .ok_or_else(|| anyhow::anyhow!("migration preview timestamp is missing"))?;
        let authority = preview_authority_token(preview)?;
        let mut snapshot_by_id = BTreeMap::new();
        let mut reserved_ids = HashSet::new();
        let mut reserved_paths = HashSet::new();
        for snapshot in &snapshots {
            if !reserved_paths.insert(migration_path_key(&snapshot.source.relative_path)) {
                anyhow::bail!("duplicate physical Markdown path in migration plan");
            }
            if let Some(note) = snapshot.note.as_ref() {
                if snapshot_by_id.insert(note.id.clone(), snapshot).is_some()
                    || !reserved_ids.insert(note.id.clone())
                {
                    anyhow::bail!("duplicate note identity in migration plan");
                }
            }
        }
        for overlay in &preview.overlay_inventory {
            let note_id = overlay
                .relative_path
                .strip_suffix(".json")
                .ok_or_else(|| anyhow::anyhow!("migration overlay path is not canonical"))?;
            reserved_ids.insert(note_id.to_string());
        }
        let mut proposals = preview.note_proposals.iter().collect::<Vec<_>>();
        proposals.sort_by(|left, right| left.note_id.cmp(&right.note_id));
        let proposal_ids = proposals
            .iter()
            .map(|proposal| proposal.note_id.clone())
            .collect::<HashSet<_>>();
        if proposal_ids.len() != proposals.len()
            || proposal_ids != snapshot_by_id.keys().cloned().collect()
        {
            anyhow::bail!("migration proposals do not exactly cover the source notes");
        }
        let titles = snapshot_by_id
            .iter()
            .map(|(id, snapshot)| {
                (
                    id.clone(),
                    snapshot.note.as_ref().expect("note snapshot").title.clone(),
                )
            })
            .collect::<HashMap<_, _>>();

        let mut blobs = Vec::new();
        let mut operations = Vec::new();
        let mut created_files = Vec::new();
        let mut overlay_note_ids = Vec::new();
        let mut touched_note_ids = Vec::new();
        let mut skipped_fallback_note_ids = Vec::new();

        if matches!(
            preview.expected_program_target,
            Some(ExpectedProgramTarget::Absent)
        ) {
            let bytes = default_program_file_contents(&preview.hub_folder, &preview.program_path)
                .into_bytes();
            push_put_operation(
                preview.preview_id.as_str(),
                &mut operations,
                &mut blobs,
                crate::services::twin_events::TargetKind::Markdown,
                preview.program_path.clone(),
                crate::services::twin_events::BeforeImage::Absent,
                bytes,
                None,
            )?;
            created_files.push(preview.program_path.clone());
        }

        for proposal in proposals {
            let snapshot = snapshot_by_id
                .get(&proposal.note_id)
                .expect("proposal coverage validated");
            let note = snapshot.note.as_ref().expect("note snapshot");
            if proposal.relative_path != note.relative_path || proposal.title != note.title {
                anyhow::bail!("migration proposal changed note identity");
            }
            if request.mode == MarkdownMigrationMode::SidecarFirst {
                let overlay = json!({
                    "aliases": proposal.aliases,
                    "tags": proposal.inferred_tags,
                    "schema_version": CURRENT_NOTE_SCHEMA_VERSION,
                    "migration_source": MIGRATION_SOURCE_MARKDOWN,
                    "optimizer_managed": false,
                    "properties": {
                        PROP_TOPIC_KEY: proposal.topic_key,
                        PROP_INFERRED_LINK_IDS: proposal.inferred_link_ids,
                    }
                });
                let target_key = format!("{}.json", proposal.note_id);
                let before_bytes = snapshot.overlay_raw_bytes.clone();
                let before = before_bytes.as_ref().map_or(
                    crate::services::twin_events::BeforeImage::Absent,
                    |bytes| {
                        crate::services::twin_events::BeforeImage::Sha256(
                            crate::services::twin_events::digest_bytes(bytes),
                        )
                    },
                );
                let after_bytes = serde_json::to_string_pretty(&overlay)?.into_bytes();
                if before_bytes.as_deref() != Some(after_bytes.as_slice()) {
                    let after_state = crate::services::twin_events::BeforeImage::Sha256(
                        crate::services::twin_events::digest_bytes(&after_bytes),
                    );
                    let apply_payload = migration_overlay_state_digest(
                        &snapshot.source.markdown_digest,
                        &after_state,
                    );
                    let rollback_payload =
                        migration_overlay_state_digest(&snapshot.source.markdown_digest, &before);
                    push_put_operation(
                        preview.preview_id.as_str(),
                        &mut operations,
                        &mut blobs,
                        crate::services::twin_events::TargetKind::OverlayJson,
                        target_key,
                        before,
                        after_bytes,
                        before_bytes,
                    )?;
                    let operation = operations.last_mut().expect("operation added");
                    operation.note_event = Some(StoredMigrationNoteEventV1 {
                        note_id: note.id.clone(),
                        change: crate::models::twin_event::NoteChangeKind::Updated,
                        observed_at: apply_at,
                        governance: store.migration_note_governance(note),
                        payload_digest: apply_payload,
                        evidence_digest: rollback_payload.clone(),
                    });
                    operation.rollback_note_event = Some(StoredMigrationNoteEventV1 {
                        note_id: note.id.clone(),
                        change: crate::models::twin_event::NoteChangeKind::Updated,
                        observed_at: apply_at,
                        governance: store.migration_note_governance(note),
                        payload_digest: rollback_payload,
                        evidence_digest: operation
                            .note_event
                            .as_ref()
                            .expect("apply event stored")
                            .payload_digest
                            .clone(),
                    });
                }
                overlay_note_ids.push(proposal.note_id.clone());
                continue;
            }

            if note.frontmatter_raw_fallback.is_some() {
                skipped_fallback_note_ids.push(note.id.clone());
                continue;
            }
            let mut properties = note.properties.clone();
            if let Some(topic_key) = &proposal.topic_key {
                properties.insert(PROP_TOPIC_KEY.into(), Value::String(topic_key.clone()));
                properties.insert(
                    PROP_TOPIC_ALIASES.into(),
                    Value::Array(
                        proposal
                            .aliases
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                );
            }
            if !proposal.inferred_link_ids.is_empty() {
                properties.insert(
                    PROP_INFERRED_LINK_IDS.into(),
                    Value::Array(
                        proposal
                            .inferred_link_ids
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                );
            }
            let new_content = if request.auto_insert_links == Some(true) {
                let (content, inserted) = append_related_links_from_snapshot(
                    &note.content,
                    &proposal.inferred_link_ids,
                    &titles,
                )?;
                if !inserted.is_empty() {
                    properties.insert(
                        PROP_AUTO_INSERTED_LINK_IDS.into(),
                        Value::Array(inserted.into_iter().map(Value::String).collect()),
                    );
                }
                Some(normalize_rewritten_content(
                    &note.title,
                    &content,
                    request.mode == MarkdownMigrationMode::FullRewrite,
                ))
            } else if request.mode == MarkdownMigrationMode::FullRewrite {
                Some(normalize_rewritten_content(
                    &note.title,
                    &note.content,
                    true,
                ))
            } else {
                None
            };
            let exact = store.materialize_note_update_at(
                note,
                NoteUpdate {
                    title: None,
                    content: new_content,
                    relative_path: Some(note.relative_path.clone()),
                    aliases: Some(merge_unique_strings(
                        note.aliases.clone(),
                        proposal.aliases.clone(),
                    )),
                    status: None,
                    tags: Some(merge_unique_strings(
                        note.tags.clone(),
                        proposal.inferred_tags.clone(),
                    )),
                    schema_version: Some(CURRENT_NOTE_SCHEMA_VERSION),
                    migration_source: Some(MIGRATION_SOURCE_MARKDOWN.into()),
                    optimizer_managed: Some(false),
                    properties: Some(properties),
                },
                apply_at,
            )?;
            let (relative_path, after_bytes) = store.serialized_note_bytes(&exact)?;
            if relative_path != snapshot.source.relative_path {
                anyhow::bail!("migration note target path changed while planning");
            }
            if snapshot.raw_bytes != after_bytes {
                let after_digest = crate::services::twin_events::digest_bytes(&after_bytes);
                let event = store.migration_note_event(
                    &exact,
                    crate::models::twin_event::NoteChangeKind::Updated,
                    after_digest.clone(),
                    snapshot.source.markdown_digest.clone(),
                    apply_at,
                );
                push_put_operation(
                    preview.preview_id.as_str(),
                    &mut operations,
                    &mut blobs,
                    crate::services::twin_events::TargetKind::Markdown,
                    relative_path,
                    crate::services::twin_events::BeforeImage::Sha256(
                        snapshot.source.markdown_digest.clone(),
                    ),
                    after_bytes,
                    Some(snapshot.raw_bytes.clone()),
                )?;
                let operation = operations.last_mut().expect("operation added");
                operation.note_event = Some(stored_note_event(event));
                operation.rollback_note_event =
                    Some(stored_note_event(store.migration_note_event(
                        note,
                        crate::models::twin_event::NoteChangeKind::Updated,
                        snapshot.source.markdown_digest.clone(),
                        operation.after_digest.clone(),
                        apply_at,
                    )));
            }
            touched_note_ids.push(note.id.clone());
        }

        let mut topics = preview.topic_candidates.iter().collect::<Vec<_>>();
        topics.sort_by(|left, right| left.topic_key.cmp(&right.topic_key));
        let mut topic_keys = HashSet::new();
        let mut created_hub_note_ids = Vec::new();
        for topic in topics {
            if !topic_keys.insert(topic.topic_key.clone()) {
                anyhow::bail!("duplicate migration topic key");
            }
            if topic.reuse_existing_hub_id.is_some() {
                continue;
            }
            let title = format!("Hub: {}", topic.display_name);
            let mut id = slugify(&title);
            if id.is_empty() {
                id = "note".into();
            }
            let base = id.clone();
            let mut suffix = 1usize;
            while reserved_ids.contains(&id) {
                id = format!("{base}-{suffix}");
                suffix += 1;
            }
            reserved_ids.insert(id.clone());
            let relative_path =
                format!("{}/{}.md", preview.hub_folder, slugify(&topic.display_name));
            if !reserved_paths.insert(migration_path_key(&relative_path)) {
                anyhow::bail!("migration hub target path already exists");
            }
            let content = format!(
                "# Hub: {}\n\nGrafyn will keep this topic hub updated from its member notes.\n",
                topic.display_name
            );
            let mut note = Note {
                id: id.clone(),
                title,
                content,
                relative_path: relative_path.clone(),
                aliases: vec![topic.display_name.clone()],
                status: crate::models::note::NoteStatus::Canonical,
                tags: vec!["hub".into()],
                created_at: apply_at,
                updated_at: apply_at,
                schema_version: CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: Some(MIGRATION_SOURCE_MARKDOWN.into()),
                optimizer_managed: true,
                wikilinks: Vec::new(),
                parsed_links: Vec::new(),
                properties: HashMap::from([
                    (
                        PROP_TOPIC_KEY.into(),
                        Value::String(topic.topic_key.clone()),
                    ),
                    (
                        PROP_TOPIC_ALIASES.into(),
                        Value::Array(vec![Value::String(topic.display_name.clone())]),
                    ),
                    ("is_topic_hub".into(), Value::Bool(true)),
                ]),
                frontmatter_raw_fallback: None,
            };
            note.wikilinks = store.extract_wikilinks(&note.content);
            note.parsed_links = store.extract_links(&note.content, &note.relative_path);
            let (_, after_bytes) = store.serialized_note_bytes(&note)?;
            let event = store.migration_note_event(
                &note,
                crate::models::twin_event::NoteChangeKind::Created,
                crate::services::twin_events::digest_bytes(&after_bytes),
                crate::services::twin_events::digest_bytes(&after_bytes),
                apply_at,
            );
            push_put_operation(
                preview.preview_id.as_str(),
                &mut operations,
                &mut blobs,
                crate::services::twin_events::TargetKind::Markdown,
                relative_path.clone(),
                crate::services::twin_events::BeforeImage::Absent,
                after_bytes,
                None,
            )?;
            let operation = operations.last_mut().expect("operation added");
            operation.note_event = Some(stored_note_event(event));
            operation.rollback_note_event = Some(StoredMigrationNoteEventV1 {
                note_id: note.id.clone(),
                change: crate::models::twin_event::NoteChangeKind::Deleted,
                observed_at: apply_at,
                governance: store.migration_note_governance(&note),
                payload_digest: operation.after_digest.clone(),
                evidence_digest: operation.after_digest.clone(),
            });
            created_files.push(relative_path);
            created_hub_note_ids.push(id);
        }

        validate_operation_sequence(&operations)?;
        let total_blob_bytes = blobs.iter().try_fold(0usize, |total, (_, bytes)| {
            total
                .checked_add(bytes.len())
                .ok_or_else(|| anyhow::anyhow!("migration blob byte count overflowed"))
        })?;
        if total_blob_bytes > MAX_MIGRATION_BLOB_BYTES {
            anyhow::bail!("migration before/after blobs exceed their aggregate byte limit");
        }
        let manifest = StoredManifest {
            schema_version: MIGRATION_MANIFEST_SCHEMA_VERSION,
            run_id: preview.preview_id.clone(),
            preview_id: preview.preview_id.clone(),
            root_scope: preview.root_scope.clone(),
            vault_path: preview.vault_path.clone(),
            mode: request.mode.clone(),
            created_at: preview.created_at.unwrap_or(apply_at),
            applied_at: None,
            status: "prepared".into(),
            created_files,
            backup_files: operations
                .iter()
                .filter(|operation| {
                    operation.target_kind == crate::services::twin_events::TargetKind::Markdown
                        && operation.before_blob_key.is_some()
                })
                .map(|operation| operation.target_key.clone())
                .collect(),
            overlay_note_ids,
            touched_note_ids,
            created_hub_note_ids,
            skipped_fallback_note_ids,
            expected_program_target: preview.expected_program_target.clone(),
            program_after_digest: preview.program_after_digest.clone(),
            program_path: Some(preview.program_path.clone()),
            request: Some(request.clone()),
            source_inventory: preview.source_inventory.clone(),
            overlay_inventory: preview.overlay_inventory.clone(),
            starting_authority: Some(authority.clone()),
            final_authority: Some(authority),
            operations,
            apply_next: 0,
            rollback_next: 0,
            rollback_total: 0,
            authority_only_advances: 0,
            active_step: None,
            last_commit: None,
        };
        Ok(PreparedMigrationPlan { manifest, blobs })
    }

    fn persist_prepared_plan(&self, prepared: &PreparedMigrationPlan) -> Result<()> {
        let root = self.strict_root()?;
        for (key, bytes) in &prepared.blobs {
            root.put_atomic(key, bytes).map_err(anyhow::Error::new)?;
            let durable = root
                .read_bounded(key, crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES)
                .map_err(anyhow::Error::new)?
                .ok_or_else(|| anyhow::anyhow!("migration blob disappeared after publication"))?;
            if &durable != bytes {
                anyhow::bail!("migration blob changed after publication");
            }
        }
        self.write_immutable_plan(&prepared.manifest)?;
        self.write_strict_manifest(&prepared.manifest)
    }

    fn require_exact_prepared_plan(
        &self,
        manifest: &StoredManifest,
        expected: &PreparedMigrationPlan,
    ) -> Result<()> {
        self.require_manifest_plan_binding(manifest, &expected.manifest)
            .context("prepared migration plan differs from its exact preview derivation")?;
        for (key, bytes) in &expected.blobs {
            if self.read_blob(manifest, key)? != *bytes {
                anyhow::bail!("prepared migration blob differs from its exact preview derivation");
            }
        }
        Ok(())
    }

    fn execute_apply_manifest(
        &self,
        store: &mut KnowledgeStore,
        manifest: &mut StoredManifest,
    ) -> Result<MigrationMutationOutcome<MarkdownMigrationApplyResult>> {
        let mut last_commit = None;
        let mut warning = None;
        if manifest.active_step.is_some() {
            match self.recover_active_step(manifest, store) {
                Ok(RecoveredMigrationStep::Committed(commit)) => {
                    if commit.mutation_id.is_none() {
                        return self.apply_partial_or_error(
                            manifest,
                            Some(commit),
                            anyhow::anyhow!(
                                "migration authority advanced before its first target was staged"
                            ),
                        );
                    }
                    warning = warning.or_else(|| commit.postcommit_warning.then(fixed_warning));
                    last_commit = Some(commit);
                }
                Ok(RecoveredMigrationStep::AuthorityOnly(commit)) => {
                    return self.apply_partial_or_error(
                        manifest,
                        Some(commit),
                        anyhow::anyhow!("migration authority advanced without applying its target"),
                    )
                }
                Ok(RecoveredMigrationStep::None) => {}
                Err(error) => return self.apply_partial_or_error(manifest, last_commit, error),
            }
        }
        if let Err(error) = self.verify_manifest_inventory_and_blobs(manifest, store) {
            return self.apply_partial_or_error(manifest, last_commit, error);
        }
        while manifest.apply_next < manifest.operations.len() || manifest.active_step.is_some() {
            match self.recover_active_step(manifest, store) {
                Ok(RecoveredMigrationStep::Committed(commit)) => {
                    if commit.mutation_id.is_none() {
                        return self.apply_partial_or_error(
                            manifest,
                            Some(commit),
                            anyhow::anyhow!(
                                "migration authority advanced before its target was staged"
                            ),
                        );
                    }
                    warning = warning.or_else(|| commit.postcommit_warning.then(fixed_warning));
                    last_commit = Some(commit);
                    continue;
                }
                Ok(RecoveredMigrationStep::AuthorityOnly(commit)) => {
                    return self.apply_partial_or_error(
                        manifest,
                        Some(commit),
                        anyhow::anyhow!("migration authority advanced without applying its target"),
                    )
                }
                Ok(RecoveredMigrationStep::None) => {}
                Err(error) => {
                    return self.apply_partial_or_error(manifest, last_commit, error);
                }
            }
            if manifest.apply_next >= manifest.operations.len() {
                break;
            }
            if store.current_migration_authority()?
                != manifest
                    .final_authority
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("migration manifest lost chained authority"))?
            {
                return self.apply_partial_or_error(
                    manifest,
                    last_commit,
                    anyhow::anyhow!("migration authority changed between apply steps"),
                );
            }
            if manifest.authority_only_advances >= MAX_MIGRATION_AUTHORITY_ONLY_ADVANCES {
                return self.apply_partial_or_error(
                    manifest,
                    last_commit.or_else(|| commit_from_manifest(manifest)),
                    anyhow::anyhow!("migration authority-only advance limit is exhausted"),
                );
            }
            let operation = manifest.operations[manifest.apply_next].clone();
            let authority = manifest
                .final_authority
                .clone()
                .ok_or_else(|| anyhow::anyhow!("migration manifest lost chained authority"))?;
            let target =
                self.runtime_target(manifest, &operation, MigrationDirectionV1::Apply, store)?;
            manifest.active_step = Some(MigrationStepWitnessV1 {
                direction: MigrationDirectionV1::Apply,
                operation_index: operation.index,
                expected_authority: authority.clone(),
                intent: None,
                committed_authority: None,
                progress_recorded: false,
                receipt_consumed: false,
                abort_recorded: false,
            });
            self.write_strict_manifest(manifest)?;
            let manifest_cell = std::cell::RefCell::new(manifest.clone());
            let mut prepared_hook = |intent: &crate::services::twin_events::MutationIntentV1| {
                let mut durable = manifest_cell.borrow_mut();
                durable
                    .active_step
                    .as_mut()
                    .expect("active migration witness")
                    .intent = Some(intent.clone());
                self.write_strict_manifest(&durable).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })
            };
            let mut committed_hook = |commit: &crate::services::twin_events::MutationCommit| {
                let mut durable = manifest_cell.borrow_mut();
                durable
                    .active_step
                    .as_mut()
                    .expect("active migration witness")
                    .committed_authority = commit.authority_token.clone();
                self.write_strict_manifest(&durable).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })
            };
            let result = store.commit_exact_migration_target_with_hooks(
                target,
                authority,
                &mut prepared_hook,
                &mut committed_hook,
            );
            *manifest = manifest_cell.into_inner();
            match result {
                Ok(commit) => {
                    if let Err(error) = self.finish_committed_step(manifest, store, commit.clone())
                    {
                        return self.apply_partial_or_error(manifest, Some(commit), error);
                    }
                    warning = warning.or_else(|| commit.postcommit_warning.then(fixed_warning));
                    last_commit = Some(commit);
                }
                Err(error) => {
                    let authority_advanced = error
                        .downcast_ref::<crate::services::twin_events::MutationError>()
                        .and_then(|error| error.authority_advanced_commit());
                    match self.recover_active_step(manifest, store) {
                        Ok(RecoveredMigrationStep::Committed(commit)) => {
                            if commit.mutation_id.is_none() {
                                return self.apply_partial_or_error(
                                    manifest,
                                    Some(commit),
                                    anyhow::anyhow!(
                                        "migration authority advanced before its target was staged"
                                    ),
                                );
                            }
                            warning = Some(fixed_warning());
                            last_commit = Some(commit);
                        }
                        Ok(RecoveredMigrationStep::AuthorityOnly(commit)) => {
                            return self.apply_partial_or_error(
                                manifest,
                                Some(commit),
                                anyhow::anyhow!(
                                    "migration authority advanced without applying its target"
                                ),
                            )
                        }
                        Ok(RecoveredMigrationStep::None)
                            if manifest.apply_next == 0 && authority_advanced.is_none() =>
                        {
                            return Err(error)
                        }
                        Ok(RecoveredMigrationStep::None) => {
                            return self.apply_partial_or_error(
                                manifest,
                                authority_advanced.or(last_commit),
                                error,
                            )
                        }
                        Err(recovery) => {
                            log::error!("Migration apply recovery failed: {recovery}");
                            return self.apply_partial_or_error(
                                manifest,
                                authority_advanced.or(last_commit),
                                error,
                            );
                        }
                    }
                }
            }
        }
        manifest.status = "applied".into();
        manifest.applied_at = Some(Utc::now());
        if let Err(error) = self.write_strict_manifest(manifest) {
            return self.apply_partial_or_error(manifest, last_commit, error);
        }
        store.reload_authoritative_state();
        let value = apply_result_from_manifest(manifest);
        let commit = last_commit.or_else(|| commit_from_manifest(manifest));
        Ok(match commit {
            Some(commit) => MigrationMutationOutcome::Committed {
                authority: commit
                    .authority_token
                    .clone()
                    .expect("migration commit authority validated"),
                value,
                commit,
                warning,
            },
            None => MigrationMutationOutcome::NoWrite { value },
        })
    }

    fn execute_rollback_manifest(
        &self,
        store: &mut KnowledgeStore,
        manifest: &mut StoredManifest,
    ) -> Result<MigrationMutationOutcome<crate::models::migration::MarkdownMigrationRollbackResult>>
    {
        let mut last_commit = None;
        let mut warning = None;
        if manifest.active_step.is_some() {
            match self.recover_active_step(manifest, store) {
                Ok(RecoveredMigrationStep::Committed(commit)) => {
                    if commit.mutation_id.is_none() {
                        return self.rollback_partial_or_error(
                            manifest,
                            Some(commit),
                            anyhow::anyhow!(
                                "rollback authority advanced before its first target was staged"
                            ),
                        );
                    }
                    warning = warning.or_else(|| commit.postcommit_warning.then(fixed_warning));
                    last_commit = Some(commit);
                }
                Ok(RecoveredMigrationStep::AuthorityOnly(commit)) => {
                    return self.rollback_partial_or_error(
                        manifest,
                        Some(commit),
                        anyhow::anyhow!(
                            "migration authority advanced without rolling back its target"
                        ),
                    )
                }
                Ok(RecoveredMigrationStep::None) => {}
                Err(error) => return self.rollback_partial_or_error(manifest, last_commit, error),
            }
        }
        if let Err(error) = self.verify_manifest_inventory_and_blobs(manifest, store) {
            return self.rollback_partial_or_error(manifest, last_commit, error);
        }
        if let Err(error) = self.prevalidate_remaining_rollback(manifest, store) {
            return self.rollback_partial_or_error(manifest, last_commit, error);
        }
        while manifest.rollback_next < manifest.rollback_total || manifest.active_step.is_some() {
            match self.recover_active_step(manifest, store) {
                Ok(RecoveredMigrationStep::Committed(commit)) => {
                    if commit.mutation_id.is_none() {
                        return self.rollback_partial_or_error(
                            manifest,
                            Some(commit),
                            anyhow::anyhow!(
                                "rollback authority advanced before its target was staged"
                            ),
                        );
                    }
                    warning = warning.or_else(|| commit.postcommit_warning.then(fixed_warning));
                    last_commit = Some(commit);
                    continue;
                }
                Ok(RecoveredMigrationStep::AuthorityOnly(commit)) => {
                    return self.rollback_partial_or_error(
                        manifest,
                        Some(commit),
                        anyhow::anyhow!(
                            "migration authority advanced without rolling back its target"
                        ),
                    )
                }
                Ok(RecoveredMigrationStep::None) => {}
                Err(error) => {
                    return self.rollback_partial_or_error(manifest, last_commit, error);
                }
            }
            if manifest.rollback_next >= manifest.rollback_total {
                break;
            }
            if store.current_migration_authority()?
                != manifest
                    .final_authority
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("migration manifest lost chained authority"))?
            {
                return self.rollback_partial_or_error(
                    manifest,
                    last_commit,
                    anyhow::anyhow!("migration authority changed between rollback steps"),
                );
            }
            if manifest.authority_only_advances >= MAX_MIGRATION_AUTHORITY_ONLY_ADVANCES {
                return self.rollback_partial_or_error(
                    manifest,
                    last_commit.or_else(|| commit_from_manifest(manifest)),
                    anyhow::anyhow!("migration authority-only advance limit is exhausted"),
                );
            }
            let index = manifest.rollback_total - 1 - manifest.rollback_next;
            let operation = manifest.operations[index].clone();
            let authority = manifest
                .final_authority
                .clone()
                .ok_or_else(|| anyhow::anyhow!("migration manifest lost chained authority"))?;
            let target =
                self.runtime_target(manifest, &operation, MigrationDirectionV1::Rollback, store)?;
            manifest.active_step = Some(MigrationStepWitnessV1 {
                direction: MigrationDirectionV1::Rollback,
                operation_index: operation.index,
                expected_authority: authority.clone(),
                intent: None,
                committed_authority: None,
                progress_recorded: false,
                receipt_consumed: false,
                abort_recorded: false,
            });
            self.write_strict_manifest(manifest)?;
            let manifest_cell = std::cell::RefCell::new(manifest.clone());
            let mut prepared_hook = |intent: &crate::services::twin_events::MutationIntentV1| {
                let mut durable = manifest_cell.borrow_mut();
                durable
                    .active_step
                    .as_mut()
                    .expect("active rollback witness")
                    .intent = Some(intent.clone());
                self.write_strict_manifest(&durable).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })
            };
            let mut committed_hook = |commit: &crate::services::twin_events::MutationCommit| {
                let mut durable = manifest_cell.borrow_mut();
                durable
                    .active_step
                    .as_mut()
                    .expect("active rollback witness")
                    .committed_authority = commit.authority_token.clone();
                self.write_strict_manifest(&durable).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })
            };
            let result = store.commit_exact_migration_target_with_hooks(
                target,
                authority,
                &mut prepared_hook,
                &mut committed_hook,
            );
            *manifest = manifest_cell.into_inner();
            match result {
                Ok(commit) => {
                    if let Err(error) = self.finish_committed_step(manifest, store, commit.clone())
                    {
                        return self.rollback_partial_or_error(manifest, Some(commit), error);
                    }
                    warning = warning.or_else(|| commit.postcommit_warning.then(fixed_warning));
                    last_commit = Some(commit);
                }
                Err(error) => {
                    let authority_advanced = error
                        .downcast_ref::<crate::services::twin_events::MutationError>()
                        .and_then(|error| error.authority_advanced_commit());
                    match self.recover_active_step(manifest, store) {
                        Ok(RecoveredMigrationStep::Committed(commit)) => {
                            if commit.mutation_id.is_none() {
                                return self.rollback_partial_or_error(
                                    manifest,
                                    Some(commit),
                                    anyhow::anyhow!(
                                        "rollback authority advanced before its target was staged"
                                    ),
                                );
                            }
                            warning = Some(fixed_warning());
                            last_commit = Some(commit);
                        }
                        Ok(RecoveredMigrationStep::AuthorityOnly(commit)) => {
                            return self.rollback_partial_or_error(
                                manifest,
                                Some(commit),
                                anyhow::anyhow!(
                                    "migration authority advanced without rolling back its target"
                                ),
                            )
                        }
                        Ok(RecoveredMigrationStep::None)
                            if manifest.rollback_next == 0 && authority_advanced.is_none() =>
                        {
                            return Err(error)
                        }
                        Ok(RecoveredMigrationStep::None) => {
                            return self.rollback_partial_or_error(
                                manifest,
                                authority_advanced.or(last_commit),
                                error,
                            )
                        }
                        Err(recovery) => {
                            log::error!("Migration rollback recovery failed: {recovery}");
                            return self.rollback_partial_or_error(
                                manifest,
                                authority_advanced.or(last_commit),
                                error,
                            );
                        }
                    }
                }
            }
        }
        manifest.status = "rolled_back".into();
        if let Err(error) = self.write_strict_manifest(manifest) {
            return self.rollback_partial_or_error(manifest, last_commit, error);
        }
        store.reload_authoritative_state();
        let value = rollback_result_from_manifest(manifest, true);
        let commit = last_commit.or_else(|| commit_from_manifest(manifest));
        Ok(match commit {
            Some(commit) => MigrationMutationOutcome::Committed {
                authority: commit
                    .authority_token
                    .clone()
                    .expect("migration commit authority validated"),
                value,
                commit,
                warning,
            },
            None => MigrationMutationOutcome::NoWrite { value },
        })
    }

    pub(super) fn runtime_target(
        &self,
        manifest: &StoredManifest,
        operation: &MigrationOperationV1,
        direction: MigrationDirectionV1,
        _store: &KnowledgeStore,
    ) -> Result<crate::services::knowledge_store::ExactMigrationTarget> {
        let (expected_before, desired, note_event) = match direction {
            MigrationDirectionV1::Apply => {
                let desired = desired_for_state(
                    operation.after.clone(),
                    operation.after_blob_key.as_deref(),
                    manifest,
                    self,
                )?;
                let event = operation.note_event.as_ref().map(exact_note_event);
                (operation.before.clone(), desired, event)
            }
            MigrationDirectionV1::Rollback => {
                let desired = desired_for_state(
                    operation.before.clone(),
                    operation.before_blob_key.as_deref(),
                    manifest,
                    self,
                )?;
                let event = operation.rollback_note_event.as_ref().map(exact_note_event);
                (operation.after.clone(), desired, event)
            }
        };
        Ok(crate::services::knowledge_store::ExactMigrationTarget {
            kind: operation.target_kind,
            relative_key: operation.target_key.clone(),
            expected_before,
            desired,
            note_event,
        })
    }

    fn prevalidate_remaining_rollback(
        &self,
        manifest: &StoredManifest,
        store: &KnowledgeStore,
    ) -> Result<()> {
        validate_operation_sequence(&manifest.operations)?;
        if manifest.rollback_total > manifest.apply_next
            || manifest.rollback_next > manifest.rollback_total
        {
            anyhow::bail!("migration rollback cursor is invalid");
        }
        for operation in manifest
            .operations
            .iter()
            .take(manifest.rollback_total - manifest.rollback_next)
        {
            if store.migration_target_before_image(operation.target_kind, &operation.target_key)?
                != operation.after
            {
                anyhow::bail!("migration rollback target changed after apply");
            }
            if let Some(key) = operation.after_blob_key.as_deref() {
                let bytes = self.read_blob(manifest, key)?;
                if crate::services::twin_events::digest_bytes(&bytes) != operation.after_digest {
                    anyhow::bail!("migration after-image blob digest changed");
                }
            }
            if let (crate::services::twin_events::BeforeImage::Sha256(expected), Some(key)) =
                (&operation.before, operation.before_blob_key.as_deref())
            {
                if crate::services::twin_events::digest_bytes(&self.read_blob(manifest, key)?)
                    != *expected
                {
                    anyhow::bail!("migration backup digest changed");
                }
            } else if matches!(
                operation.before,
                crate::services::twin_events::BeforeImage::Sha256(_)
            ) {
                anyhow::bail!("migration rollback backup is missing");
            }
        }
        Ok(())
    }

    pub(super) fn project_manifest_inventory(
        &self,
        manifest: &mut StoredManifest,
        operation_index: usize,
        direction: MigrationDirectionV1,
    ) -> Result<()> {
        let (source_inventory, overlay_inventory) =
            self.projected_manifest_inventory(manifest, [(operation_index, direction)])?;
        manifest.source_inventory = source_inventory;
        manifest.overlay_inventory = overlay_inventory;
        Ok(())
    }

    fn verify_manifest_inventory_and_blobs(
        &self,
        manifest: &StoredManifest,
        store: &mut KnowledgeStore,
    ) -> Result<()> {
        let program_path = manifest
            .program_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("migration manifest program path is missing"))?;
        let (snapshots, overlays) =
            store.migration_source_snapshot(program_path, manifest.created_at)?;
        let sources = snapshots
            .into_iter()
            .map(|snapshot| snapshot.source)
            .collect::<Vec<_>>();
        if sources != manifest.source_inventory || overlays != manifest.overlay_inventory {
            anyhow::bail!("migration physical inventory changed outside the transaction");
        }
        self.verify_manifest_blobs(manifest)
    }

    fn apply_partial_or_error(
        &self,
        manifest: &mut StoredManifest,
        commit: Option<crate::services::twin_events::MutationCommit>,
        error: anyhow::Error,
    ) -> Result<MigrationMutationOutcome<MarkdownMigrationApplyResult>> {
        log::error!("Markdown migration apply stopped after a committed step: {error}");
        let commit = commit.or_else(|| {
            (manifest.status == "apply_partial"
                && (manifest.apply_next > 0 || manifest.authority_only_advances > 0))
                .then(|| commit_from_manifest(manifest))
                .flatten()
        });
        let Some(commit) = commit else {
            return Err(error);
        };
        let authority = commit
            .authority_token
            .clone()
            .ok_or_else(|| anyhow::anyhow!("partial migration lost its exact authority"))?;
        let commit = commit.mutation_id.is_some().then_some(commit);
        manifest.status = "apply_partial".into();
        let _ = self.write_strict_manifest(manifest);
        let mut value = apply_result_from_manifest(manifest);
        value.status = "partial".into();
        value.message = "Markdown migration committed partially; recovery is required".into();
        value.warning = Some(fixed_warning());
        Ok(MigrationMutationOutcome::Partial {
            value,
            authority,
            commit,
            warning: fixed_warning(),
        })
    }

    fn rollback_partial_or_error(
        &self,
        manifest: &mut StoredManifest,
        commit: Option<crate::services::twin_events::MutationCommit>,
        error: anyhow::Error,
    ) -> Result<MigrationMutationOutcome<crate::models::migration::MarkdownMigrationRollbackResult>>
    {
        log::error!("Markdown migration rollback stopped after a committed step: {error}");
        let commit = commit.or_else(|| {
            (manifest.status == "rollback_partial"
                && (manifest.rollback_next > 0 || manifest.authority_only_advances > 0))
                .then(|| commit_from_manifest(manifest))
                .flatten()
        });
        let Some(commit) = commit else {
            return Err(error);
        };
        let authority = commit
            .authority_token
            .clone()
            .ok_or_else(|| anyhow::anyhow!("partial rollback lost its exact authority"))?;
        let commit = commit.mutation_id.is_some().then_some(commit);
        manifest.status = "rollback_partial".into();
        let _ = self.write_strict_manifest(manifest);
        let mut value = rollback_result_from_manifest(manifest, false);
        value.status = "partial".into();
        value.message =
            "Markdown migration rollback committed partially; recovery is required".into();
        value.warning = Some(fixed_warning());
        Ok(MigrationMutationOutcome::Partial {
            value,
            authority,
            commit,
            warning: fixed_warning(),
        })
    }

    pub(super) fn load_strict_preview(&self, preview_id: &str) -> Result<MarkdownMigrationPreview> {
        validate_canonical_uuid(preview_id, "migration preview ID")?;
        let bytes = self
            .strict_root()?
            .read_bounded(
                &format!("runs/{preview_id}/preview.json"),
                MAX_MIGRATION_RECORD_BYTES,
            )
            .map_err(anyhow::Error::new)?
            .ok_or_else(|| anyhow::anyhow!("migration preview does not exist"))?;
        let value: Value = serde_json::from_slice(&bytes)?;
        let schema = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("migration preview schema is missing"))?;
        if schema != u64::from(MIGRATION_PREVIEW_SCHEMA_VERSION) {
            anyhow::bail!("legacy migration preview is audit-only");
        }
        require_exact_fields(
            &value,
            &[
                "schema_version",
                "preview_id",
                "root_scope",
                "authority",
                "request",
                "vault_path",
                "created_at",
                "mode",
                "hub_folder",
                "program_path",
                "expected_program_target",
                "program_after_digest",
                "summary",
                "topic_candidates",
                "note_proposals",
                "source_inventory",
                "overlay_inventory",
                "ambiguous_titles",
            ],
        )?;
        let preview: MarkdownMigrationPreview = serde_json::from_value(value)?;
        validate_current_preview(&preview, preview_id)?;
        Ok(preview)
    }

    pub(super) fn load_strict_manifest(&self, run_id: &str) -> Result<StoredManifest> {
        self.load_strict_manifest_optional(run_id)?
            .ok_or_else(|| anyhow::anyhow!("migration manifest does not exist"))
    }

    fn load_strict_manifest_optional(&self, run_id: &str) -> Result<Option<StoredManifest>> {
        validate_canonical_uuid(run_id, "migration run ID")?;
        let Some(bytes) = self
            .strict_root()?
            .read_bounded(
                &format!("runs/{run_id}/manifest.json"),
                MAX_MIGRATION_RECORD_BYTES,
            )
            .map_err(anyhow::Error::new)?
        else {
            return Ok(None);
        };
        let value: Value = serde_json::from_slice(&bytes)?;
        let schema = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("migration manifest schema is missing"))?;
        if schema != u64::from(MIGRATION_MANIFEST_SCHEMA_VERSION) {
            anyhow::bail!("legacy migration manifest is audit-only");
        }
        require_exact_fields(
            &value,
            &[
                "schema_version",
                "run_id",
                "preview_id",
                "root_scope",
                "vault_path",
                "mode",
                "created_at",
                "applied_at",
                "status",
                "created_files",
                "backup_files",
                "overlay_note_ids",
                "touched_note_ids",
                "created_hub_note_ids",
                "skipped_fallback_note_ids",
                "expected_program_target",
                "program_after_digest",
                "program_path",
                "request",
                "source_inventory",
                "overlay_inventory",
                "starting_authority",
                "final_authority",
                "operations",
                "apply_next",
                "rollback_next",
                "rollback_total",
                "authority_only_advances",
                "active_step",
                "last_commit",
            ],
        )?;
        let manifest: StoredManifest = serde_json::from_value(value)?;
        if manifest.run_id != run_id || manifest.preview_id != run_id {
            anyhow::bail!("migration manifest identity mismatch");
        }
        self.validate_current_manifest(&manifest)?;
        self.verify_manifest_blobs(&manifest)?;
        let plan = self.load_immutable_plan(run_id)?;
        self.require_manifest_plan_binding(&manifest, &plan)?;
        self.require_manifest_commit_binding(&manifest)?;
        Ok(Some(manifest))
    }

    pub(super) fn write_strict_manifest(&self, manifest: &StoredManifest) -> Result<()> {
        #[cfg(test)]
        {
            let remaining = self
                .fail_manifest_write_at
                .load(std::sync::atomic::Ordering::SeqCst);
            if remaining > 0
                && self
                    .fail_manifest_write_at
                    .fetch_sub(1, std::sync::atomic::Ordering::SeqCst)
                    == 1
            {
                anyhow::bail!("injected migration manifest publication failure");
            }
        }
        validate_canonical_uuid(&manifest.run_id, "migration run ID")?;
        self.validate_current_manifest(manifest)?;
        let plan = self.load_immutable_plan(&manifest.run_id)?;
        self.require_manifest_plan_binding(manifest, &plan)?;
        self.require_manifest_commit_binding(manifest)?;
        let bytes = serde_json::to_vec_pretty(manifest)?;
        if bytes.len() > MAX_MIGRATION_RECORD_BYTES {
            anyhow::bail!("migration manifest exceeds its bounded record limit");
        }
        self.strict_root()?
            .put_atomic(&format!("runs/{}/manifest.json", manifest.run_id), &bytes)
            .map_err(anyhow::Error::new)
    }

    pub(super) fn validate_current_manifest(&self, manifest: &StoredManifest) -> Result<()> {
        validate_canonical_uuid(&manifest.run_id, "migration run ID")?;
        if manifest.schema_version != MIGRATION_MANIFEST_SCHEMA_VERSION
            || manifest.preview_id != manifest.run_id
            || manifest.request.is_none()
            || manifest.root_scope.is_none()
            || manifest.starting_authority.is_none()
            || manifest.final_authority.is_none()
            || manifest.apply_next > manifest.operations.len()
            || manifest.rollback_total > manifest.apply_next
            || manifest.rollback_next > manifest.rollback_total
            || manifest.authority_only_advances > MAX_MIGRATION_AUTHORITY_ONLY_ADVANCES
        {
            anyhow::bail!("migration manifest exact fields are invalid");
        }
        validate_operation_sequence(&manifest.operations)?;
        let program_path = manifest
            .program_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("migration manifest program path is missing"))?;
        if canonical_program_path(program_path)? != program_path {
            anyhow::bail!("migration manifest program path is not canonical");
        }
        let request = manifest.request.as_ref().expect("validated request");
        if request.program_path.as_deref() != Some(program_path) || request.mode != manifest.mode {
            anyhow::bail!("migration manifest request is inconsistent");
        }
        for operation in &manifest.operations {
            if !matches!(
                operation.target_kind,
                crate::services::twin_events::TargetKind::Markdown
                    | crate::services::twin_events::TargetKind::OverlayJson
            ) {
                anyhow::bail!("migration operation target kind is unsupported");
            }
            let after_key = format!("runs/{}/blobs/{}.after", manifest.run_id, operation.index);
            if operation.after_blob_key.as_deref() != Some(after_key.as_str()) {
                anyhow::bail!("migration operation after-image blob key is not canonical");
            }
            let before_key = format!("runs/{}/blobs/{}.before", manifest.run_id, operation.index);
            match (&operation.before, operation.before_blob_key.as_deref()) {
                (crate::services::twin_events::BeforeImage::Absent, None) => {}
                (crate::services::twin_events::BeforeImage::Sha256(_), Some(key))
                    if key == before_key => {}
                _ => anyhow::bail!("migration operation before-image blob key is invalid"),
            }
            match operation.target_kind {
                crate::services::twin_events::TargetKind::Markdown
                    if operation.target_key == program_path =>
                {
                    if operation.note_event.is_some() || operation.rollback_note_event.is_some() {
                        anyhow::bail!("migration program operation cannot carry note events");
                    }
                }
                crate::services::twin_events::TargetKind::Markdown => {
                    let apply = operation.note_event.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("migration Markdown operation lost its apply event")
                    })?;
                    let rollback = operation.rollback_note_event.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("migration Markdown operation lost its rollback event")
                    })?;
                    if apply.note_id != rollback.note_id || apply.governance != rollback.governance
                    {
                        anyhow::bail!("migration Markdown operation events are inconsistent");
                    }
                    match operation.before {
                        crate::services::twin_events::BeforeImage::Absent
                            if apply.change
                                == crate::models::twin_event::NoteChangeKind::Created
                                && rollback.change
                                    == crate::models::twin_event::NoteChangeKind::Deleted => {}
                        crate::services::twin_events::BeforeImage::Sha256(_)
                            if apply.change
                                == crate::models::twin_event::NoteChangeKind::Updated
                                && rollback.change
                                    == crate::models::twin_event::NoteChangeKind::Updated => {}
                        _ => anyhow::bail!("migration Markdown event direction is invalid"),
                    }
                }
                crate::services::twin_events::TargetKind::OverlayJson => {
                    let apply = operation.note_event.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("migration overlay operation lost its apply event")
                    })?;
                    let rollback = operation.rollback_note_event.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("migration overlay operation lost its rollback event")
                    })?;
                    if operation.target_key != format!("{}.json", apply.note_id)
                        || apply.note_id != rollback.note_id
                        || apply.governance != rollback.governance
                        || apply.change != crate::models::twin_event::NoteChangeKind::Updated
                        || rollback.change != crate::models::twin_event::NoteChangeKind::Updated
                    {
                        anyhow::bail!("migration overlay event binding is invalid");
                    }
                }
                _ => unreachable!("target kind checked above"),
            }
        }

        let source_ids = manifest
            .source_inventory
            .iter()
            .filter_map(|source| source.note_id.clone())
            .collect::<HashSet<_>>();
        require_unique_bounded(&manifest.overlay_note_ids, "overlay note IDs")?;
        require_unique_bounded(&manifest.touched_note_ids, "touched note IDs")?;
        require_unique_bounded(&manifest.created_hub_note_ids, "created hub note IDs")?;
        require_unique_bounded(&manifest.skipped_fallback_note_ids, "skipped note IDs")?;
        if manifest
            .overlay_note_ids
            .iter()
            .chain(manifest.touched_note_ids.iter())
            .chain(manifest.skipped_fallback_note_ids.iter())
            .any(|id| !source_ids.contains(id))
        {
            anyhow::bail!("migration result note IDs are not bound to source inventory");
        }
        let created_hubs = manifest
            .operations
            .iter()
            .filter(|operation| {
                operation.target_kind == crate::services::twin_events::TargetKind::Markdown
                    && operation.target_key != program_path
                    && matches!(
                        operation.before,
                        crate::services::twin_events::BeforeImage::Absent
                    )
            })
            .filter_map(|operation| {
                operation
                    .note_event
                    .as_ref()
                    .map(|event| event.note_id.clone())
            })
            .collect::<HashSet<_>>();
        if manifest
            .created_hub_note_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>()
            != created_hubs
        {
            anyhow::bail!("migration created hub result is inconsistent with operations");
        }
        let created_files = manifest
            .operations
            .iter()
            .filter(|operation| {
                operation.target_kind == crate::services::twin_events::TargetKind::Markdown
                    && matches!(
                        operation.before,
                        crate::services::twin_events::BeforeImage::Absent
                    )
            })
            .map(|operation| operation.target_key.clone())
            .collect::<HashSet<_>>();
        require_unique_bounded(&manifest.created_files, "created files")?;
        if manifest
            .created_files
            .iter()
            .cloned()
            .collect::<HashSet<_>>()
            != created_files
        {
            anyhow::bail!("migration created file result is inconsistent with operations");
        }
        let backup_files = manifest
            .operations
            .iter()
            .filter(|operation| {
                operation.target_kind == crate::services::twin_events::TargetKind::Markdown
                    && matches!(
                        operation.before,
                        crate::services::twin_events::BeforeImage::Sha256(_)
                    )
            })
            .map(|operation| operation.target_key.clone())
            .collect::<HashSet<_>>();
        require_unique_bounded(&manifest.backup_files, "backup files")?;
        if manifest
            .backup_files
            .iter()
            .cloned()
            .collect::<HashSet<_>>()
            != backup_files
        {
            anyhow::bail!("migration backup result is inconsistent with operations");
        }

        let root_scope = manifest.root_scope.as_ref().expect("validated root scope");
        let starting = manifest
            .starting_authority
            .as_ref()
            .expect("validated starting authority");
        let final_authority = manifest
            .final_authority
            .as_ref()
            .expect("validated final authority");
        validate_manifest_authority(starting, root_scope)?;
        validate_manifest_authority(final_authority, root_scope)?;
        if starting.lease_epoch_uuid != final_authority.lease_epoch_uuid {
            anyhow::bail!("migration manifest authority lease changed");
        }
        let committed_steps = u64::try_from(manifest.apply_next)?
            .checked_add(u64::try_from(manifest.rollback_next)?)
            .and_then(|total| total.checked_add(manifest.authority_only_advances))
            .ok_or_else(|| anyhow::anyhow!("migration manifest progress overflowed"))?;
        if final_authority.authority_generation
            != starting
                .authority_generation
                .checked_add(committed_steps)
                .ok_or_else(|| anyhow::anyhow!("migration authority generation overflowed"))?
        {
            anyhow::bail!("migration manifest authority chain is inconsistent");
        }
        match (&manifest.last_commit, committed_steps) {
            (None, 0) => {}
            (Some(proof), steps) if steps > 0 && proof.authority == *final_authority => {
                validate_manifest_authority(&proof.authority, root_scope)?;
            }
            _ => anyhow::bail!("migration manifest last commit is inconsistent"),
        }

        match manifest.status.as_str() {
            "prepared"
                if manifest.apply_next == 0
                    && manifest.rollback_next == 0
                    && manifest.rollback_total == 0
                    && manifest.authority_only_advances == 0
                    && manifest.applied_at.is_none() => {}
            "apply_partial"
                if manifest.rollback_next == 0
                    && manifest.rollback_total == 0
                    && (manifest.apply_next > 0
                        || manifest.authority_only_advances > 0
                        || manifest.active_step.is_some())
                    && manifest.applied_at.is_none() => {}
            "applied"
                if manifest.apply_next == manifest.operations.len()
                    && manifest.rollback_next == 0
                    && manifest.rollback_total == 0
                    && manifest.active_step.is_none()
                    && manifest.applied_at.is_some() => {}
            "rollback_prepared" | "rollback_partial"
                if manifest.apply_next > 0
                    && manifest.rollback_total == manifest.apply_next
                    && manifest.rollback_next <= manifest.rollback_total
                    && (manifest.status == "rollback_prepared" && manifest.rollback_next == 0
                        || manifest.status == "rollback_partial"
                            && (manifest.rollback_next > 0
                                || manifest.authority_only_advances > 0
                                || manifest.active_step.is_some()))
                    && manifest.applied_at.is_some() => {}
            "rolled_back"
                if manifest.apply_next > 0
                    && manifest.rollback_total == manifest.apply_next
                    && manifest.rollback_next == manifest.rollback_total
                    && manifest.active_step.is_none()
                    && manifest.applied_at.is_some() => {}
            _ => anyhow::bail!("migration manifest status and cursors are inconsistent"),
        }

        if let Some(witness) = manifest.active_step.as_ref() {
            let operation = manifest
                .operations
                .get(witness.operation_index)
                .ok_or_else(|| anyhow::anyhow!("migration witness operation is out of range"))?;
            if operation.index != witness.operation_index {
                anyhow::bail!("migration witness operation index is inconsistent");
            }
            let expected_index = match (witness.direction, witness.progress_recorded) {
                (MigrationDirectionV1::Apply, false) => manifest.apply_next,
                (MigrationDirectionV1::Apply, true) => manifest
                    .apply_next
                    .checked_sub(1)
                    .ok_or_else(|| anyhow::anyhow!("migration apply witness cursor underflow"))?,
                (MigrationDirectionV1::Rollback, false) => manifest
                    .rollback_total
                    .checked_sub(manifest.rollback_next + 1)
                    .ok_or_else(|| {
                        anyhow::anyhow!("migration rollback witness cursor underflow")
                    })?,
                (MigrationDirectionV1::Rollback, true) => manifest
                    .rollback_total
                    .checked_sub(manifest.rollback_next)
                    .ok_or_else(|| {
                        anyhow::anyhow!("migration rollback witness cursor underflow")
                    })?,
            };
            if witness.operation_index != expected_index
                || matches!(witness.direction, MigrationDirectionV1::Apply)
                    && !matches!(manifest.status.as_str(), "prepared" | "apply_partial")
                || matches!(witness.direction, MigrationDirectionV1::Rollback)
                    && !matches!(
                        manifest.status.as_str(),
                        "rollback_prepared" | "rollback_partial"
                    )
            {
                anyhow::bail!("migration witness direction or cursor is inconsistent");
            }
            validate_manifest_authority(&witness.expected_authority, root_scope)?;
            if witness.expected_authority.lease_epoch_uuid != starting.lease_epoch_uuid {
                anyhow::bail!("migration witness authority lease changed");
            }
            let authority_only_abort = witness.abort_recorded
                && !witness.progress_recorded
                && witness.committed_authority.is_some();
            if witness.abort_recorded && witness.progress_recorded
                || witness.progress_recorded && witness.committed_authority.is_none()
                || witness.intent.is_none()
                    && (witness.abort_recorded
                        || witness.progress_recorded
                        || witness.committed_authority.is_some())
            {
                anyhow::bail!("migration witness phase flags are inconsistent");
            }
            if witness.progress_recorded {
                let committed = witness
                    .committed_authority
                    .as_ref()
                    .expect("validated committed witness");
                let intent = witness.intent.as_ref().expect("validated progress intent");
                if committed != final_authority
                    || committed.authority_generation
                        != witness
                            .expected_authority
                            .authority_generation
                            .checked_add(1)
                            .ok_or_else(|| anyhow::anyhow!("migration authority overflowed"))?
                    || manifest
                        .last_commit
                        .as_ref()
                        .map(|proof| &proof.mutation_id)
                        != Some(&intent.mutation_id)
                {
                    anyhow::bail!("migration committed witness authority is inconsistent");
                }
            } else if authority_only_abort {
                let committed = witness
                    .committed_authority
                    .as_ref()
                    .expect("validated authority-only witness");
                let intent = witness.intent.as_ref().expect("validated abort intent");
                if committed != final_authority
                    || committed.authority_generation
                        != witness
                            .expected_authority
                            .authority_generation
                            .checked_add(1)
                            .ok_or_else(|| anyhow::anyhow!("migration authority overflowed"))?
                    || manifest
                        .last_commit
                        .as_ref()
                        .map(|proof| (&proof.mutation_id, &proof.authority))
                        != Some((&intent.mutation_id, committed))
                {
                    anyhow::bail!("migration authority-only witness is inconsistent");
                }
            } else {
                if witness.expected_authority != *final_authority {
                    anyhow::bail!("migration active witness does not chain from final authority");
                }
                if let Some(committed) = witness.committed_authority.as_ref() {
                    if committed.root_scope != witness.expected_authority.root_scope
                        || committed.lease_epoch_uuid != witness.expected_authority.lease_epoch_uuid
                        || committed.authority_generation
                            != witness
                                .expected_authority
                                .authority_generation
                                .checked_add(1)
                                .ok_or_else(|| anyhow::anyhow!("migration authority overflowed"))?
                    {
                        anyhow::bail!("migration pending committed authority is inconsistent");
                    }
                }
            }
            if let Some(intent) = witness.intent.as_ref() {
                self.validate_witness_intent(manifest, witness, operation, intent)?;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn prepare_apply_only_for_test(
        &self,
        preview: &MarkdownMigrationPreview,
        request: &MarkdownMigrationRequest,
        store: &mut KnowledgeStore,
    ) -> Result<()> {
        let created_at = preview
            .created_at
            .ok_or_else(|| anyhow::anyhow!("migration preview timestamp is missing"))?;
        let (snapshots, overlays) =
            store.migration_source_snapshot(&preview.program_path, created_at)?;
        let sources = snapshots
            .iter()
            .map(|snapshot| snapshot.source.clone())
            .collect::<Vec<_>>();
        if sources != preview.source_inventory || overlays != preview.overlay_inventory {
            anyhow::bail!("migration sources changed before test preparation");
        }
        let canonical = preview
            .request
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("migration preview request is missing"))?;
        if canonical_migration_request(
            request,
            canonical.hub_folder.as_deref().unwrap_or("_grafyn/hubs"),
            canonical
                .program_path
                .as_deref()
                .unwrap_or("_grafyn/program.md"),
        ) != *canonical
        {
            anyhow::bail!("migration test request differs from preview");
        }
        let prepared = self.prepare_apply_plan(preview, canonical, snapshots, store)?;
        self.persist_prepared_plan(&prepared)
    }

    #[cfg(test)]
    pub(super) fn install_prehook_witness_for_test(&self, run_id: &str) -> Result<()> {
        let mut manifest = self.load_strict_manifest(run_id)?;
        let operation = manifest
            .operations
            .get(manifest.apply_next)
            .ok_or_else(|| anyhow::anyhow!("migration test has no pending operation"))?;
        manifest.active_step = Some(MigrationStepWitnessV1 {
            direction: MigrationDirectionV1::Apply,
            operation_index: operation.index,
            expected_authority: manifest
                .final_authority
                .clone()
                .ok_or_else(|| anyhow::anyhow!("migration test authority is missing"))?,
            intent: None,
            committed_authority: None,
            progress_recorded: false,
            receipt_consumed: false,
            abort_recorded: false,
        });
        self.write_strict_manifest(&manifest)
    }

    #[cfg(test)]
    pub(super) fn mark_rollback_resume_for_test(&self, run_id: &str) -> Result<()> {
        let mut manifest = self.load_strict_manifest(run_id)?;
        manifest.status = "rollback_prepared".into();
        manifest.rollback_total = manifest.apply_next;
        manifest.rollback_next = 0;
        manifest.active_step = None;
        self.write_strict_manifest(&manifest)
    }

    #[cfg(test)]
    pub(super) fn fail_manifest_write_at_for_test(&self, ordinal: usize) {
        self.fail_manifest_write_at
            .store(ordinal, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(super) fn set_blob_byte_limit_for_test(&self, limit: usize) {
        self.blob_byte_limit
            .store(limit, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(super) fn set_status_scan_byte_limit_for_test(&self, limit: usize) {
        self.status_scan_byte_limit
            .store(limit, std::sync::atomic::Ordering::SeqCst);
    }
}

fn push_put_operation(
    run_id: &str,
    operations: &mut Vec<MigrationOperationV1>,
    blobs: &mut Vec<(String, Vec<u8>)>,
    target_kind: crate::services::twin_events::TargetKind,
    target_key: String,
    before: crate::services::twin_events::BeforeImage,
    after_bytes: Vec<u8>,
    before_bytes: Option<Vec<u8>>,
) -> Result<()> {
    if operations.len() >= MAX_MIGRATION_OPERATIONS {
        anyhow::bail!("migration operation inventory is too large");
    }
    let index = operations.len();
    let after_digest = crate::services::twin_events::digest_bytes(&after_bytes);
    let after_blob_key = format!("runs/{run_id}/blobs/{index}.after");
    let before_blob_key = before_bytes
        .as_ref()
        .map(|_| format!("runs/{run_id}/blobs/{index}.before"));
    if let Some(bytes) = before_bytes {
        let expected = match &before {
            crate::services::twin_events::BeforeImage::Sha256(digest) => digest,
            crate::services::twin_events::BeforeImage::Absent => {
                anyhow::bail!("migration operation has bytes for an absent before-image")
            }
        };
        if crate::services::twin_events::digest_bytes(&bytes) != *expected {
            anyhow::bail!("migration backup bytes do not match their before-image");
        }
        blobs.push((before_blob_key.clone().expect("before key"), bytes));
    } else if matches!(before, crate::services::twin_events::BeforeImage::Sha256(_)) {
        anyhow::bail!("migration operation is missing its before bytes");
    }
    blobs.push((after_blob_key.clone(), after_bytes));
    operations.push(MigrationOperationV1 {
        index,
        target_kind,
        target_key,
        before,
        after: crate::services::twin_events::BeforeImage::Sha256(after_digest.clone()),
        after_digest,
        after_blob_key: Some(after_blob_key),
        before_blob_key,
        note_event: None,
        rollback_note_event: None,
    });
    Ok(())
}

pub(super) fn desired_for_state(
    state: crate::services::twin_events::BeforeImage,
    blob_key: Option<&str>,
    manifest: &StoredManifest,
    service: &MarkdownMigrationService,
) -> Result<crate::services::twin_events::DesiredImage> {
    match state {
        crate::services::twin_events::BeforeImage::Absent => {
            if blob_key.is_some() {
                anyhow::bail!("absent migration image unexpectedly has a blob");
            }
            Ok(crate::services::twin_events::DesiredImage::Tombstone)
        }
        crate::services::twin_events::BeforeImage::Sha256(expected) => {
            let key = blob_key.ok_or_else(|| anyhow::anyhow!("migration image blob is missing"))?;
            let bytes = service.read_blob(manifest, key)?;
            if crate::services::twin_events::digest_bytes(&bytes) != expected {
                anyhow::bail!("migration image blob digest changed");
            }
            Ok(crate::services::twin_events::DesiredImage::Utf8Bytes(
                String::from_utf8(bytes).context("migration image is not UTF-8")?,
            ))
        }
    }
}

fn stored_note_event(
    event: crate::services::knowledge_store::ExactMigrationNoteEvent,
) -> StoredMigrationNoteEventV1 {
    StoredMigrationNoteEventV1 {
        note_id: event.note_id,
        change: event.change,
        observed_at: event.observed_at,
        governance: event.governance,
        payload_digest: event.payload_digest,
        evidence_digest: event.evidence_digest,
    }
}

fn migration_overlay_state_digest(
    markdown_digest: &crate::models::twin_event::ContentDigest,
    overlay: &crate::services::twin_events::BeforeImage,
) -> crate::models::twin_event::ContentDigest {
    let overlay_state = match overlay {
        crate::services::twin_events::BeforeImage::Absent => "absent",
        crate::services::twin_events::BeforeImage::Sha256(digest) => digest.as_str(),
    };
    crate::services::twin_events::digest_bytes(
        format!(
            "grafyn.migration.overlay-state.v1\n{}\n{overlay_state}",
            markdown_digest.as_str()
        )
        .as_bytes(),
    )
}

fn exact_note_event(
    event: &StoredMigrationNoteEventV1,
) -> crate::services::knowledge_store::ExactMigrationNoteEvent {
    crate::services::knowledge_store::ExactMigrationNoteEvent {
        note_id: event.note_id.clone(),
        change: event.change.clone(),
        observed_at: event.observed_at,
        governance: event.governance.clone(),
        payload_digest: event.payload_digest.clone(),
        evidence_digest: event.evidence_digest.clone(),
    }
}

fn append_related_links_from_snapshot(
    content: &str,
    inferred_link_ids: &[String],
    titles: &HashMap<String, String>,
) -> Result<(String, Vec<String>)> {
    let mut additions = Vec::new();
    let mut inserted = Vec::new();
    for note_id in inferred_link_ids.iter().take(3) {
        let title = titles
            .get(note_id)
            .ok_or_else(|| anyhow::anyhow!("migration inferred link target is absent"))?;
        if !content.contains(&format!("[[{title}]]")) {
            additions.push(title.clone());
            inserted.push(note_id.clone());
        }
    }
    if additions.is_empty() {
        return Ok((content.into(), inserted));
    }
    let mut rewritten = content.trim_end().to_string();
    rewritten.push_str("\n\n## Related Notes\n");
    for title in additions {
        rewritten.push_str(&format!("- [[{title}]]\n"));
    }
    Ok((rewritten, inserted))
}

fn apply_result_from_manifest(manifest: &StoredManifest) -> MarkdownMigrationApplyResult {
    MarkdownMigrationApplyResult {
        run_id: manifest.run_id.clone(),
        status: manifest.status.clone(),
        created_hub_note_ids: manifest.created_hub_note_ids.clone(),
        touched_note_ids: manifest.touched_note_ids.clone(),
        overlay_note_ids: manifest.overlay_note_ids.clone(),
        skipped_fallback_note_ids: manifest.skipped_fallback_note_ids.clone(),
        message: if manifest.skipped_fallback_note_ids.is_empty() {
            "Markdown migration applied".into()
        } else {
            format!(
                "Markdown migration applied ({} note(s) skipped: unparsable frontmatter preserved verbatim)",
                manifest.skipped_fallback_note_ids.len()
            )
        },
        warning: None,
        accepted_request: manifest.request.clone(),
    }
}

fn rollback_result_from_manifest(
    manifest: &StoredManifest,
    rolled_back: bool,
) -> crate::models::migration::MarkdownMigrationRollbackResult {
    crate::models::migration::MarkdownMigrationRollbackResult {
        run_id: manifest.run_id.clone(),
        rolled_back,
        status: manifest.status.clone(),
        message: if rolled_back {
            "Markdown migration rolled back".into()
        } else {
            "Markdown migration rollback is incomplete".into()
        },
        warning: None,
    }
}

fn commit_from_manifest(
    manifest: &StoredManifest,
) -> Option<crate::services::twin_events::MutationCommit> {
    manifest
        .last_commit
        .as_ref()
        .map(|proof| crate::services::twin_events::MutationCommit {
            mutation_id: Some(proof.mutation_id.clone()),
            events: Vec::new(),
            authority_token: Some(proof.authority.clone()),
            postcommit_warning: false,
        })
}

pub(super) fn commit_from_witness(
    witness: &MigrationStepWitnessV1,
) -> Result<crate::services::twin_events::MutationCommit> {
    Ok(crate::services::twin_events::MutationCommit {
        mutation_id: Some(
            witness
                .intent
                .as_ref()
                .map(|intent| intent.mutation_id.clone())
                .ok_or_else(|| anyhow::anyhow!("migration witness mutation ID is missing"))?,
        ),
        events: Vec::new(),
        authority_token: Some(
            witness
                .committed_authority
                .clone()
                .ok_or_else(|| anyhow::anyhow!("migration witness authority is missing"))?,
        ),
        postcommit_warning: true,
    })
}

fn fixed_warning() -> crate::models::mutation::CommittedMutationWarningV1 {
    crate::models::mutation::CommittedMutationWarningV1::derived_state_unavailable()
}
