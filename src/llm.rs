// Ported from codedesign-loop (Loop-Suite)'s src/llm.rs. Text-only calls disable all tools
// (`--tools ""`), same as the original.
//
// discourse critics now both receive real vision input on either backend — the Claude CLI opens
// the file directly with the Read tool, while OpenRouter attaches the image to the message
// directly as a base64 data URI.
// Reason for this change (from research notes): "Nine Judges, Two Effective Votes"
// (arXiv:2605.29800) — if judges are correlated with each other, N judges effectively collapse to
// 2 or fewer votes; PoLL (arXiv:2404.18796) — a heterogeneous provider mix correlates better with
// humans and is cheaper than a single strong judge. If all 3 critics share the same backend and
// model, discourse's "independence" premise breaks down entirely — main.rs's
// spec.critic_backends actually mixes these two providers across critics.
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::de::DeserializeOwned;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
pub const OPENROUTER_DEFAULT_MODEL: &str = "openai/gpt-oss-120b";

#[derive(Clone, Debug)]
pub enum Provider {
    ClaudeCli { bin: String },
    OpenRouter { api_key: String },
}

#[derive(Clone, Debug)]
pub struct Llm {
    pub provider_label: &'static str,
    provider: Provider,
    model: Option<String>,
    retries: u32,
    verbose: bool,
}

impl Llm {
    pub fn claude_cli(bin: String, model: Option<String>, retries: u32, verbose: bool) -> Self {
        Self {
            provider_label: "claude",
            provider: Provider::ClaudeCli { bin },
            model,
            retries,
            verbose,
        }
    }

    pub fn openrouter(model: Option<String>, retries: u32, verbose: bool) -> Result<Self> {
        let api_key = std::env::var("OPENROUTER_API_KEY")
            .context("OPENROUTER_API_KEY environment variable not set")?;
        Ok(Self {
            provider_label: "openrouter",
            provider: Provider::OpenRouter { api_key },
            model: Some(model.unwrap_or_else(|| OPENROUTER_DEFAULT_MODEL.to_string())),
            retries,
            verbose,
        })
    }

    fn call_once(&self, prompt: &str, system: Option<&str>, images: &[PathBuf]) -> Result<String> {
        match &self.provider {
            Provider::ClaudeCli { bin } => {
                call_claude(bin, self.model.as_deref(), prompt, system, images)
            }
            Provider::OpenRouter { api_key } => {
                call_openrouter(api_key, self.model.as_deref(), prompt, system, images)
            }
        }
    }

    pub fn text(&self, prompt: &str, system: Option<&str>) -> Result<String> {
        self.text_with_images(prompt, system, &[])
    }

