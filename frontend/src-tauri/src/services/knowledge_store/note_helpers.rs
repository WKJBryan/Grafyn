use super::*;

pub(super) fn note_update_changes(note: &Note, update: &NoteUpdate) -> Result<bool> {
    if update
        .title
        .as_ref()
        .is_some_and(|value| value != &note.title)
        || update
            .content
            .as_ref()
            .is_some_and(|value| value != &note.content)
        || update
            .aliases
            .as_ref()
            .is_some_and(|value| dedupe_strings(value.clone()) != note.aliases)
        || update
            .status
            .as_ref()
            .is_some_and(|value| value != &note.status)
        || update
            .tags
            .as_ref()
            .is_some_and(|value| dedupe_strings(value.clone()) != note.tags)
        || update
            .schema_version
            .is_some_and(|value| value.max(CURRENT_NOTE_SCHEMA_VERSION) != note.schema_version)
        || update
            .migration_source
            .as_ref()
            .is_some_and(|value| Some(value) != note.migration_source.as_ref())
        || update
            .optimizer_managed
            .is_some_and(|value| value != note.optimizer_managed)
        || update
            .properties
            .as_ref()
            .is_some_and(|value| value != &note.properties)
    {
        return Ok(true);
    }
    if let Some(relative_path) = &update.relative_path {
        return Ok(normalize_note_relative_path(relative_path)? != note.relative_path);
    }
    Ok(false)
}

pub(super) fn note_capture_governance(
    note: &Note,
    source_channel: &str,
) -> crate::models::twin_event::Governance {
    let mut governance = if source_channel == "import" {
        crate::services::twin_events::imported_capture_governance()
    } else {
        crate::services::twin_events::standard_capture_governance()
    };
    let local_only = note
        .properties
        .get("grafyn_sync")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "local_only")
        || note
            .properties
            .get("private")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let sensitivity = note
        .properties
        .get("sensitivity")
        .and_then(Value::as_str)
        .map(|value| match value {
            "restricted" | "private" => crate::models::twin_event::Sensitivity::Restricted,
            "sensitive" => crate::models::twin_event::Sensitivity::Sensitive,
            _ => crate::models::twin_event::Sensitivity::Standard,
        })
        .unwrap_or(governance.sensitivity.clone());
    if local_only || sensitivity == crate::models::twin_event::Sensitivity::Restricted {
        governance = crate::services::twin_events::local_capture_governance(sensitivity);
    } else {
        governance.sensitivity = sensitivity;
    }
    governance
}

pub(super) fn normalize_lookup_key(value: &str) -> String {
    value.trim().replace('\\', "/").to_lowercase()
}

pub(super) fn migration_paths_equal(left: &str, right: &str) -> bool {
    #[cfg(windows)]
    {
        left.eq_ignore_ascii_case(right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

pub(super) fn migration_physical_path_key(value: &str) -> String {
    #[cfg(windows)]
    {
        value.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        value.to_string()
    }
}

pub(super) fn normalize_relative_path_for_output(value: &str) -> String {
    value
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

/// Windows reserved device names — invalid as a file/directory stem
/// regardless of extension (e.g. `con`, `CON.md`, `con.backup.md`).
/// Checked platform-independently: a vault synced across OSes must not
/// contain files that are unopenable on Windows.
pub(super) const RESERVED_WINDOWS_STEMS: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Returns true if `component` (an id or a single path segment, with or
/// without an extension) is a Windows-reserved device name. The reserved
/// stem is the text before the *first* dot, matched case-insensitively, so
/// `con.backup.md` is still reserved. Windows additionally strips trailing
/// spaces and dots before device-name resolution (`con .md` still reaches
/// the CON device), so the stem is trimmed of those before comparison.
pub(super) fn is_reserved_windows_component(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    let stem = stem.trim_end_matches([' ', '.']);
    RESERVED_WINDOWS_STEMS
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

/// Belt-and-braces check run after joining a (validated) relative path onto
/// the vault root: confirms the resolved path is still lexically nested
/// under `vault_path`. This is a pure component walk — no filesystem
/// canonicalize, since the target may not exist yet (e.g. a note being
/// created). Catches anything the string-level validators might miss,
/// including Windows drive-relative joins (`PathBuf::join` replaces the
/// base entirely when the argument carries its own drive prefix).
pub(super) fn ensure_path_within_vault(vault_path: &Path, resolved: &Path) -> Result<()> {
    let remainder = resolved.strip_prefix(vault_path).map_err(|_| {
        anyhow::anyhow!(
            "Resolved note path escapes the vault: {}",
            resolved.display()
        )
    })?;
    for component in remainder.components() {
        match component {
            std::path::Component::Prefix(_)
            | std::path::Component::RootDir
            | std::path::Component::ParentDir => {
                anyhow::bail!(
                    "Resolved note path escapes the vault: {}",
                    resolved.display()
                );
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn normalize_note_relative_path(value: &str) -> Result<String> {
    let normalized = normalize_relative_path_for_output(value)
        .trim_matches('/')
        .to_string();
    if normalized.is_empty() {
        anyhow::bail!("Relative note path cannot be empty");
    }
    if Path::new(&normalized).is_absolute() {
        anyhow::bail!("Absolute note paths are not allowed");
    }
    if normalized.contains(':') {
        anyhow::bail!(
            "Note paths must not contain ':' (drive-relative or alternate-data-stream syntax is not allowed): {}",
            normalized
        );
    }
    for segment in normalized.split('/') {
        if segment.is_empty() || segment == ".." {
            anyhow::bail!("Path traversal is not allowed in note paths");
        }
        if is_reserved_windows_component(segment) {
            anyhow::bail!(
                "Note paths must not use a reserved device name: {}",
                segment
            );
        }
    }
    if normalized.to_lowercase().ends_with(".md") {
        Ok(normalized)
    } else {
        Ok(format!("{}.md", normalized))
    }
}

#[allow(dead_code)] // Used by bounded legacy migration recovery helpers.
pub(super) fn before_image_for_path(
    path: &Path,
) -> Result<crate::services::twin_events::BeforeImage> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(crate::services::twin_events::BeforeImage::Sha256(
            crate::services::twin_events::digest_bytes(&bytes),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(crate::services::twin_events::BeforeImage::Absent)
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) fn slugify(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else if character.is_whitespace()
                || character == '-'
                || character == '_'
                || character == '/'
            {
                '-'
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

pub(super) fn humanize_filename(value: &str) -> String {
    value
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn alias_candidates(title: &str, file_stem: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let humanized = humanize_filename(file_stem);
    if !humanized.trim().is_empty() && !humanized.eq_ignore_ascii_case(title.trim()) {
        candidates.push(humanized);
    }
    let compact = file_stem.replace(['-', '_'], "");
    if !compact.is_empty()
        && !compact.eq_ignore_ascii_case(file_stem)
        && !compact.eq_ignore_ascii_case(title)
    {
        candidates.push(compact);
    }
    candidates
}

pub(super) fn extract_inline_hashtags(content: &str) -> Vec<String> {
    HASHTAG_REGEX
        .captures_iter(content)
        .filter_map(|caps| caps.get(1).map(|value| value.as_str().trim().to_string()))
        .filter(|value| !value.is_empty())
        .collect()
}

pub(super) fn dedupe_strings<I>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let owned = trimmed.to_string();
        let key = owned.to_lowercase();
        if seen.insert(key) {
            result.push(owned);
        }
    }
    result
}
