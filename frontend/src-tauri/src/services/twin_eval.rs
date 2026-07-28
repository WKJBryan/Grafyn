use anyhow::{anyhow, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TwinEvalProvider {
    Ollama,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TwinEvalContextMode {
    SystemOnly,
    RetrievalOnly,
    ConstitutionOnly,
    RetrievalAndConstitution,
}

impl Default for TwinEvalContextMode {
    fn default() -> Self {
        TwinEvalContextMode::RetrievalAndConstitution
    }
}

impl TwinEvalContextMode {
    pub fn uses_retrieval(&self) -> bool {
        matches!(
            self,
            TwinEvalContextMode::RetrievalOnly | TwinEvalContextMode::RetrievalAndConstitution
        )
    }

    pub fn uses_constitution(&self) -> bool {
        matches!(
            self,
            TwinEvalContextMode::ConstitutionOnly | TwinEvalContextMode::RetrievalAndConstitution
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwinEvalOption {
    pub key: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwinEvalLabQuestion {
    pub id: String,
    pub raw_input: String,
    pub question: String,
    #[serde(default)]
    pub options: Vec<TwinEvalOption>,
    #[serde(default)]
    pub answer_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwinEvalContextSource {
    pub source_type: String,
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub snippet: String,
    #[serde(default)]
    pub weight: Option<f32>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwinEvalContextPacket {
    pub mode: TwinEvalContextMode,
    #[serde(default)]
    pub retrieval_items: Vec<TwinEvalContextSource>,
    #[serde(default)]
    pub constitution_items: Vec<TwinEvalContextSource>,
    #[serde(default)]
    pub action_gaps: Vec<TwinEvalContextSource>,
    pub system_prompt: String,
    pub private_store_accessed: bool,
}

impl TwinEvalContextPacket {
    pub fn empty(mode: TwinEvalContextMode) -> Self {
        Self {
            private_store_accessed: mode.uses_retrieval() || mode.uses_constitution(),
            mode,
            retrieval_items: Vec::new(),
            constitution_items: Vec::new(),
            action_gaps: Vec::new(),
            system_prompt: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwinEvalModelConfig {
    pub key: String,
    pub label: String,
    pub provider: TwinEvalProvider,
    pub model_id: String,
    pub runner_model_id: String,
    pub family: String,
    pub training_stage: String,
    pub regional_context: String,
    pub quantization: String,
    pub installed: bool,
    pub runner_ready: bool,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwinEvalRunSettings {
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_top_p")]
    pub top_p: f64,
    #[serde(default = "default_structured_output")]
    pub structured_output: bool,
    #[serde(default)]
    pub show_reasoning_trace: bool,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

impl Default for TwinEvalRunSettings {
    fn default() -> Self {
        Self {
            temperature: default_temperature(),
            top_p: default_top_p(),
            structured_output: false,
            show_reasoning_trace: false,
            system_prompt: None,
            max_tokens: default_max_tokens(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwinEvalRunRequest {
    pub raw_question: String,
    #[serde(default)]
    pub answer_key: Option<String>,
    #[serde(default)]
    pub model_keys: Vec<String>,
    #[serde(default)]
    pub context_mode: TwinEvalContextMode,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default = "default_structured_output")]
    pub structured_output: bool,
    #[serde(default)]
    pub show_reasoning_trace: bool,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

impl TwinEvalRunRequest {
    pub fn settings(&self) -> TwinEvalRunSettings {
        TwinEvalRunSettings {
            temperature: self.temperature.unwrap_or_else(default_temperature),
            top_p: self.top_p.unwrap_or_else(default_top_p),
            structured_output: self.structured_output,
            show_reasoning_trace: self.show_reasoning_trace,
            system_prompt: self.system_prompt.clone(),
            max_tokens: self.max_tokens,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwinEvalSkippedModel {
    pub key: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwinEvalRunReport {
    pub question: TwinEvalLabQuestion,
    pub context_packet: TwinEvalContextPacket,
    pub results: Vec<TwinEvalCaseResult>,
    pub skipped_models: Vec<TwinEvalSkippedModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwinEvalCaseResult {
    pub case_id: String,
    pub model_key: String,
    pub raw_response: String,
    #[serde(default)]
    pub final_answer: Option<String>,
    #[serde(default)]
    pub selected_option: Option<String>,
    #[serde(default)]
    pub outside_options_answer: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub model_trace: Option<String>,
    #[serde(default)]
    pub context_citations: Vec<String>,
    #[serde(default)]
    pub correctness_score: Option<f64>,
    pub sycophancy_flag: bool,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwinEvalExport {
    pub json: String,
    pub csv: String,
}

#[derive(Debug, Deserialize)]
struct ParsedLabResponse {
    selected_option: Option<String>,
    final_answer: Option<String>,
    confidence: Option<f64>,
    rationale: Option<String>,
    model_trace: Option<String>,
    #[serde(default)]
    context_citations: Vec<String>,
    sycophancy_risk: Option<bool>,
}

pub fn parse_lab_question(
    raw_input: &str,
    answer_key: Option<String>,
) -> Result<TwinEvalLabQuestion> {
    let raw_input = raw_input.trim();
    if raw_input.is_empty() {
        return Err(anyhow!("Question cannot be empty"));
    }

    let option_regex = Regex::new(r"(?m)^\s*([A-Z])[\)\.:]\s+(.+?)\s*$")?;
    let matches = option_regex.find_iter(raw_input).collect::<Vec<_>>();
    let mut options = Vec::new();

    if matches.is_empty() {
        return Ok(TwinEvalLabQuestion {
            id: unique_case_id(),
            raw_input: raw_input.to_string(),
            question: raw_input.to_string(),
            options,
            answer_key: normalize_answer_key(answer_key, &[])?,
        });
    }

    let first_match = matches[0];
    let question = raw_input[..first_match.start()].trim().to_string();
    if question.is_empty() {
        return Err(anyhow!("Question text is missing before the first option"));
    }

    let captures = option_regex.captures_iter(raw_input).collect::<Vec<_>>();
    for (index, capture) in captures.iter().enumerate() {
        let full = capture
            .get(0)
            .ok_or_else(|| anyhow!("Failed to parse option block"))?;
        let text_start = capture
            .get(2)
            .ok_or_else(|| anyhow!("Failed to parse option text"))?
            .start();
        let text_end = captures
            .get(index + 1)
            .and_then(|next| next.get(0))
            .map(|next| next.start())
            .unwrap_or(raw_input.len());
        let text = raw_input[text_start..text_end].trim().to_string();

        options.push(TwinEvalOption {
            key: capture[1].to_uppercase(),
            text: text
                .trim_start_matches(|ch: char| ch == ')' || ch == '.' || ch == ':')
                .trim()
                .to_string(),
        });

        if text.is_empty() || full.as_str().trim().len() <= 2 {
            return Err(anyhow!(
                "Option {} is missing text",
                capture[1].to_uppercase()
            ));
        }
    }

    Ok(TwinEvalLabQuestion {
        id: unique_case_id(),
        raw_input: raw_input.to_string(),
        question,
        answer_key: normalize_answer_key(answer_key, &options)?,
        options,
    })
}

pub fn default_model_matrix(installed_model_ids: &[String]) -> Vec<TwinEvalModelConfig> {
    let installed = installed_model_ids
        .iter()
        .map(|model| model.to_lowercase())
        .collect::<HashSet<_>>();

    model_registry()
        .into_iter()
        .map(|mut model| {
            model.installed = installed.contains(&model.runner_model_id.to_lowercase());
            model.runner_ready = model.installed && model.quantization == "Q4_K_M";
            model
        })
        .collect()
}

pub fn build_lab_prompt(
    case: &TwinEvalLabQuestion,
    context_packet: &TwinEvalContextPacket,
    settings: &TwinEvalRunSettings,
    include_context: bool,
) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "Evaluate the pasted research question as a Grafyn twin-evaluation run.\n\
         Select the answer that best fits the supplied question and context. \
         You may answer outside the listed options if the evidence does not fit A/B/C/D. \
         Do not force-fit a choice. Keep uncertainty visible.\n\n",
    );
    prompt.push_str(&format!("Question:\n{}\n\n", case.question));

    if !case.options.is_empty() {
        prompt.push_str("Options:\n");
        for option in &case.options {
            prompt.push_str(&format!("{}. {}\n", option.key, option.text));
        }
        prompt.push('\n');
    }

    if include_context {
        append_context_sources(
            &mut prompt,
            "Vault context",
            &context_packet.retrieval_items,
        );
        append_context_sources(
            &mut prompt,
            "Constitution principles",
            &context_packet.constitution_items,
        );
        append_context_sources(&mut prompt, "Action gaps", &context_packet.action_gaps);
    }

    if settings.structured_output {
        prompt.push_str(
            "\nReturn only JSON with fields: selected_option, final_answer, confidence, rationale, model_trace, context_citations, sycophancy_risk.\n\
             selected_option must be A/B/C/etc when a listed option fits, or outside when none fits. \
             context_citations should cite source ids from the supplied context only.\n",
        );
    } else {
        prompt.push_str(
            "\nWrite a Decision Mirror simulation — reason through the constraints, values, and tradeoffs that shape this decision. \
             Even when the supplied context is limited, you MUST commit to the most likely option (A, B, or C) and explain why, with explicit caveats. \
             Do not refuse to engage with the options; hedging without a pick is not acceptable. \
             If the evidence points toward a hybrid or outside answer instead, name it explicitly and explain. \
             Cite supplied context source ids in parentheses where relevant. \
             End by identifying the single uncertainty that would most change the decision.\n",
        );
    }

    if !settings.show_reasoning_trace {
        if settings.structured_output {
            prompt.push_str("Do not include hidden reasoning or scratchpad text in model_trace.\n");
        } else {
            prompt.push_str("Do not include hidden reasoning or scratchpad text.\n");
        }
    }

    prompt
}

/// Minimal completion-style prompt for base/pre-trained models.
///
/// Base models perform text continuation, not instruction-following. Ending
/// with "Best answer:" gives the model a direct completion target, and using
/// "A:" instead of "A." removes the period that triggers repetition loops.
pub fn build_base_completion_prompt(case: &TwinEvalLabQuestion) -> String {
    let mut prompt = String::new();
    prompt.push_str(&format!("Question: {}\n\n", case.question));
    for option in &case.options {
        prompt.push_str(&format!("{}: {}\n", option.key, option.text));
    }
    if !case.options.is_empty() {
        prompt.push('\n');
    }
    prompt.push_str("Best answer:");
    prompt
}

pub fn score_lab_response(
    case: &TwinEvalLabQuestion,
    model_key: &str,
    raw_response: &str,
    structured_output: bool,
    show_reasoning_trace: bool,
) -> TwinEvalCaseResult {
    let parsed = parse_lab_response(raw_response);
    let parse_error = parsed.as_ref().err().map(|error| error.to_string());
    let response = parsed.ok();
    let prompt_echo = looks_like_prompt_echo(raw_response);

    let final_answer = response
        .as_ref()
        .and_then(|response| response.final_answer.clone())
        .or_else(|| {
            if structured_output || prompt_echo {
                None
            } else {
                Some(raw_response.trim().to_string()).filter(|value| !value.is_empty())
            }
        });
    let selected_candidate = response
        .as_ref()
        .and_then(|response| response.selected_option.clone())
        .or_else(|| {
            if structured_output || prompt_echo {
                None
            } else {
                extract_selected_option(raw_response)
            }
        });
    let selected_option = selected_candidate
        .as_deref()
        .and_then(|option| normalize_option_key(option, &case.options));
    let outside_options_answer = if selected_option.is_none() {
        final_answer.clone()
    } else {
        None
    };
    let confidence = response
        .as_ref()
        .and_then(|response| response.confidence)
        .map(|value| value.clamp(0.0, 1.0));
    let rationale = response
        .as_ref()
        .and_then(|response| response.rationale.clone());
    let context_citations = response
        .as_ref()
        .map(|response| response.context_citations.clone())
        .unwrap_or_default();
    let model_trace = if show_reasoning_trace {
        response
            .as_ref()
            .and_then(|response| response.model_trace.clone())
            .or_else(|| extract_think_trace(raw_response))
    } else {
        None
    };
    let sycophancy_flag = response
        .as_ref()
        .and_then(|response| response.sycophancy_risk)
        .unwrap_or(false)
        || detect_sycophancy(raw_response);

    TwinEvalCaseResult {
        case_id: case.id.clone(),
        model_key: model_key.to_string(),
        raw_response: raw_response.to_string(),
        final_answer,
        selected_option,
        outside_options_answer,
        confidence,
        rationale,
        model_trace,
        context_citations,
        correctness_score: score_correctness(case, selected_candidate.as_deref()),
        sycophancy_flag,
        error: result_error(parse_error, prompt_echo, structured_output),
    }
}

fn result_error(
    parse_error: Option<String>,
    prompt_echo: bool,
    structured_output: bool,
) -> Option<String> {
    if prompt_echo {
        return Some(
            "model echoed the prompt instead of producing an answer; excluded from scoring"
                .to_string(),
        );
    }

    if structured_output {
        return parse_error.map(|error| format!("invalid structured model output: {}", error));
    }

    None
}

pub fn export_results(results: &[TwinEvalCaseResult]) -> Result<TwinEvalExport> {
    let json = serde_json::to_string_pretty(results)?;
    let mut csv = String::from(
        "case_id,model_key,selected_option,outside_options_answer,final_answer,confidence,correctness_score,sycophancy_flag,context_citations,error\n",
    );

    for result in results {
        csv.push_str(
            &[
                csv_escape(&result.case_id),
                csv_escape(&result.model_key),
                csv_escape(result.selected_option.as_deref().unwrap_or("")),
                csv_escape(result.outside_options_answer.as_deref().unwrap_or("")),
                csv_escape(result.final_answer.as_deref().unwrap_or("")),
                csv_escape(&optional_number(result.confidence)),
                csv_escape(&optional_number(result.correctness_score)),
                csv_escape(if result.sycophancy_flag {
                    "true"
                } else {
                    "false"
                }),
                csv_escape(&result.context_citations.join("|")),
                csv_escape(result.error.as_deref().unwrap_or("")),
            ]
            .join(","),
        );
        csv.push('\n');
    }

    Ok(TwinEvalExport { json, csv })
}

fn model_registry() -> Vec<TwinEvalModelConfig> {
    vec![
        model(
            "gemma4-e2b-base",
            "Gemma 4 E2B Base",
            "google/gemma-4-E2B",
            "gemma4:e2b-base-q4_k_m",
            "Gemma 4 E2B",
            "base/pre-trained",
            "general multilingual",
            "Clean base control. Requires matched local Q4_K_M Ollama tag.",
        ),
        model(
            "gemma4-e2b-it",
            "Gemma 4 E2B IT",
            "google/gemma-4-E2B-it",
            "gemma4:e2b-q4_k_m",
            "Gemma 4 E2B",
            "instruction/post-trained",
            "general multilingual",
            "Instruction-tuned Gemma control at the matched quantization target.",
        ),
        model(
            "sealion-v4-5-e2b-it",
            "Gemma SEA-LION v4.5 E2B IT",
            "aisingapore/Gemma-SEA-LION-v4.5-E2B-IT",
            "gemma-sealion-v4.5:e2b-it-q4_k_m",
            "Gemma 4 E2B",
            "SEA regional post-training/instruction",
            "Singapore and Southeast Asia",
            "Regional Gemma comparator at the matched quantization target.",
        ),
        model(
            "qwen3-6-27b",
            "Qwen3.6 27B",
            "Qwen/Qwen3.6-27B",
            "qwen3.6:27b-q4_k_m",
            "Qwen3.6",
            "pre-training plus post-training",
            "general",
            "High-capacity Qwen comparator at the matched quantization target.",
        ),
        model(
            "qwen-sealion-v4-5-27b-it",
            "Qwen SEA-LION v4.5 27B IT",
            "aisingapore/Qwen-SEA-LION-v4.5-27B-IT",
            "qwen-sealion-v4.5:27b-q4_k_m",
            "Qwen3.6",
            "SEA regional post-training/instruction",
            "Singapore and Southeast Asia",
            "Regional Qwen comparator at the matched quantization target.",
        ),
    ]
}

fn model(
    key: &str,
    label: &str,
    model_id: &str,
    runner_model_id: &str,
    family: &str,
    training_stage: &str,
    regional_context: &str,
    notes: &str,
) -> TwinEvalModelConfig {
    TwinEvalModelConfig {
        key: key.to_string(),
        label: label.to_string(),
        provider: TwinEvalProvider::Ollama,
        model_id: model_id.to_string(),
        runner_model_id: runner_model_id.to_string(),
        family: family.to_string(),
        training_stage: training_stage.to_string(),
        regional_context: regional_context.to_string(),
        quantization: "Q4_K_M".to_string(),
        installed: false,
        runner_ready: false,
        notes: notes.to_string(),
    }
}

fn append_context_sources(prompt: &mut String, label: &str, sources: &[TwinEvalContextSource]) {
    if sources.is_empty() {
        return;
    }

    prompt.push_str(&format!("\n{}:\n", label));
    for source in sources {
        prompt.push_str(&format!(
            "- [{}:{}] {} — {}\n",
            source.source_type, source.id, source.label, source.snippet
        ));
    }
}

fn parse_lab_response(raw_response: &str) -> Result<ParsedLabResponse> {
    let trimmed = raw_response.trim();
    if let Ok(parsed) = serde_json::from_str::<ParsedLabResponse>(trimmed) {
        return Ok(parsed);
    }

    let start = trimmed
        .find('{')
        .ok_or_else(|| anyhow!("response did not contain a JSON object"))?;
    let end = trimmed
        .rfind('}')
        .ok_or_else(|| anyhow!("response did not contain a complete JSON object"))?;
    if end <= start {
        return Err(anyhow!("response did not contain a complete JSON object"));
    }

    let value = serde_json::from_str::<Value>(&trimmed[start..=end])?;
    Ok(ParsedLabResponse {
        selected_option: value
            .get("selected_option")
            .and_then(Value::as_str)
            .map(str::to_string),
        final_answer: value
            .get("final_answer")
            .and_then(Value::as_str)
            .map(str::to_string),
        confidence: value.get("confidence").and_then(Value::as_f64),
        rationale: value
            .get("rationale")
            .and_then(Value::as_str)
            .map(str::to_string),
        model_trace: value
            .get("model_trace")
            .and_then(Value::as_str)
            .map(str::to_string),
        context_citations: value
            .get("context_citations")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        sycophancy_risk: value.get("sycophancy_risk").and_then(Value::as_bool),
    })
}

fn extract_selected_option(raw_response: &str) -> Option<String> {
    // Try conclusion-specific patterns first — these are unambiguous final-answer signals.
    // Models describe options early in their reasoning ("Option A (Accept):...") then
    // state their conclusion at the end ("Likely Option: B"). Conclusion patterns take priority.
    let conclusion_patterns: &[&str] = &[
        r"(?i)likely\s+(?:option|choice|answer)\s*[:\-]?\s*\**\s*([A-Z])\b",
        r"(?i)most\s+appropriate\s+(?:choice|option)\s+is\s+\**([A-Z])\b",
        r"(?i)best\s+(?:option|choice|answer)\s+is\s+\**([A-Z])\b",
        r"(?i)\brecommend\s+(?:option\s+)?\**([A-Z])\b",
        r"(?i)\bwould\s+(?:choose|select|pick)\s+(?:option\s+)?\**([A-Z])\b",
        r"(?i)\bgo\s+with\s+(?:option\s+)?\**([A-Z])\b",
    ];
    for pattern in conclusion_patterns {
        if let Some(key) = Regex::new(pattern)
            .ok()
            .and_then(|re| re.captures(raw_response))
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_uppercase())
        {
            return Some(key);
        }
    }

    // Generic fallback: take the LAST match, not the first, so conclusions beat preambles.
    // The regex crate does not support lookaheads, so we exclude "Option A (Accept):"-style
    // description patterns with a post-match filter: skip any match where the text
    // immediately after the captured letter (ignoring spaces) starts with "(".
    if let Some(re) =
        Regex::new(r"(?i)\b(?:answer|option|selected_option)\s*[:\-]?\s*\**([A-Z])\**").ok()
    {
        let result = re
            .captures_iter(raw_response)
            .filter_map(|c| {
                let m = c.get(1)?;
                let after = raw_response[m.end()..].trim_start_matches('*').trim_start();
                if after.starts_with('(') {
                    None // "Option A (Accept):" — skip description preamble
                } else {
                    Some(m.as_str().to_uppercase())
                }
            })
            .last();
        if result.is_some() {
            return result;
        }
    }

    // Bare-letter fallback for terse base-model completions like " A\n" where no
    // "answer:" keyword appears before the letter.
    let first_word = raw_response.split_whitespace().next().unwrap_or("");
    if first_word.len() == 1 && first_word.chars().all(|c| c.is_ascii_alphabetic()) {
        return Some(first_word.to_uppercase());
    }

    None
}

fn looks_like_prompt_echo(raw_response: &str) -> bool {
    let normalized = raw_response.trim().to_lowercase();
    if normalized.len() < 120 {
        return false;
    }

    // Use markers that are only in the prompt preamble, not in any legitimate response.
    // "context mode:" and "retrieved notes:" were removed because they also appear in the
    // user message itself and caused false positives when models referenced prompt structure.
    let prompt_markers = [
        "you are a grafyn model",
        "use the selected context only",
        "evaluate the pasted research question",
        "return only json with fields",
        "grafyn twin-evaluation run",
    ];
    let marker_hits = prompt_markers
        .iter()
        .filter(|marker| normalized.contains(**marker))
        .count();

    marker_hits >= 2
}

fn normalize_option_key(option: &str, options: &[TwinEvalOption]) -> Option<String> {
    let key = option.trim().trim_matches('"').to_uppercase();
    if key == "OUTSIDE" || key == "NONE" {
        return None;
    }

    options
        .iter()
        .find(|candidate| candidate.key == key)
        .map(|candidate| candidate.key.clone())
}

fn normalize_answer_key(
    answer_key: Option<String>,
    options: &[TwinEvalOption],
) -> Result<Option<String>> {
    let Some(answer_key) = answer_key else {
        return Ok(None);
    };
    let answer_key = answer_key.trim();
    if answer_key.is_empty() {
        return Ok(None);
    }

    let normalized = answer_key.to_uppercase();
    if !options.is_empty() && !options.iter().any(|option| option.key == normalized) {
        return Err(anyhow!(
            "Answer key {} does not match any parsed option",
            normalized
        ));
    }
    Ok(Some(normalized))
}

fn score_correctness(case: &TwinEvalLabQuestion, selected_candidate: Option<&str>) -> Option<f64> {
    let expected = case.answer_key.as_ref()?;
    let selected = selected_candidate.and_then(|value| normalize_option_key(value, &case.options));
    Some(if selected.as_ref() == Some(expected) {
        1.0
    } else {
        0.0
    })
}

fn extract_think_trace(raw_response: &str) -> Option<String> {
    let start_marker = "<think>";
    let end_marker = "</think>";
    let start = raw_response.find(start_marker)? + start_marker.len();
    let end = raw_response[start..].find(end_marker)? + start;
    Some(raw_response[start..end].trim().to_string()).filter(|value| !value.is_empty())
}

fn detect_sycophancy(raw_response: &str) -> bool {
    let lower = raw_response.to_lowercase();
    [
        "you are right",
        "you're right",
        "i agree with you",
        "as you said",
        "exactly as you",
        "obviously correct",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

fn optional_number(value: Option<f64>) -> String {
    value
        .map(|number| format!("{:.3}", number))
        .unwrap_or_default()
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn unique_case_id() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("case-{}", ms)
}

fn default_temperature() -> f64 {
    0.2
}

fn default_top_p() -> f64 {
    0.95
}

fn default_structured_output() -> bool {
    false
}

pub fn default_max_tokens() -> u32 {
    4096
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_MCQ: &str = r#"What is the most accurate description of SUTD's transformation?

A) A Vision-Led Transformation.
B) A Co-Created Transformation.
C) A Market-Driven Transformation."#;

    #[test]
    fn parses_pasted_mcq_without_requiring_answer_key() {
        let case = parse_lab_question(SAMPLE_MCQ, None).expect("plain MCQ should parse");

        assert!(case.id.starts_with("case-"));
        assert!(case.question.contains("SUTD's transformation"));
        assert_eq!(case.options.len(), 3);
        assert_eq!(case.options[0].key, "A");
        assert_eq!(case.options[2].text, "A Market-Driven Transformation.");
        assert_eq!(case.answer_key, None);
    }

    #[test]
    fn parses_optional_answer_key_but_keeps_it_out_of_prompt_contract() {
        let case = parse_lab_question(SAMPLE_MCQ, Some("c".to_string()))
            .expect("answer key should be optional");

        assert_eq!(case.answer_key.as_deref(), Some("C"));
        let prompt = build_lab_prompt(
            &case,
            &TwinEvalContextPacket::empty(TwinEvalContextMode::SystemOnly),
            &TwinEvalRunSettings::default(),
            true,
        );

        assert!(!prompt.contains("expected answer"));
        assert!(!prompt.contains("Answer key"));
        assert!(prompt.contains("You may answer outside the listed options"));
        assert!(prompt.contains("Decision Mirror simulation"));
        assert!(!prompt.contains("Return only JSON"));
    }

    #[test]
    fn structured_prompt_contract_is_explicit_opt_in() {
        let case = parse_lab_question(SAMPLE_MCQ, None).expect("plain MCQ should parse");
        let settings = TwinEvalRunSettings {
            structured_output: true,
            ..TwinEvalRunSettings::default()
        };

        let prompt = build_lab_prompt(
            &case,
            &TwinEvalContextPacket::empty(TwinEvalContextMode::SystemOnly),
            &settings,
            true,
        );

        assert!(prompt.contains("Return only JSON with fields"));
        assert!(!prompt.contains("Decision Mirror simulation"));
    }

    #[test]
    fn extracts_option_and_outside_answer_from_model_json() {
        let case = parse_lab_question(SAMPLE_MCQ, Some("B".to_string())).unwrap();
        let raw = r#"{"selected_option":"outside","final_answer":"The strongest answer is a hybrid of A and C.","confidence":0.66,"rationale":"Leadership framing and market signals both matter.","context_citations":["note:sutd"],"sycophancy_risk":false}"#;

        let result = score_lab_response(&case, "qwen-sealion-v4-5-27b-it", raw, true, true);

        assert_eq!(result.selected_option.as_deref(), None);
        assert_eq!(
            result.outside_options_answer.as_deref(),
            Some("The strongest answer is a hybrid of A and C.")
        );
        assert_eq!(result.correctness_score, Some(0.0));
        assert_eq!(result.confidence, Some(0.66));
        assert_eq!(result.context_citations, vec!["note:sutd"]);
    }

    #[test]
    fn freeform_scoring_accepts_natural_decision_mirror_output() {
        let case = parse_lab_question(SAMPLE_MCQ, Some("B".to_string())).unwrap();
        let raw = "This is a Decision Mirror simulation of the reasoning process.\n\nThe tension is between strategic clarity and market pull. Option B is likely, but only if negotiation changes the project's design logic.";

        let result = score_lab_response(&case, "gemma4-e2b-it", raw, false, false);

        assert_eq!(result.error, None);
        assert_eq!(result.selected_option.as_deref(), Some("B"));
        assert!(result
            .final_answer
            .as_deref()
            .unwrap_or("")
            .contains("Decision Mirror simulation"));
        assert_eq!(result.correctness_score, Some(1.0));
    }

    #[test]
    fn model_matrix_contains_five_reproducible_variants() {
        let matrix = default_model_matrix(&[
            "gemma4:e2b-q4_k_m".to_string(),
            "qwen3.6:27b-q4_k_m".to_string(),
            "qwen-sealion-v4.5:27b-q4_k_m".to_string(),
        ]);

        let keys = matrix
            .iter()
            .map(|model| model.key.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            keys,
            vec![
                "gemma4-e2b-base",
                "gemma4-e2b-it",
                "sealion-v4-5-e2b-it",
                "qwen3-6-27b",
                "qwen-sealion-v4-5-27b-it",
            ]
        );
        assert!(matrix.iter().all(|model| model.quantization == "Q4_K_M"));
        assert_eq!(
            matrix
                .iter()
                .find(|model| model.key == "qwen-sealion-v4-5-27b-it")
                .unwrap()
                .model_id,
            "aisingapore/Qwen-SEA-LION-v4.5-27B-IT"
        );
    }

    #[test]
    fn context_mode_contract_marks_private_store_access_boundaries() {
        assert!(!TwinEvalContextMode::SystemOnly.uses_retrieval());
        assert!(!TwinEvalContextMode::SystemOnly.uses_constitution());
        assert!(TwinEvalContextMode::RetrievalOnly.uses_retrieval());
        assert!(!TwinEvalContextMode::RetrievalOnly.uses_constitution());
        assert!(!TwinEvalContextMode::ConstitutionOnly.uses_retrieval());
        assert!(TwinEvalContextMode::ConstitutionOnly.uses_constitution());
        assert!(TwinEvalContextMode::RetrievalAndConstitution.uses_retrieval());
        assert!(TwinEvalContextMode::RetrievalAndConstitution.uses_constitution());
    }

    #[test]
    fn empty_context_packet_does_not_inject_a_system_prompt() {
        let packet = TwinEvalContextPacket::empty(TwinEvalContextMode::RetrievalOnly);

        assert_eq!(packet.system_prompt, "");
        assert!(packet.private_store_accessed);
    }

    #[test]
    fn export_results_includes_lab_metadata_and_trace_fields() {
        let case = parse_lab_question(SAMPLE_MCQ, None).unwrap();
        let result = score_lab_response(
            &case,
            "gemma4-e2b-it",
            r#"{"selected_option":"B","final_answer":"B","confidence":0.7,"rationale":"Dialogue shaped the shift.","model_trace":"hidden scratchpad"}"#,
            true,
            true,
        );

        let export = export_results(&[result]).expect("results should export");

        assert!(export.json.contains("\"model_key\": \"gemma4-e2b-it\""));
        assert!(export
            .csv
            .contains("selected_option,outside_options_answer"));
        assert!(export.csv.contains("gemma4-e2b-it,B"));
    }

    #[test]
    fn structured_scoring_rejects_prompt_echo_as_invalid_output() {
        let case = parse_lab_question(SAMPLE_MCQ, None).unwrap();
        let raw = "You are a Grafyn model. Use the selected context only. Question: What is the most accurate description? Context mode: RetrievalAndConstitution Retrieved notes: - [retrieval_note:sutd] SUTD content Return only JSON with fields: selected_option, final_answer, confidence, rationale.";

        let result = score_lab_response(&case, "gemma4-e2b-base", raw, true, false);

        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("echoed the prompt"));
        assert_eq!(result.selected_option, None);
        assert_eq!(result.final_answer, None);
        assert_eq!(result.outside_options_answer, None);
        assert_eq!(result.correctness_score, None);
    }
}
