use super::*;

pub(super) struct DerivedPreviewSemantics {
    pub(super) expected_program_target: ExpectedProgramTarget,
    pub(super) program_after_digest: crate::models::twin_event::ContentDigest,
    pub(super) summary: MarkdownMigrationPreviewSummary,
    pub(super) topic_candidates: Vec<MarkdownMigrationTopicCandidate>,
    pub(super) note_proposals: Vec<MarkdownMigrationNoteProposal>,
    pub(super) ambiguous_titles: HashMap<String, Vec<String>>,
}

pub(super) fn derive_preview_semantics(
    source_snapshots: &[crate::services::knowledge_store::MigrationMarkdownSnapshot],
    request: &MarkdownMigrationRequest,
    hub_folder: &str,
    program_path: &str,
) -> Result<DerivedPreviewSemantics> {
    let notes = source_snapshots
        .iter()
        .filter_map(|snapshot| snapshot.note.clone())
        .collect::<Vec<_>>();
    let mut source_by_path = BTreeMap::new();
    let mut note_paths = HashSet::new();
    for snapshot in source_snapshots {
        let lookup_key = migration_path_key(&snapshot.source.relative_path);
        if source_by_path
            .insert(lookup_key.clone(), snapshot)
            .is_some()
        {
            anyhow::bail!("duplicate or aliased Markdown path in migration snapshot");
        }
        if snapshot.note.is_some() {
            note_paths.insert(lookup_key);
        }
    }
    let program_contents = default_program_file_contents(hub_folder, program_path);
    let program_after_digest =
        crate::services::twin_events::digest_bytes(program_contents.as_bytes());
    let program_lookup_key = migration_path_key(program_path);
    let expected_program_target = match source_by_path.get(&program_lookup_key) {
        Some(snapshot) => {
            if snapshot.note.is_some() {
                anyhow::bail!("migration program path was included in note planning");
            }
            ExpectedProgramTarget::Present {
                digest: snapshot.source.markdown_digest.clone(),
            }
        }
        None => ExpectedProgramTarget::Absent,
    };

    let resolution_index = build_reference_index(&notes);
    let mut existing_hubs = HashMap::new();
    for note in notes.iter().filter(|note| note.is_topic_hub()) {
        if let Some(key) = note.topic_key() {
            if existing_hubs.insert(key.clone(), note.id.clone()).is_some() {
                anyhow::bail!("duplicate topic hub key in migration snapshot: {key}");
            }
        }
    }

    let mut summary = MarkdownMigrationPreviewSummary::default();
    let mut topic_buckets: BTreeMap<String, MarkdownMigrationTopicCandidate> = BTreeMap::new();
    let mut note_proposals = Vec::new();
    let mut ambiguous_titles = HashMap::new();
    summary.total_scanned_notes = notes.len();

    for note in &notes {
        let source = source_by_path
            .get(&migration_path_key(&note.relative_path))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "parsed note is absent from the physical migration inventory: {}",
                    note.relative_path
                )
            })?;
        if source.source.note_id.as_deref() != Some(note.id.as_str()) {
            anyhow::bail!(
                "migration note identity changed during source scan: {}",
                note.relative_path
            );
        }
        let raw_content = String::from_utf8(source.raw_bytes.clone()).with_context(|| {
            format!(
                "Markdown is not UTF-8 during migration preview: {}",
                note.relative_path
            )
        })?;
        let has_frontmatter = raw_content.trim_start().starts_with("---");
        if !has_frontmatter {
            summary.files_without_frontmatter += 1;
        }
        if !raw_content.trim_start().starts_with("---\n")
            || note.title == humanize_title(&note.relative_path)
        {
            summary.inferred_titles += 1;
        }
        if !note.aliases.is_empty() {
            summary.inferred_aliases += note.aliases.len();
        }

        let inferred_tags = infer_topic_tags(note);
        if !inferred_tags.is_empty() {
            summary.inferred_tags_or_topic_seeds += inferred_tags.len();
        }
        let mut markdown_resolved = 0usize;
        let mut wikilinks_resolved = 0usize;
        for parsed_link in &note.parsed_links {
            if let Some(target_path) = &parsed_link.target_path {
                if note_paths.contains(&migration_path_key(target_path)) {
                    markdown_resolved += 1;
                }
            } else if resolve_reference(parsed_link.target_title.as_str(), &resolution_index)
                .is_some()
            {
                wikilinks_resolved += 1;
            }
        }
        summary.markdown_links_resolved += markdown_resolved;
        summary.wikilinks_resolved += wikilinks_resolved;

        let collisions = find_ambiguous_references(note, &resolution_index);
        if !collisions.is_empty() {
            summary.ambiguous_matches += collisions.len();
            ambiguous_titles.extend(collisions);
        }
        let inferred_link_ids = infer_unlinked_note_mentions(note, &resolution_index);
        let topic_key = inferred_tags
            .first()
            .map(|value| normalize_topic_key(value))
            .filter(|value| !value.is_empty());
        let confidence = if note.tags.is_empty() { 0.78 } else { 0.91 };
        let write_required = request.mode.allows_user_note_writes()
            && (!inferred_link_ids.is_empty()
                || note.schema_version < CURRENT_NOTE_SCHEMA_VERSION
                || !inferred_tags.is_empty());
        if write_required {
            summary.files_to_rewrite += 1;
        }
        if !inferred_link_ids.is_empty() && request.mode.allows_user_note_writes() {
            summary.proposed_auto_link_edits += inferred_link_ids.len().min(3);
        }
        if note.schema_version < CURRENT_NOTE_SCHEMA_VERSION && note.migration_source.is_none() {
            summary.old_grafyn_notes_eligible_for_backfill += 1;
        }
        if let Some(topic_key) = &topic_key {
            let display_name = display_topic_name(topic_key);
            let entry = topic_buckets.entry(topic_key.clone()).or_insert_with(|| {
                MarkdownMigrationTopicCandidate {
                    topic_key: topic_key.clone(),
                    display_name: display_name.clone(),
                    reuse_existing_hub_id: existing_hubs.get(topic_key).cloned(),
                    ..Default::default()
                }
            });
            entry.member_note_ids.push(note.id.clone());
            entry.member_note_titles.push(note.title.clone());
        }
        note_proposals.push(MarkdownMigrationNoteProposal {
            note_id: note.id.clone(),
            title: note.title.clone(),
            relative_path: note.relative_path.clone(),
            aliases: note.aliases.clone(),
            inferred_tags,
            inferred_link_ids,
            topic_key,
            confidence,
            write_required,
        });
    }
    let topic_candidates = topic_buckets.into_values().collect::<Vec<_>>();
    summary.proposed_hubs = topic_candidates
        .iter()
        .filter(|candidate| candidate.reuse_existing_hub_id.is_none())
        .count();
    summary.files_to_create = summary.proposed_hubs
        + usize::from(matches!(
            &expected_program_target,
            ExpectedProgramTarget::Absent
        ));
    Ok(DerivedPreviewSemantics {
        expected_program_target,
        program_after_digest,
        summary,
        topic_candidates,
        note_proposals,
        ambiguous_titles,
    })
}

pub(super) fn require_exact_preview_semantics(
    preview: &MarkdownMigrationPreview,
    derived: &DerivedPreviewSemantics,
) -> Result<()> {
    let actual = serde_json::to_value((
        &preview.expected_program_target,
        &preview.program_after_digest,
        &preview.summary,
        &preview.topic_candidates,
        &preview.note_proposals,
        &preview.ambiguous_titles,
    ))?;
    let expected = serde_json::to_value((
        Some(&derived.expected_program_target),
        Some(&derived.program_after_digest),
        &derived.summary,
        &derived.topic_candidates,
        &derived.note_proposals,
        &derived.ambiguous_titles,
    ))?;
    if actual != expected {
        anyhow::bail!("migration preview semantics differ from its exact source snapshot");
    }
    Ok(())
}
