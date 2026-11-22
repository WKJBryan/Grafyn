use crate::models::twin::{TwinPrediction, TwinPredictionDraft};
use serde_json::Value;

const PREDICTION_OPTION_MAX_CHARS: usize = 500;

const PREDICTION_RATIONALE_MAX_CHARS: usize = 2000;

/// Normalize an option string for comparison: trim, strip wrapping quotes,
/// lowercase, collapse whitespace. Label forms ("Option 2", "B") are handled
/// separately by `match_option_index` so meaningful digits survive.
pub fn normalize_option(text: &str) -> String {
    text.trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        text.chars().take(max_chars).collect()
    }
}

pub(super) fn match_option_index(text: &str, options: &[String]) -> Option<usize> {
    let normalized = normalize_option(text);
    if normalized.is_empty() {
        return None;
    }

    // Label forms: "option 2" / "choice b" / bare "2" / bare "b".
    let label = ["option", "choice"]
        .iter()
        .find_map(|prefix| normalized.strip_prefix(prefix))
        .map(str::trim)
        .unwrap_or(&normalized)
        .trim_matches(|c: char| c == '.' || c == ')' || c == ':' || c.is_whitespace());
    if label.len() == 1 {
        if let Some(letter) = label.chars().next() {
            if letter.is_ascii_lowercase() {
                let index = (letter as usize) - ('a' as usize);
                if index < options.len() {
                    return Some(index);
                }
            }
        }
    }
    // Bare number labels: humans count from 1.
    if let Ok(number) = label.parse::<usize>() {
        if (1..=options.len()).contains(&number) {
            return Some(number - 1);
        }
    }

    options
        .iter()
        .position(|option| normalize_option(option) == normalized)
}

fn sanitize_confidence(raw: Option<f64>) -> Option<f32> {
    let value = raw?;
    if !value.is_finite() {
        return None;
    }
    let value = if value > 2.0 && value <= 100.0 {
        // Percent-style answer ("73" meaning 73%).
        value / 100.0
    } else {
        // Near-misses like 1.2 are over-confident, not percentages.
        value
    };
    Some(value.clamp(0.0, 1.0) as f32)
}

/// Keep the model's per-option vector only when it is exactly one finite
/// probability in [0,1] per option. Never renormalized: the harness scores
/// what the model actually said.
fn sanitize_option_probabilities(raw: Option<&Value>, option_count: usize) -> Option<Vec<f32>> {
    let items = raw?.as_array()?;
    if items.len() != option_count {
        return None;
    }
    items
        .iter()
        .map(|item| {
            item.as_f64()
                .filter(|p| p.is_finite() && (0.0..=1.0).contains(p))
                .map(|p| p as f32)
        })
        .collect()
}

/// Extract the first balanced `{...}` span from `raw`, for best-effort JSON parsing of
/// model output. Returns `None` if there's no opening brace, no closing brace, or the
/// closing brace appears before the opening one (e.g. truncated/malformed model output
/// like `"Option A} — but {incomplete"`) — guarding this instead of blindly slicing
/// `&raw[start..=end]` is what prevents a panic on malformed input.
pub(super) fn extract_json_slice(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if start > end {
        return None;
    }
    Some(&raw[start..=end])
}

/// Parse the raw model output of a sealed-prediction call into a draft.
/// Fallback chain: fenced/embedded strict JSON -> normalized string match
/// against `options` -> raw text (manual adjudication later).
pub fn parse_twin_prediction(raw: &str, options: &[String]) -> TwinPredictionDraft {
    #[derive(serde::Deserialize)]
    struct ParsedPrediction {
        predicted_option: Option<String>,
        option_index: Option<Value>,
        confidence: Option<f64>,
        rationale: Option<String>,
        // Raw Value so a malformed vector cannot fail the whole JSON parse.
        option_probabilities: Option<Value>,
    }

    let json_slice = extract_json_slice(raw);

    if let Some(slice) = json_slice {
        if let Ok(parsed) = serde_json::from_str::<ParsedPrediction>(slice) {
            let text = parsed.predicted_option.unwrap_or_default();
            let text_index = match_option_index(&text, options);
            let field_index = parsed.option_index.as_ref().and_then(|value| match value {
                Value::Number(number) => number.as_u64().map(|n| n as usize),
                Value::String(text) => text.trim().parse::<usize>().ok(),
                _ => None,
            });
            // The prompt displays a 1-based option list, so prefer the
            // 1-based reading; tolerate 0-based answers, and let the option
            // text disambiguate when both readings are valid.
            let field_index = field_index.and_then(|index| {
                let one_based = index.checked_sub(1).filter(|i| *i < options.len());
                let zero_based = Some(index).filter(|i| *i < options.len());
                match (one_based, zero_based) {
                    (Some(ob), Some(zb)) => match text_index {
                        Some(text_idx) if text_idx == zb => Some(zb),
                        _ => Some(ob),
                    },
                    (Some(ob), None) => Some(ob),
                    (None, Some(zb)) => Some(zb),
                    (None, None) => None,
                }
            });
            // The option text is what the model actually said; a conflicting
            // numeric index loses.
            let matched_option_index = match (text_index, field_index) {
                (Some(text_idx), _) => Some(text_idx),
                (None, Some(field_idx)) if text.is_empty() => Some(field_idx),
                _ => None,
            };
            let predicted_option = if text.is_empty() {
                matched_option_index
                    .map(|index| options[index].clone())
                    .unwrap_or_else(|| truncate_chars(raw.trim(), PREDICTION_OPTION_MAX_CHARS))
            } else {
                truncate_chars(&text, PREDICTION_OPTION_MAX_CHARS)
            };
            return TwinPredictionDraft {
                predicted_option,
                matched_option_index,
                confidence: sanitize_confidence(parsed.confidence),
                rationale: parsed
                    .rationale
                    .map(|text| truncate_chars(&text, PREDICTION_RATIONALE_MAX_CHARS)),
                parse_mode: "json".to_string(),
                option_probabilities: sanitize_option_probabilities(
                    parsed.option_probabilities.as_ref(),
                    options.len(),
                ),
            };
        }
    }

    if let Some(index) = match_option_index(raw, options) {
        return TwinPredictionDraft {
            predicted_option: options[index].clone(),
            matched_option_index: Some(index),
            confidence: None,
            rationale: None,
            parse_mode: "string_match".to_string(),
            option_probabilities: None,
        };
    }

    TwinPredictionDraft {
        predicted_option: truncate_chars(raw.trim(), PREDICTION_OPTION_MAX_CHARS),
        matched_option_index: None,
        confidence: None,
        rationale: None,
        parse_mode: "raw".to_string(),
        option_probabilities: None,
    }
}

pub(super) fn compute_agreement(
    prediction: &TwinPrediction,
    chosen: &str,
    options: &[String],
) -> bool {
    let chosen_index = match_option_index(chosen, options);
    match (prediction.matched_option_index, chosen_index) {
        (Some(predicted), Some(chosen)) => predicted == chosen,
        _ => normalize_option(&prediction.predicted_option) == normalize_option(chosen),
    }
}
