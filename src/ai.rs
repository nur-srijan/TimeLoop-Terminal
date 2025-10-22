use serde::{Deserialize, Serialize};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, CONTENT_TYPE, USER_AGENT};
use crate::{Storage, EventType};

#[derive(Debug, Clone, PartialEq)]
pub enum ApiProvider {
    OpenRouter,
    OpenAI,
}

impl ApiProvider {
    pub fn from_env() -> crate::Result<Self> {
        // Check for OpenAI API key first, then OpenRouter
        if std::env::var("OPENAI_API_KEY").is_ok() {
            Ok(ApiProvider::OpenAI)
        } else if std::env::var("OPENROUTER_API_KEY").is_ok() {
            Ok(ApiProvider::OpenRouter)
        } else {
            Err(crate::error::TimeLoopError::Configuration(
                "Neither OPENAI_API_KEY nor OPENROUTER_API_KEY environment variable found".to_string()
            ))
        }
    }

    pub fn base_url(&self) -> String {
        match self {
            ApiProvider::OpenAI => "https://api.openai.com/v1".to_string(),
            ApiProvider::OpenRouter => {
                std::env::var("OPENROUTER_BASE_URL")
                    .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string())
            }
        }
    }

    pub fn api_key(&self) -> crate::Result<String> {
        match self {
            ApiProvider::OpenAI => {
                std::env::var("OPENAI_API_KEY")
                    .map_err(|_| crate::error::TimeLoopError::Configuration("Missing OPENAI_API_KEY".to_string()))
            }
            ApiProvider::OpenRouter => {
                std::env::var("OPENROUTER_API_KEY")
                    .map_err(|_| crate::error::TimeLoopError::Configuration("Missing OPENROUTER_API_KEY".to_string()))
            }
        }
    }

    pub fn default_model(&self) -> &'static str {
        match self {
            ApiProvider::OpenAI => "gpt-3.5-turbo",
            ApiProvider::OpenRouter => "openrouter/auto",
        }
    }
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChatMessageOut,
}

#[derive(Deserialize)]
struct ChatMessageOut {
    content: String,
}

fn build_timeline(storage: &Storage, session_id: &str, max_items: usize) -> crate::Result<String> {
    let mut events = storage.get_events_for_session(session_id)?;
    events.sort_by_key(|e| e.sequence_number);
    let mut lines: Vec<String> = Vec::new();
    for e in events.into_iter().rev().take(max_items).rev() {
        match e.event_type {
            EventType::Command { ref command, ref working_directory, ref exit_code, .. } => {
                lines.push(format!("[cmd] '{}' (dir: {}, exit: {})", command, working_directory, exit_code));
            }
            EventType::FileChange { ref path, ref change_type, .. } => {
                lines.push(format!("[file] {:?} {}", change_type, path));
            }
            EventType::KeyPress { ref key, .. } => {
                lines.push(format!("[key] {}", key));
            }
            EventType::TerminalState { ref screen_size, .. } => {
                lines.push(format!("[term] size {}x{}", screen_size.0, screen_size.1));
            }
            EventType::SessionMetadata { ref name, .. } => {
                lines.push(format!("[session] {}", name));
            }
        }
    }
    Ok(lines.join("\n"))
}

