//! The wire side of one OpenCode generation: the request, its route fallback,
//! the startup retry, and the shared deadline. The parent module decides which
//! model, if any, a request names.

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use color_eyre::eyre::{Result, WrapErr, eyre};
use serde::{Deserialize, Serialize};

use super::Model;
use crate::desktop::opencode_service::Service;

/// Newest one-shot generation route first; older OpenCode V2 builds only serve the later name.
const GENERATE_ENDPOINTS: [&str; 2] = ["/api/experimental/generate", "/api/generate"];

#[derive(Serialize)]
pub(super) struct Request<'a> {
    pub(super) prompt: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) model: Option<&'a Model>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Response {
    Success { data: ResponseData },
    Error(GenerationFailure),
}

#[derive(Deserialize)]
pub(super) struct ResponseData {
    pub(super) text: String,
}

pub(super) fn remaining_generation_deadline(
    started: Instant,
    deadline: Duration,
) -> Result<Duration> {
    let remaining = deadline.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        Err(eyre!(
            "OpenCode generation exceeded {} seconds",
            deadline.as_secs()
        ))
    } else {
        Ok(remaining)
    }
}

pub(super) fn generate_single_model(
    prompt: &str,
    model: Option<&Model>,
    started: Instant,
    deadline: Duration,
    cancelled: &AtomicBool,
    mut post: impl FnMut(&str, Duration, &AtomicBool) -> Result<Response>,
) -> Result<String> {
    let request = Request { prompt, model };
    let data = serde_json::to_string(&request)?;
    for attempt in 0..2 {
        let remaining = remaining_generation_deadline(started, deadline)?;
        match post(&data, remaining, cancelled)? {
            Response::Success { data } => return Ok(data.text),
            Response::Error(error)
                if attempt == 0
                    && (error.model_unavailable()
                        || retryable_generation_error(&error.message)) =>
            {
                tracing::warn!(message = %error.message, "OpenCode model unavailable during startup; retrying");
                wait_for_generation_retry(started, deadline, cancelled)?;
            }
            Response::Error(error) => {
                let message = error.message;
                return Err(eyre!("OpenCode generation failed: {message}"));
            }
        }
    }
    unreachable!("generation loop returns on its second attempt")
}

/// Posts to the newest generation route and falls back to the older name only
/// when the server reports that route missing (a 404 that isn't OpenCode's
/// own error). Every attempt shares one deadline.
pub(super) fn post_generation(
    service: &Service,
    data: &str,
    deadline: Duration,
    cancelled: &AtomicBool,
) -> Result<Response> {
    let started = Instant::now();
    let mut response = None;
    for path in GENERATE_ENDPOINTS {
        let attempt = service.request(
            "POST",
            path,
            Some(data.as_bytes()),
            None,
            deadline.saturating_sub(started.elapsed()),
            cancelled,
        )?;
        let missing = route_missing(&attempt);
        response = Some(attempt);
        if !missing {
            break;
        }
    }
    let response = response.expect("at least one generation route");
    if route_missing(&response) {
        return Err(eyre!("OpenCode does not serve a generation route"));
    }
    serde_json::from_slice(&response.body)
        .wrap_err("OpenCode returned an invalid generation response")
}

/// A 404 means the route is missing only when it isn't OpenCode's own tagged
/// error (such as "Model not found"), which the route answered.
fn route_missing(response: &crate::desktop::opencode_service::Response) -> bool {
    response.status == 404
        && !serde_json::from_slice::<serde_json::Value>(&response.body).is_ok_and(|value| {
            value.get("_tag").is_some()
                && value
                    .get("message")
                    .is_some_and(serde_json::Value::is_string)
        })
}

pub(super) fn retryable_generation_error(message: &str) -> bool {
    message.starts_with("Model unavailable:")
}

