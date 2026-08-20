// Ollama HTTP client — talks to a local Ollama instance at localhost:11434.
//
// Ollama is a free, open-source tool that runs LLMs locally. The user
// installs it separately (https://ollama.ai). We detect it at runtime
// and degrade gracefully when absent.
//
// The Assist layer is NEVER on the critical analysis path. Key/BPM/energy
// detection runs entirely locally without any LLM call. The Assist features
// (metadata repair, genre inference, setlist analysis, transition
// explanations, NL set planning) are all user-initiated and optional.

use anyhow::{Context, Result};
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const OLLAMA_BASE_URL: &str = "http://localhost:11434";
const TIMEOUT_SECS: u64 = 120; // LLM generation can be slow

/// Ollama model info returned by /api/tags
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModel {
    pub name: String,
    pub size: Option<u64>,
}

/// Status of the Ollama integration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistStatus {
    pub available: bool,
    pub ollama_url: String,
    pub models: Vec<OllamaModel>,
    pub selected_model: Option<String>,
    pub enabled: bool,
}

/// Chat message in Ollama format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // "system", "user", "assistant"
    pub content: String,
}

/// Request to Ollama /api/chat endpoint
#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    options: ChatOptions,
}

#[derive(Debug, Serialize)]
struct ChatOptions {
    temperature: f32,
    num_ctx: u32,
}

/// Response from Ollama /api/chat endpoint
#[derive(Debug, Deserialize)]
struct ChatResponse {
    message: Option<ChatMessage>,
    error: Option<String>,
}

/// Tags response from Ollama /api/tags
#[derive(Debug, Deserialize)]
struct TagsResponse {
    models: Vec<TagsModel>,
}

#[derive(Debug, Deserialize)]
struct TagsModel {
    name: String,
    size: Option<u64>,
}

pub struct OllamaClient {
    base_url: String,
    client: reqwest::Client,
}

impl OllamaClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            base_url: OLLAMA_BASE_URL.to_string(),
            client,
        }
    }

    /// Check if Ollama is running and list available models.
    pub async fn check_status(&self) -> (bool, Vec<OllamaModel>) {
        match self.client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
        {
            Ok(resp) => {
                if resp.status().is_success() {
                    match resp.json::<TagsResponse>().await {
                        Ok(tags) => {
                            let models = tags.models.into_iter().map(|m| OllamaModel {
                                name: m.name,
                                size: m.size,
                            }).collect();
                            (true, models)
                        }
                        Err(_) => (true, vec![]),
                    }
                } else {
                    (false, vec![])
                }
            }
            Err(_) => (false, vec![]),
        }
    }

    /// Send a chat completion request to Ollama.
    /// Returns the assistant's response text.
    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        temperature: f32,
    ) -> Result<String> {
        let request = ChatRequest {
            model: model.to_string(),
            messages,
            stream: false,
            options: ChatOptions {
                temperature,
                num_ctx: 4096,
            },
        };

        let resp: ChatResponse = self.client
            .post(format!("{}/api/chat", self.base_url))
            .json(&request)
            .send()
            .await
            .context("Failed to connect to Ollama")?
            .json()
            .await
            .context("Failed to parse Ollama response")?;

        if let Some(err) = resp.error {
            return Err(anyhow::anyhow!("Ollama error: {}", err));
        }

        Ok(resp.message.map(|m| m.content).unwrap_or_default())
    }

    /// Send a simple prompt (system + user) and get a text response.
    pub async fn prompt(&self, model: &str, system: &str, user: &str) -> Result<String> {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: system.to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: user.to_string(),
            },
        ];
        self.chat(model, messages, 0.3).await
    }

    /// Send a prompt and parse the response as JSON.
    /// Falls back to raw text if JSON parsing fails.
    pub async fn prompt_json(&self, model: &str, system: &str, user: &str) -> Result<serde_json::Value> {
        let response = self.prompt(model, system, user).await?;
        // Try to extract JSON from the response (LLMs sometimes wrap in ```json)
        let cleaned = response
            .trim()
            .strip_prefix("```json")
            .or_else(|| response.trim().strip_prefix("```"))
            .unwrap_or(&response)
            .trim()
            .strip_suffix("```")
            .unwrap_or(&response)
            .trim();

        serde_json::from_str(cleaned)
            .or_else(|_| {
                // Try to find JSON object in the response
                let start = response.find('{');
                let end = response.rfind('}');
                if let (Some(s), Some(e)) = (start, end) {
                    serde_json::from_str(&response[s..=e])
                } else {
                    Err(serde_json::Error::custom("No JSON found in response"))
                }
            })
            .context("Failed to parse LLM response as JSON")
    }
}