pub async fn summarize_session(session_id: &str, model: Option<&str>, provider: Option<ApiProvider>) -> crate::Result<String> {
    let storage = Storage::new()?;
    let timeline = build_timeline(&storage, session_id, 200)?;
    let prompt = format!("You are an expert assistant. Summarize the following terminal session succinctly with key actions, commands run, files changed, and possible next steps.\n\n{}", timeline);

    // Determine API provider
    let api_provider = provider.unwrap_or_else(|| ApiProvider::from_env().unwrap_or(ApiProvider::OpenRouter));
    let api_key = api_provider.api_key()?;
    let base_url = api_provider.base_url();
    let model_name = model.unwrap_or(api_provider.default_model());

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {}", api_key)).unwrap());
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(USER_AGENT, HeaderValue::from_static("timeloop-terminal/ai"));

    let body = ChatRequest {
        model: model_name.to_string(),
        messages: vec![
            ChatMessage { role: "system".to_string(), content: "You are a concise expert assistant for terminal session summaries.".to_string() },
            ChatMessage { role: "user".to_string(), content: prompt },
        ],
    };

    let client = reqwest::Client::new();
    let resp = client.post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| crate::error::TimeLoopError::Unknown(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let error_text = resp.text().await.unwrap_or_else(|_| "Unknown error".to_string());
        return Err(crate::error::TimeLoopError::Unknown(format!("API request failed ({}): {}", status, error_text)));
    }

    let parsed: ChatResponse = resp.json().await.map_err(|e| crate::error::TimeLoopError::Unknown(e.to_string()))?;
    let content = parsed.choices.get(0).map(|c| c.message.content.clone()).unwrap_or_else(|| "No response".to_string());
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Use a mutex to ensure tests run sequentially and don't interfere with each other
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_api_provider_from_env() {
        let _guard = TEST_MUTEX.lock().unwrap();
        
        // Save original values
        let openai_key = std::env::var("OPENAI_API_KEY").ok();
        let openrouter_key = std::env::var("OPENROUTER_API_KEY").ok();
        let openrouter_url = std::env::var("OPENROUTER_BASE_URL").ok();
        
        // Test with OpenAI key
        std::env::set_var("OPENAI_API_KEY", "test-key");
        std::env::remove_var("OPENROUTER_API_KEY");
        
        let provider = ApiProvider::from_env().unwrap();
        assert_eq!(provider, ApiProvider::OpenAI);
        assert_eq!(provider.base_url(), "https://api.openai.com/v1");
        assert_eq!(provider.default_model(), "gpt-3.5-turbo");
        
        // Test with OpenRouter key
        std::env::remove_var("OPENAI_API_KEY");
        std::env::set_var("OPENROUTER_API_KEY", "test-key");
        
        let provider = ApiProvider::from_env().unwrap();
        assert_eq!(provider, ApiProvider::OpenRouter);
        assert_eq!(provider.base_url(), "https://openrouter.ai/api/v1");
        assert_eq!(provider.default_model(), "openrouter/auto");
        
        // Test with custom OpenRouter URL
        std::env::set_var("OPENROUTER_BASE_URL", "https://custom.openrouter.com/api/v1");
        let provider = ApiProvider::from_env().unwrap();
        assert_eq!(provider.base_url(), "https://custom.openrouter.com/api/v1");
        
        // Restore original values
        if let Some(key) = openai_key {
            std::env::set_var("OPENAI_API_KEY", key);
        } else {
            std::env::remove_var("OPENAI_API_KEY");
        }
        if let Some(key) = openrouter_key {
            std::env::set_var("OPENROUTER_API_KEY", key);
        } else {
            std::env::remove_var("OPENROUTER_API_KEY");
        }
        if let Some(url) = openrouter_url {
            std::env::set_var("OPENROUTER_BASE_URL", url);
        } else {
            std::env::remove_var("OPENROUTER_BASE_URL");
        }
    }

    #[test]
    fn test_api_provider_no_keys() {
        let _guard = TEST_MUTEX.lock().unwrap();
        
        // Save original values
        let openai_key = std::env::var("OPENAI_API_KEY").ok();
        let openrouter_key = std::env::var("OPENROUTER_API_KEY").ok();
        
        // Remove environment variables
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("OPENROUTER_API_KEY");
        
        let result = ApiProvider::from_env();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Neither OPENAI_API_KEY nor OPENROUTER_API_KEY"));
        
        // Restore original values
        if let Some(key) = openai_key {
            std::env::set_var("OPENAI_API_KEY", key);
        }
        if let Some(key) = openrouter_key {
            std::env::set_var("OPENROUTER_API_KEY", key);
        }
    }

    #[test]
    fn test_api_provider_precedence() {
        let _guard = TEST_MUTEX.lock().unwrap();
        
        // Save original values
        let openai_key = std::env::var("OPENAI_API_KEY").ok();
        let openrouter_key = std::env::var("OPENROUTER_API_KEY").ok();
        
        // OpenAI should take precedence when both are present
        std::env::set_var("OPENAI_API_KEY", "openai-key");
        std::env::set_var("OPENROUTER_API_KEY", "openrouter-key");
        
        let provider = ApiProvider::from_env().unwrap();
        assert_eq!(provider, ApiProvider::OpenAI);
        
        // Restore original values
        if let Some(key) = openai_key {
            std::env::set_var("OPENAI_API_KEY", key);
        } else {
            std::env::remove_var("OPENAI_API_KEY");
        }
        if let Some(key) = openrouter_key {
            std::env::set_var("OPENROUTER_API_KEY", key);
        } else {
            std::env::remove_var("OPENROUTER_API_KEY");
        }
    }

    #[test]
    fn test_api_key_retrieval() {
        let _guard = TEST_MUTEX.lock().unwrap();
        
        // Save original values
        let openai_key = std::env::var("OPENAI_API_KEY").ok();
        let openrouter_key = std::env::var("OPENROUTER_API_KEY").ok();
        
        // Test OpenAI key retrieval
        std::env::set_var("OPENAI_API_KEY", "test-openai-key");
        std::env::remove_var("OPENROUTER_API_KEY"); // Ensure OpenRouter key is not set
        let openai_provider = ApiProvider::OpenAI;
        assert_eq!(openai_provider.api_key().unwrap(), "test-openai-key");
        
        // Test OpenRouter key retrieval
        std::env::remove_var("OPENAI_API_KEY"); // Ensure OpenAI key is not set
        std::env::set_var("OPENROUTER_API_KEY", "test-openrouter-key");
        let openrouter_provider = ApiProvider::OpenRouter;
        assert_eq!(openrouter_provider.api_key().unwrap(), "test-openrouter-key");
        
        // Restore original values
        if let Some(key) = openai_key {
            std::env::set_var("OPENAI_API_KEY", key);
        } else {
            std::env::remove_var("OPENAI_API_KEY");
        }
        if let Some(key) = openrouter_key {
            std::env::set_var("OPENROUTER_API_KEY", key);
        } else {
            std::env::remove_var("OPENROUTER_API_KEY");
        }
    }
}

// Backward compatibility function
pub async fn summarize_session_legacy(session_id: &str, model: &str) -> crate::Result<String> {
    summarize_session(session_id, Some(model), None).await
}