    pub fn text_with_images(
        &self,
        prompt: &str,
        system: Option<&str>,
        images: &[PathBuf],
    ) -> Result<String> {
        let mut last = None;
        for attempt in 0..=self.retries {
            match self.call_once(prompt, system, images) {
                Ok(raw) if !raw.trim().is_empty() => return Ok(raw),
                Ok(_) => last = Some(anyhow!("empty response")),
                Err(error) => last = Some(error),
            }
            if self.verbose {
                match last.as_ref() {
                    Some(error) => {
                        eprintln!("[retry {}/{}] {error}", attempt + 1, self.retries);
                    }
                    None => {
                        eprintln!(
                            "[retry {}/{}] unknown retry error",
                            attempt + 1,
                            self.retries
                        );
                    }
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("LLM call failed")))
    }

    pub fn json_with_images<T: DeserializeOwned>(
        &self,
        prompt: &str,
        system: Option<&str>,
        images: &[PathBuf],
    ) -> Result<T> {
        let mut last = None;
        for attempt in 0..=self.retries {
            match self
                .call_once(prompt, system, images)
                .and_then(|raw| extract_json(&raw))
                .and_then(|value| {
                    serde_json::from_value(value).with_context(|| {
                        format!("JSON schema mismatch: {}", std::any::type_name::<T>())
                    })
                }) {
                Ok(value) => return Ok(value),
                Err(error) => last = Some(error),
            }
            if self.verbose {
                match last.as_ref() {
                    Some(error) => {
                        eprintln!("[json retry {}/{}] {error}", attempt + 1, self.retries);
                    }
                    None => {
                        eprintln!(
                            "[json retry {}/{}] unknown json retry error",
                            attempt + 1,
                            self.retries
                        );
                    }
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("JSON response failed")))
    }
}

fn call_claude(
    bin: &str,
    model: Option<&str>,
    prompt: &str,
    system: Option<&str>,
    images: &[PathBuf],
) -> Result<String> {
    let mut command = Command::new(bin);
    command
        .arg("-p")
        .arg("--output-format")
        .arg("json")
        .arg("--safe-mode")
        .arg("--disable-slash-commands")
        .arg("--no-session-persistence");

    if images.is_empty() {
        command.arg("--tools").arg("");
    } else {
        // discourse critic calls need to see the rendered PNG — grant only minimal Read tool access.
        let dir = images[0].parent().ok_or_else(|| {
            anyhow!(
                "image path has no parent directory: {}",
                images[0].display()
            )
        })?;
        command
            .arg("--tools")
            .arg("Read")
            .arg("--allowedTools")
            .arg("Read")
            .arg("--add-dir")
            .arg(dir);
    }

    if let Some(model) = model {
        command.arg("--model").arg(model);
    }
    if let Some(system) = system {
        command.arg("--append-system-prompt").arg(system);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to run `{bin}` (check installation and PATH)"))?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow!("failed to open claude stdin"))?
        .write_all(prompt.as_bytes())?;
    drop(child.stdin.take());

    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(anyhow!(
            "claude exited with code {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let envelope: serde_json::Value = serde_json::from_str(stdout.trim()).with_context(|| {
        format!(
            "failed to parse claude JSON output: {}",
            truncate(&stdout, 400)
        )
    })?;
    if envelope
        .get("is_error")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err(anyhow!("claude error response: {}", truncate(&stdout, 400)));
    }
    envelope
        .get("result")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| anyhow!("claude response has no result field"))
}

fn image_mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

fn call_openrouter(
    api_key: &str,
    model: Option<&str>,
    prompt: &str,
    system: Option<&str>,
    images: &[PathBuf],
) -> Result<String> {
    let mut messages = Vec::new();
    if let Some(system) = system {
        messages.push(serde_json::json!({"role": "system", "content": system}));
    }

    let user_content = if images.is_empty() {
        serde_json::Value::String(prompt.to_string())
    } else {
        let mut parts = vec![serde_json::json!({"type": "text", "text": prompt})];
        for image_path in images {
            let bytes = std::fs::read(image_path)
                .with_context(|| format!("failed to read image: {}", image_path.display()))?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            let data_url = format!("data:{};base64,{}", image_mime(image_path), b64);
            parts.push(serde_json::json!({
                "type": "image_url",
                "image_url": { "url": data_url }
            }));
        }
        serde_json::Value::Array(parts)
    };
    messages.push(serde_json::json!({"role": "user", "content": user_content}));

    let mut body = serde_json::Map::new();
    body.insert(
        "model".to_string(),
        serde_json::Value::String(model.unwrap_or(OPENROUTER_DEFAULT_MODEL).to_string()),
    );
    body.insert(
        "messages".to_string(),
        serde_json::Value::Array(messages.into_iter().collect()),
    );

    let body = serde_json::Value::Object(body);

    let response = ureq::post(OPENROUTER_URL)
        .config()
        .http_status_as_error(false)
        .build()
        .header("Authorization", &format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .send_json(body);
    let mut response = match response {
        Ok(response) => response,
        Err(error) => return Err(anyhow!("OpenRouter call failed: {error}")),
    };
    if !response.status().is_success() {
        let code = response.status().as_u16();
        let body = response.body_mut().read_to_string().unwrap_or_default();
        return Err(anyhow!(
            "OpenRouter response code {code}: {}",
            truncate(&body, 400)
        ));
    }
    let value: serde_json::Value = response
        .body_mut()
        .read_json()
        .context("failed to parse OpenRouter response JSON")?;
    value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| {
            anyhow!(
                "OpenRouter response has no content: {}",
                truncate(&value.to_string(), 400)
            )
        })
}

pub fn extract_json(raw: &str) -> Result<serde_json::Value> {
    let trimmed = raw.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        let after = after.strip_prefix("json").unwrap_or(after);
        if let Some(end) = after.find("```") {
            if let Ok(value) = serde_json::from_str(after[..end].trim()) {
                return Ok(value);
            }
        }
    }
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end {
            if let Ok(value) = serde_json::from_str(&trimmed[start..=end]) {
                return Ok(value);
            }
        }
    }
    if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']')) {
        if start < end {
            if let Ok(value) = serde_json::from_str(&trimmed[start..=end]) {
                return Ok(value);
            }
        }
    }
    Err(anyhow!(
        "failed to extract JSON: {}",
        truncate(trimmed, 400)
    ))
}

pub fn truncate(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        value.to_string()
    } else {
        value.chars().take(limit).collect::<String>() + "…"
    }
}