pub(super) fn wait_for_generation_retry(
    started: Instant,
    deadline: Duration,
    cancelled: &AtomicBool,
) -> Result<()> {
    let wait_started = Instant::now();
    while wait_started.elapsed() < Duration::from_millis(1_500) {
        if cancelled.load(Ordering::Acquire) {
            return Err(eyre!("OpenCode generation was cancelled"));
        }
        if started.elapsed() >= deadline {
            return Err(eyre!(
                "OpenCode generation exceeded {} seconds",
                deadline.as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// OpenCode's protocol error envelope. Only typed provider failures admit
/// failover; human-readable messages are for diagnostics, never routing.
#[derive(Debug, Default, Deserialize)]
pub(super) struct GenerationFailure {
    #[serde(default, rename = "_tag")]
    pub(super) tag: String,
    #[serde(default)]
    pub(super) message: String,
    #[serde(default, rename = "providerID")]
    pub(super) provider: Option<String>,
    #[serde(default)]
    pub(super) reason: Option<String>,
    #[serde(default)]
    pub(super) field: Option<String>,
}

impl GenerationFailure {
    fn quota_limited(&self, model: &Model) -> bool {
        self.tag == "ProviderRequestError"
            && self.reason.as_deref() == Some("rate_limit")
            && self.provider.as_deref() == Some(model.provider.as_str())
    }

    pub(super) fn model_unavailable(&self) -> bool {
        self.tag == "InvalidRequestError" && self.field.as_deref() == Some("model")
    }
}

/// At most one same-model provider fallback, using the original prompt,
/// variant, cancellation flag, and remaining deadline. No retry on transport
/// errors, timeouts, authentication failures, or untyped legacy errors.
#[allow(clippy::too_many_arguments)]
pub(super) fn generate_with_quota_fallback(
    prompt: &str,
    model: &Model,
    provider: &str,
    started: Instant,
    deadline: Duration,
    cancelled: &AtomicBool,
    mut post: impl FnMut(&str, Duration, &AtomicBool) -> Result<Response>,
) -> Result<String> {
    let request =
        |model: &Model, post: &mut dyn FnMut(&str, Duration, &AtomicBool) -> Result<Response>| {
            if cancelled.load(Ordering::Acquire) {
                return Err(eyre!("OpenCode generation was cancelled"));
            }
            let remaining = remaining_generation_deadline(started, deadline)?;
            let data = serde_json::to_string(&Request {
                prompt,
                model: Some(model),
            })?;
            post(&data, remaining, cancelled)
        };
    let failure = match request(model, &mut post)? {
        Response::Success { data } => return Ok(data.text),
        Response::Error(error) => error,
    };
    let provider = provider.trim();
    if !failure.quota_limited(model) || provider.is_empty() || provider == model.provider {
        return Err(eyre!("OpenCode generation failed: {}", failure.message));
    }
    let fallback = Model {
        provider: provider.to_owned(),
        ..model.clone()
    };
    tracing::info!(primary_provider = %model.provider, fallback_provider = %fallback.provider,
        model = %model.id, reason = "rate_limit", "OpenCode provider quota fallback");
    match request(&fallback, &mut post)? {
        Response::Success { data } => Ok(data.text),
        Response::Error(error) => Err(eyre!(
            "OpenCode fallback {} failed: {}",
            fallback.provider,
            error.message
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        Model {
            provider: "opencode-go".into(),
            id: "gpt-6-luna".into(),
            variant: Some("none".into()),
        }
    }

    fn quota() -> Response {
        serde_json::from_value(serde_json::json!({
            "_tag": "ProviderRequestError", "providerID": "opencode-go",
            "reason": "rate_limit", "status": 429, "message": "arbitrary localized message"
        }))
        .unwrap()
    }

    #[test]
    fn typed_quota_rejection_retries_same_prompt_model_and_variant_on_configured_provider() {
        let response = r#"{"_tag":"ProviderRequestError","providerID":"opencode-go","reason":"rate_limit","status":402,"message":"任意のメッセージ"}"#;
        let (service, server) = super::super::tests::generation_server(vec![
            ("/api/experimental/generate", 502, response),
            (
                "/api/experimental/generate",
                200,
                r#"{"data":{"text":"Clean text."}}"#,
            ),
        ]);
        let result = generate_with_quota_fallback(
            "Exact prompt.",
            &model(),
            "openai",
            Instant::now(),
            Duration::from_secs(5),
            &AtomicBool::new(false),
            |data, deadline, cancelled| post_generation(&service, data, deadline, cancelled),
        )
        .unwrap();
        assert_eq!(result, "Clean text.");
        let requests: Vec<serde_json::Value> = server
            .join()
            .unwrap()
            .into_iter()
            .map(|body| serde_json::from_slice(&body).unwrap())
            .collect();
        assert_eq!(requests.len(), 2);
        for (request, provider) in requests.iter().zip(["opencode-go", "openai"]) {
            assert_eq!(request["prompt"], "Exact prompt.");
            assert_eq!(
                request["model"],
                serde_json::json!({"providerID":provider,"id":"gpt-6-luna","variant":"none"})
            );
        }
    }

    #[test]
    fn only_typed_limits_from_the_requested_provider_allow_fallback() {
        for error in [
            serde_json::json!({"message":"usage quota rate limit exhausted"}),
            serde_json::json!({"_tag":"ProviderRequestError","providerID":"opencode-go","reason":"authentication","status":401,"message":"usage limit"}),
            serde_json::json!({"_tag":"ProviderRequestError","providerID":"opencode-go","reason":"provider","status":504,"message":"timeout"}),
            serde_json::json!({"_tag":"ProviderRequestError","providerID":"opencode-go","reason":"overloaded","status":503,"message":"busy"}),
            serde_json::json!({"_tag":"ProviderRequestError","providerID":"opencode-go","reason":"content_filter","message":"denied"}),
            serde_json::json!({"_tag":"ProviderRequestError","providerID":"opencode-go","reason":"invalid_request","message":"invalid"}),
            serde_json::json!({"_tag":"ProviderRequestError","providerID":"other","reason":"rate_limit","status":429,"message":"limit"}),
            serde_json::json!({"_tag":"InvalidRequestError","reason":"rate_limit","providerID":"opencode-go","message":"limit"}),
        ] {
            let mut calls = 0;
            let result = generate_with_quota_fallback(
                "input",
                &model(),
                "openai",
                Instant::now(),
                Duration::from_secs(5),
                &AtomicBool::new(false),
                |_, _, _| {
                    calls += 1;
                    assert_eq!(calls, 1, "unexpected fallback for {error}");
                    Ok(serde_json::from_value(error.clone()).unwrap())
                },
            );
            assert!(result.is_err());
            assert_eq!(calls, 1);
        }
    }

    #[test]
    fn timeouts_and_transport_failures_do_not_retry() {
        for error in ["deadline exceeded", "connection reset"] {
            let mut calls = 0;
            assert!(
                generate_with_quota_fallback(
                    "input",
                    &model(),
                    "openai",
                    Instant::now(),
                    Duration::from_secs(5),
                    &AtomicBool::new(false),
                    |_, _, _| {
                        calls += 1;
                        Err(eyre!("{error}"))
                    }
                )
                .is_err()
            );
            assert_eq!(calls, 1);
        }
    }

    #[test]
    fn cancellation_and_deadline_expiry_between_attempts_prevent_fallback() {
        for cancel in [true, false] {
            let cancelled = AtomicBool::new(false);
            let mut calls = 0;
            let result = generate_with_quota_fallback(
                "input",
                &model(),
                "openai",
                Instant::now(),
                Duration::from_millis(20),
                &cancelled,
                |_, _, _| {
                    calls += 1;
                    if cancel {
                        cancelled.store(true, Ordering::Release);
                    } else {
                        thread::sleep(Duration::from_millis(25));
                    }
                    Ok(quota())
                },
            );
            assert!(result.is_err());
            assert_eq!(calls, 1);
        }
    }

    #[test]
    fn one_fallback_only_and_both_attempts_share_the_deadline() {
        let mut remaining = Vec::new();
        let result = generate_with_quota_fallback(
            "input",
            &model(),
            "openai",
            Instant::now(),
            Duration::from_secs(2),
            &AtomicBool::new(false),
            |_, deadline, _| {
                remaining.push(deadline);
                if remaining.len() == 1 {
                    thread::sleep(Duration::from_millis(20));
                }
                Ok(quota())
            },
        );
        assert!(result.is_err());
        assert_eq!(remaining.len(), 2);
        assert!(remaining[1] < remaining[0]);
    }

    #[test]
    fn successful_primary_and_empty_or_duplicate_provider_do_not_fallback() {
        for provider in ["openai", "", "opencode-go", " opencode-go "] {
            let mut calls = 0;
            let result = generate_with_quota_fallback(
                "input",
                &model(),
                provider,
                Instant::now(),
                Duration::from_secs(5),
                &AtomicBool::new(false),
                |_, _, _| {
                    calls += 1;
                    if provider == "openai" {
                        Ok(Response::Success {
                            data: ResponseData {
                                text: "done".into(),
                            },
                        })
                    } else {
                        Ok(quota())
                    }
                },
            );
            assert_eq!(result.is_ok(), provider == "openai");
            assert_eq!(calls, 1);
        }
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    #[test]
    #[ignore = "sends one synthetic prompt to the configured Codex OAuth provider"]
    fn live_quota_fallback_to_codex() {
        let service = crate::desktop::opencode_service::find().expect("OpenCode must be running");
        let model = Model {
            provider: "opencode-go".into(),
            id: "gpt-6-luna".into(),
            variant: Some("none".into()),
        };
        let mut attempts = 0;
        let started = Instant::now();
        let text = generate_with_quota_fallback(
            "Reply with exactly: test",
            &model,
            "openai",
            started,
            Duration::from_secs(8),
            &AtomicBool::new(false),
            |data, deadline, cancelled| {
                attempts += 1;
                if attempts == 1 {
                    Ok(Response::Error(GenerationFailure {
                        tag: "ProviderRequestError".into(),
                        provider: Some("opencode-go".into()),
                        reason: Some("rate_limit".into()),
                        message: "synthetic quota rejection".into(),
                        field: None,
                    }))
                } else {
                    let request: serde_json::Value = serde_json::from_str(data).unwrap();
                    assert_eq!(request["model"]["providerID"], "openai");
                    post_generation(&service, data, deadline, cancelled)
                }
            },
        )
        .unwrap();
        assert_eq!(text.trim(), "test");
        assert_eq!(attempts, 2);
        println!(
            "Synthetic quota -> live Codex Luna passed in {:?}",
            started.elapsed()
        );
    }
}

/// Finds the running OpenCode service, or why it can't answer. Service
/// discovery spends the generation's own remaining budget, so a stalled
/// OpenCode falls back by the deadline instead of adding its fixed lookup
/// timeouts on top.
pub(super) fn generation_service(
    started: Instant,
    deadline: Duration,
    cancelled: &AtomicBool,
) -> Result<Service> {
    let lock = crate::policy::Lock::OpenCodeProcessing;
    if crate::policy::Policy::current().locks(lock) {
        return Err(eyre!(lock.refusal()));
    }
    let service = crate::desktop::opencode_service::find_cancellable(
        remaining_generation_deadline(started, deadline)?,
        cancelled,
    );
    if cancelled.load(Ordering::Acquire) {
        return Err(eyre!("OpenCode generation was cancelled"));
    }
    remaining_generation_deadline(started, deadline)?;
    service.map_err(|missing| eyre!(missing.message()))
}
