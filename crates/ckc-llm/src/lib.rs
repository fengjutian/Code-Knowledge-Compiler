//! LLM Compiler — enrich Knowledge IR with semantic understanding.
//!
//! The [`LlmProvider`] trait abstracts LLM backends (OpenAI, Ollama, etc.).
//! [`SemanticCompiler`] orchestrates batch code analysis: it sends function
//! signatures + docstrings to an LLM and parses structured semantic metadata
//! (purpose, summary, responsibility, business capability, design patterns).
//!
//! # Architecture
//!
//! ```text
//! IrNode → Prompt Template → LLM API → JSON Response → SemanticInfo
//! ```

use ckc_ir::{IrNode, RiskSeverity, RiskTag, SemanticInfo};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;

// ── Error ──────────────────────────────────────────────────────────────────

#[derive(Error, Debug)]
pub enum LlmError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("LLM returned empty response")]
    EmptyResponse,
    #[error("LLM error ({status}): {body}")]
    ApiError { status: u16, body: String },
    #[error("Configuration error: {0}")]
    Config(String),
}

// ── LLM Provider Trait ─────────────────────────────────────────────────────

/// Abstract LLM backend.
pub trait LlmProvider: Send + Sync {
    /// Send a chat completion request and return the response text.
    fn chat(
        &self,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<String, LlmError>;

    /// Model name for metadata tracking.
    fn model_name(&self) -> &str;
    fn model_version(&self) -> &str;
}

// ── OpenAI-compatible Provider ─────────────────────────────────────────────

/// Works with OpenAI, Ollama, and any OpenAI-compatible API.
pub struct OpenAiProvider {
    client: reqwest::blocking::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
}

impl OpenAiProvider {
    /// Create an OpenAI provider.
    ///
    /// # Environment
    /// - `OPENAI_API_KEY` — API key (not needed for Ollama)
    /// - `OPENAI_BASE_URL` — override base URL (default: `https://api.openai.com/v1`)
    pub fn from_env() -> Result<Self, LlmError> {
        let base_url = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".into());
        let api_key = std::env::var("OPENAI_API_KEY").ok();
        let model = std::env::var("CKC_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());

        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()?,
            base_url,
            api_key,
            model,
        })
    }

    /// Create a provider for local Ollama.
    pub fn ollama(model: &str) -> Result<Self, LlmError> {
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()?,
            base_url: "http://localhost:11434/v1".into(),
            api_key: None,
            model: model.to_string(),
        })
    }
}

impl LlmProvider for OpenAiProvider {
    fn chat(&self, system_prompt: &str, user_message: &str) -> Result<String, LlmError> {
        let url = format!("{}/chat/completions", self.base_url);

        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_message}
            ],
            "temperature": 0.2,
            "max_tokens": 1024,
        });

        let req = self.client.post(&url).json(&body);
        let req = if let Some(ref key) = self.api_key {
            req.header("Authorization", format!("Bearer {}", key))
        } else {
            req
        };

        let resp = req.send()?;
        let status = resp.status();

        if !status.is_success() {
            let body_text = resp.text().unwrap_or_default();
            return Err(LlmError::ApiError {
                status: status.as_u16(),
                body: body_text,
            });
        }

        let json: serde_json::Value = resp.json()?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or(LlmError::EmptyResponse)?
            .to_string();

        Ok(content)
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn model_version(&self) -> &str {
        "1.0"
    }
}

// ── No-op Provider (for testing / offline mode) ────────────────────────────

/// A provider that returns empty results — used when LLM is not configured.
pub struct NoopProvider;

impl LlmProvider for NoopProvider {
    fn chat(&self, _system: &str, _user: &str) -> Result<String, LlmError> {
        Ok("{}".into())
    }
    fn model_name(&self) -> &str {
        "noop"
    }
    fn model_version(&self) -> &str {
        "0"
    }
}

// ── Prompt Templates ───────────────────────────────────────────────────────

