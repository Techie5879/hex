# Local SpeakoFlow cleanup

This fork can clean dictation with SpeakoFlow Mini on an Apple Silicon Mac,
without OpenCode or a cloud provider. Enable **Labs > Local SpeakoFlow cleanup**.
It applies to normal dictation in every mode. A mode can optionally run OpenCode
afterward. The raw-dictation shortcut still bypasses processing.

When OpenCode is disabled, the mode's instructions become the system message, preceded by a short
output-only instruction. The corrected transcript is the user message by
itself. Application metadata and XML wrappers are not added. Both cleanup
stages receive the saved vocabulary as spelling context, with instructions to
recognize plausible phonetic or spelling errors from the surrounding sentence.
Aliases are optional examples, not an exhaustive mapping. Ordinary words,
quoted text, code and numbers should be preserved. The local vocabulary block
is capped at 1,024 characters, including instructions, with whole entries only.

This is model guidance, not a spelling guarantee. In local replay probes,
SpeakoFlow ignored several name corrections despite receiving the vocabulary.
Its fine-tuning favors preserving names. A short dictation that skips OpenCode
can therefore retain a misspelled name. Deterministic transformations still run
afterward.

## Optional OpenCode pass

Enable `post_processing.enabled` in the desired mode and set its
`post_processing.prompt` to the extra instruction. OpenCode then receives
SpeakoFlow's cleaned output, along with Hex's usual app context and vocabulary.
SpeakoFlow uses its basic cleanup prompt in this combination; only OpenCode
receives the mode's custom instruction. The Output pane shows both configured steps.

Optional `post_processing.min_words` and `post_processing.min_chars` set an
inclusive size gate, evaluated after local cleanup. OpenCode runs if either
positive threshold is met. Both default to zero, preserving rewriting on every
enabled dictation. A zero threshold is ignored when the other is positive.
For example, `min_words: 150` and `min_chars: 3000` keep shorter dictations local.
Words are whitespace-separated; characters are Unicode characters, not bytes.
When local cleanup fails, the gate uses its unchanged input. History only names
OpenCode if it actually ran. Raw dictation still bypasses both steps.

The order is speech recognition, replacements, SpeakoFlow, OpenCode, then
transformations and paste. OpenCode stays off by default. It uses the running
OpenCode service, selected model and deadline. Its provider may receive the
text, depending on the chosen model. It adds a second generation to dictation
latency. If OpenCode fails or returns empty text, Hex keeps SpeakoFlow's result.
If SpeakoFlow fails, an explicitly enabled OpenCode pass can still rewrite the
corrected original. History names the failed stage. Cancellation stops the
chain before starting another model call.

## Provider quota fallback

Labs > Provider quota fallback is off by default. When enabled, a mode may set
`post_processing.quota_fallback_provider` to a second OpenCode provider ID.
An explicitly selected model is required; Auto keeps its existing ranked-model
behavior. The fallback preserves the exact model ID, variant, and prompt.
For example, a mode using `opencode-go/gpt-6-luna`, variant `none`, can set the
fallback provider to `openai`. The `openai` provider uses whichever credentials
are configured in OpenCode; use its Codex OAuth login for subscription access.

Only OpenCode's structured `ProviderRequestError` with `reason: "rate_limit"`
and the requested `providerID` triggers the fallback. OpenCode 2.2.5 maps both
quota exhaustion and rate limiting to this reason. Human-readable messages are
never inspected. Timeouts, network failures, authentication, invalid requests,
provider outages, untyped errors, and empty output keep the prior stage's text.
One alternate request is allowed, sharing the original total deadline and
cancellation flag. If that request fails, the previous text is kept. A log event
records the primary provider, fallback provider, model, and structured reason.

The protocol comes from OpenCode's
[generate handler](https://github.com/anomalyco/opencode/blob/v2.2.5/packages/server/src/handlers/generate.ts)
and [typed generation errors](https://github.com/anomalyco/opencode/blob/v2.2.5/packages/core/src/generate.ts).
An older service without these fields fails closed instead of guessing from text.

## Setup

1. Install llama.cpp with `brew install llama.cpp`.
2. Download `SpeakoFlow-Mini-0.8B-Q4_K_M.gguf` from
   [SpeakoFlow](https://huggingface.co/SpeakoFlow/speakoflow-mini).
3. Put the model in `~/Library/Application Support/hex/models/`.
4. Enable the Labs toggle, or run `hex settings set labs.local_polish true`.

The default runtime paths are `/opt/homebrew/bin/llama-server`, then
`/usr/local/bin/llama-server`, then `llama-server` on PATH. Override both paths
with `~/Library/Application Support/hex/local-polishing.json`:

```json
{
  "modelPath": "/absolute/path/SpeakoFlow-Mini-0.8B-Q4_K_M.gguf",
  "serverPath": "/opt/homebrew/bin/llama-server"
}
```

Restart Hex after changing these paths. Missing files cause a visible processing
fallback; they never enable OpenCode implicitly.

## Lifetime and failure behavior

Hex starts its own loopback-only, authenticated llama-server child when recording
begins. It does not register a LaunchAgent or connect to another app's model
server. It keeps the worker for five idle minutes, then kills and reaps it.
Disabling the feature stops an idle worker on the next five-second check.
Quitting Hex stops the worker. Loading has a fifteen-second limit; generation
has a four-second limit. Inputs over 3,000 characters keep the original text,
matching the tested input range without risking context truncation. Cancellation stops the active worker rather than
leaving it generating in the background.

Empty, truncated, or metadata-contaminated output is rejected. A guard also
rejects changed quoted text, missing numbers, and changed technical tokens.
Explicit numeric self-correction such as "12, no, 13" may remove the old number.
On a failure, that stage's input is preserved and the reason is recorded in
processing history. These guards are conservative checks, not a guarantee that
the model preserves every word or meaning.

## Local builds

Use `HEX_RELEASE_UNSIGNED=1 HEX_LOCAL_BUILD=1 bash scripts/build-app.sh` for an
ad-hoc-signed local build. The local-build flag disables upstream automatic
updates. macOS may require granting Accessibility and Microphone permissions to
this build. A public fork contains no recordings, prompts copied from personal
history, credentials, or local configuration.

## Verification

The `dictation::local_polish` tests exercise separate prompt roles, cancellation,
HTTP completion without waiting for EOF, malformed responses, and output guards.
The ignored `installed_model_cleanup_and_cancel` test exercises an installed
model using synthetic text. These tests do not prove physical microphone capture
or native paste; verify those separately in the installed app.

The `dictation::processor::rewrite` tests cover chaining order, independent
prompts, local-only and OpenCode-only modes, failed or empty OpenCode output,
local failure, cancellation between stages, inclusive size boundaries, Unicode
character counts and gating after cleanup using deterministic model doubles.
Vocabulary tests cover shared spelling instructions, optional aliases and
bounded whole-entry context. They do not establish model correction accuracy.

The `dictation::processor::generation` fixtures cover typed quota rejection over
HTTP, provider identity, unchanged prompt/model/variant, transport failure,
cancellation, shared deadlines, and at most one fallback. These fixtures do not
exhaust a live provider's allowance or verify a physical dictation.

Unit-test Labs defaults are isolated from the owner's saved settings, so test
profiles cannot accidentally start the owner's local model or cloud rewrite.
The ignored `live_quota_fallback_to_codex` check injects a synthetic primary quota
rejection and sends one synthetic prompt through the real secondary provider.