const SYSTEM_PROMPT: &str = r#"You are a code analysis expert. Given a function or class definition, output a JSON object with these fields:
- "purpose": one-sentence description of what this code does
- "summary": 2-3 sentence summary of its behavior
- "responsibility": array of 1-3 responsibility tags (e.g., ["validation", "data-access", "orchestration"])
- "business_capability": array of 1-2 business domain tags (e.g., ["payment", "authentication", "user-management"])
- "design_pattern": array of 0-2 design patterns if detected (e.g., ["Singleton", "Factory", "Strategy", "Observer"])
- "risks": array of risk objects with "severity" (low/medium/high/critical), "category", "description"

Output ONLY valid JSON, no markdown, no explanation."#;

fn build_user_prompt(node: &IrNode, source_snippet: Option<&str>) -> String {
    let mut prompt = String::new();

    // Include decorators if present
    if let Some(decorators) = node.metadata.get("decorators") {
        prompt.push_str(&format!("Decorators: {}\n", decorators));
    }
    if let Some(d) = node.metadata.get("decorator").and_then(|v| v.as_str()) {
        prompt.push_str(&format!("Decorator: @{}\n", d));
    }
    if node.metadata.get("async").and_then(|v| v.as_bool()).unwrap_or(false) {
        prompt.push_str("Async: true\n");
    }

    // Kind + name
    let kind_str = crate::node_kind_label(node.kind);
    prompt.push_str(&format!("{}: {}\n", kind_str, node.name));

    // Parameters
    if let Some(params) = node.metadata.get("parameters").and_then(|v| v.as_array()) {
        let param_strs: Vec<String> = params
            .iter()
            .map(|p| {
                let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let typ = p.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if typ.is_empty() {
                    name.to_string()
                } else {
                    format!("{}: {}", name, typ)
                }
            })
            .collect();
        prompt.push_str(&format!("Parameters: {}\n", param_strs.join(", ")));
    }

    // Return type
    if let Some(rt) = node.metadata.get("return_type").and_then(|v| v.as_str()) {
        prompt.push_str(&format!("Returns: {}\n", rt));
    }

    // Existing purpose from docstring
    if let Some(sem) = &node.semantic {
        if let Some(purpose) = &sem.purpose {
            prompt.push_str(&format!("Docstring: {}\n", purpose));
        }
    }

    // File location
    prompt.push_str(&format!("File: {}:{}\n", node.id.file_path, node.location.line_start));

    // Source snippet (optional, for better context)
    if let Some(snippet) = source_snippet {
        let truncated = if snippet.len() > 2000 {
            format!("{}...", &snippet[..2000])
        } else {
            snippet.to_string()
        };
        prompt.push_str(&format!("\nCode:\n```\n{}\n```", truncated));
    }

    prompt
}

fn node_kind_label(kind: ckc_ir::NodeKind) -> &'static str {
    match kind {
        ckc_ir::NodeKind::Function => "Function",
        ckc_ir::NodeKind::Method => "Method",
        ckc_ir::NodeKind::Class => "Class",
        _ => "Symbol",
    }
}

// ── Semantic Compiler ──────────────────────────────────────────────────────

/// Orchestrates LLM-based semantic enrichment with caching.
pub struct SemanticCompiler<P: LlmProvider> {
    provider: P,
    batch_size: usize,
    skip_enriched: bool,
    /// In-memory cache: (name, signature_hash) → SemanticInfo
    cache: HashMap<(String, u64), SemanticInfo>,
}

impl<P: LlmProvider> SemanticCompiler<P> {
    pub fn new(provider: P) -> Self {
        Self {
            provider,
            batch_size: 5,
            skip_enriched: true,
            cache: HashMap::new(),
        }
    }

    /// Enrich a batch of nodes, using cache to avoid repeated LLM calls.
    pub fn enrich_batch(
        &mut self,
        nodes: &mut [IrNode],
        source_snippets: &HashMap<String, String>,
    ) -> Result<usize, LlmError> {
        let candidates: Vec<usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                matches!(n.kind, ckc_ir::NodeKind::Function | ckc_ir::NodeKind::Method | ckc_ir::NodeKind::Class)
                    && (!self.skip_enriched || n.semantic.is_none() || n.semantic.as_ref().map_or(true, |s| s.summary.is_none()))
            })
            .map(|(i, _)| i)
            .collect();

        if candidates.is_empty() {
            return Ok(0);
        }

        let mut enriched = 0;

        for chunk in candidates.chunks(self.batch_size) {
            for &idx in chunk {
                let node = &nodes[idx];

                // Check cache first
                let cache_key = (node.name.clone(), node.id.signature_hash);
                if let Some(cached) = self.cache.get(&cache_key) {
                    nodes[idx].semantic = Some(cached.clone());
                    enriched += 1;
                    continue;
                }

                let file_key = &node.id.file_path;
                let snippet = source_snippets.get(file_key).map(|s| s.as_str());
                let user_prompt = build_user_prompt(node, snippet);

                match self.provider.chat(SYSTEM_PROMPT, &user_prompt) {
                    Ok(response) => {
                        if let Ok(info) = parse_semantic_response(&response) {
                            self.cache.insert(cache_key, info.clone());
                            nodes[idx].semantic = Some(info);
                            enriched += 1;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "LLM enrichment failed for {}::{}: {}",
                            node.id.file_path,
                            node.name,
                            e
                        );
                    }
                }
            }
        }

        Ok(enriched)
    }

    /// Number of cached entries.
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }
}

// ── Response Parsing ───────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct LlmSemanticResponse {
    purpose: Option<String>,
    summary: Option<String>,
    #[serde(default)]
    responsibility: Vec<String>,
    #[serde(default)]
    business_capability: Vec<String>,
    #[serde(default)]
    design_pattern: Vec<String>,
    #[serde(default)]
    risks: Vec<LlmRiskResponse>,
}

#[derive(Deserialize)]
struct LlmRiskResponse {
    severity: Option<String>,
    category: Option<String>,
    description: Option<String>,
}

fn parse_semantic_response(json_str: &str) -> Result<SemanticInfo, LlmError> {
    // Extract JSON from markdown code blocks if present
    let cleaned = json_str
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let parsed: LlmSemanticResponse = serde_json::from_str(cleaned)?;

    let risks: Vec<RiskTag> = parsed
        .risks
        .into_iter()
        .map(|r| RiskTag {
            severity: match r.severity.as_deref() {
                Some("low") => RiskSeverity::Low,
                Some("medium") => RiskSeverity::Medium,
                Some("high") => RiskSeverity::High,
                Some("critical") => RiskSeverity::Critical,
                _ => RiskSeverity::Low,
            },
            category: r.category.unwrap_or_default(),
            description: r.description.unwrap_or_default(),
        })
        .collect();

    Ok(SemanticInfo {
        purpose: parsed.purpose,
        summary: parsed.summary,
        responsibility: parsed.responsibility,
        business_capability: parsed.business_capability,
        design_pattern: parsed.design_pattern,
        complexity: None, // LLM doesn't compute metrics
        risks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_llm_response_json() {
        let response = r#"{
            "purpose": "Validates user input",
            "summary": "Checks that the input fields are non-empty and the email is valid.",
            "responsibility": ["validation", "input-processing"],
            "business_capability": ["user-management"],
            "design_pattern": ["Strategy"],
            "risks": [
                {"severity": "medium", "category": "InputValidation", "description": "No XSS check"}
            ]
        }"#;

        let info = parse_semantic_response(response).unwrap();
        assert_eq!(info.purpose.unwrap(), "Validates user input");
        assert_eq!(info.responsibility.len(), 2);
        assert_eq!(info.business_capability[0], "user-management");
        assert_eq!(info.design_pattern[0], "Strategy");
        assert_eq!(info.risks.len(), 1);
        assert_eq!(info.risks[0].severity, RiskSeverity::Medium);
    }

    #[test]
    fn parse_llm_response_markdown() {
        let response = "```json\n{\"purpose\": \"Does stuff\"}\n```";
        let info = parse_semantic_response(response).unwrap();
        assert_eq!(info.purpose.unwrap(), "Does stuff");
    }

    #[test]
    fn build_prompt_includes_metadata() {
        let node = IrNode::new(
            ckc_ir::SymbolId::new("lib.py", vec!["lib".into()], "validate", 0),
            ckc_ir::NodeKind::Function,
            "validate",
            ckc_ir::SourceLocation {
                line_start: 10,
                line_end: 15,
                col_start: 0,
                col_end: 0,
            },
        );

        let prompt = build_user_prompt(&node, None);
        assert!(prompt.contains("Function: validate"));
        assert!(prompt.contains("lib.py:10"));
    }
}
