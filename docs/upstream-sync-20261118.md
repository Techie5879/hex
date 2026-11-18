# Upstream merge, November 18, 2026

This sync merges upstream history into the fork. Base: `81d0d9f386104f9e1b6b95d8b79aa6bf22765cbb`; upstream: `52811a7f9b34d8b66c98b734e35b04fb923847f6`; merge base: `2595bc568580f2a0858c63ab9d56ec86f6936c9b`. The earlier cherry-picks remain in history. This change records upstream as the second parent of one real merge commit.

Upstream brings grouped Labs cards and discovery, settings search and transactional saves, additional shortcut and dictation experiments, Apple Intelligence cleanup and voice prompts, common-media CLI transcription, expanded vocabulary behavior, History multiselect and statistics, meeting imports and export/retention improvements, call switching/recovery, a revised meeting pill, screenshot editing/recording, Captures, terminal routing, platform audio/input/paste fixes, and newer Rune rendering/accessibility fixes.

The owner's latest instruction governs presentation: use upstream's UI, groups, layout, and wording; preserve fork behavior and saved settings. No Hex app build, install, launch, live-device test, or change to `/Applications/Hex.app` was performed. Compiler/test builds below are the requested checks.

## Resolution decisions

- `src/labs.rs`, `src/labs/{feature,catalog,card,description,group,gates}.rs`, `src/rune_ui/labs.rs`: upstream split registry, card layout, filtering and groups replace the fork's flat list. Six fork-owned labs join that registry. A separate packed state in `src/labs/fork.rs` avoids colliding with upstream's nearly full runtime bit field. Policy locks, OS availability, CLI switching, all-off defaults and snapshot shutdown apply to these entries too.
- `src/app_settings{.rs,/labs.rs}`, `src/control/settings{.rs,/registry.rs}`: keep upstream's settings registry split and transactional persistence; retain the fork's compressed-audio retention, local cleanup, provider fallback and profile fields. The old `labs.stop_when_call_ends` key remains writable and persisted as an alias of the Meetings setting. Its default remains off. A present original key remains authoritative for old writers; the new UI/CLI transaction keeps both keys in sync. No old enabled value is discarded.
- `src/macos_rune{.rs,/runtime.rs,/request.rs,/profile.rs}` and `src/rune_ui/profile_sync.rs`: upstream runtime/request split and settings revisions coexist with the fork's in-process profile server. Profile exchanges and UI rebases save through the new base-aware update transaction; the phone does not get the control API token. Android and the profile CLI remain intact.
- `src/rune_ui{.rs,/sidebar.rs,/scenes.rs,/playground.rs,/deep_link.rs}`: use upstream navigation, footer, Labs search/deep links and fixture organization. Preserve the shared-room and snapshot scenes. Fork-only local footer placement is intentionally superseded, per the owner's clarification.
- `src/dictation/processor{.rs,/generation.rs,/rewrite.rs}`, `src/app_settings/post_processing.rs`: preserve upstream speech/typed-text separation, corrections, voice prompts, nearby-text context, model observations and deadline-bounded service discovery. Normal speech can still run local SpeakoFlow cleanup before an optional OpenCode rewrite. Fork word/character thresholds use the cleaned text. A failed cloud step keeps successful local cleanup. If both on-device cleanup switches are on, the fork’s SpeakoFlow cleanup takes precedence. Typed provider quota failures allow exactly one same-model alternate provider within the same deadline; transport errors and timeouts do not.
- `src/dictation/processor/rewrite.rs`: upstream model reporting is applied to the alternate provider that actually answered. The existing local cleanup and threshold tests remain; voice-prompt routing keeps upstream's precedence.
- `src/transcription/{engine.rs,engine/streaming.rs,engine/preview.rs,engine/timing.rs,gguf_session.rs,speech_detector.rs}`: retain fork Whisper rolling previews, interactive Metal scheduling, cancellation scopes and coarse speech bounds; combine with upstream ignored start-tone/pre-roll coverage, offline fallback and native timestamp APIs. Warm-up still omits vocabulary. Small helpers were split to meet file-size caps.
- `src/parakeet.rs`: keep the fork's compressed audio capture path and upstream return-to-app/ordered output. Upstream removed dead cancellation of already-accepted dictations, so accepted output continues through processing; local cleanup's own cancellation interface remains.
- `src/history{.rs,/...}`, `src/rune_ui/history.rs`: upstream History multiselect, playback, raw-WAV retention and statistics coexist with the fork's independent compressed Ogg/Opus archive. The compressed archive keeps `audio_retention=forever` by default; its choice is on the Save dictation audio card, using upstream's own-settings-on-card convention.
- `src/meeting{.rs,/entry.rs,/mode.rs,/transcript.rs,/transcript/*,/speakers/*}`: preserve per-recording Standard/SharedRoom mode, conservative cross-track deduplication, named speakers, recording renames and source-aware transcript copying. Combine with upstream microphone leveling, native timing, imported recordings, separate notes files and audio-only retention. Imported files still use Speaker; live recorded calls use names or the correct capture-path label. Speaker observations are finalized before publication.
- `src/meeting/{watcher.rs,watcher/call.rs,watcher/call_switch.rs,detection.rs,runtime.rs}`: keep upstream offer identities, browser-call switching, auto-stop withdrawal, restart recovery and calendar menu. Preserve the fork's distinction between Automatic, AcceptedOffer and Requested recording origins: accepted Teams/Slack offers may stop when their own call ends, while manual/calendar recordings do not. Only Automatic says it started automatically. Synthetic accepted-offer tests were ported to the new controller interface.
- `src/rune_ui/{meeting_capsules.rs,meeting_capsules/panel.rs,meetings.rs,meetings/fixture.rs,meetings/room_mode.rs,meetings_host.rs,meeting_snapshots.rs}`: use upstream's new pill and import/reveal UI, while retaining snapshot status and controls, background snapshot scans, shared-room selection and named transcript labels. Upstream's single CallEndedWithdrawn event replaces the fork's older cancellation event, and Keep recording restores the pill without hiding a newer panel.
- `src/meeting/notes.rs` and `src/meeting/mcp.rs`: combine fork speaker/shared-room prompt semantics with upstream transcript-language instructions. Remove personal author assumptions from the MCP description, while keeping speaker names and room-source explanations.
- `src/accessibility.rs`: keep the guard against querying Hex's own AX tree, the fork's invalid-PID guard and speaker-reader support, together with upstream caret context. The newer Rune revision also fixes off-main AccessKit access.
- `src/rune_ui/live_text.rs`: retain the fork's unified stable/tentative layout and fading overflow while adding upstream failure notices and overlay recovery. It remains click-through.
- `src/local_api{.rs,/http.rs}`: retain profile exchange's conditional ETags and duplicate-header rejection alongside upstream CORS, API processing and timestamp endpoints. Share only the required HTTP helpers with the profile listener.
- `crates/router/src/routing/{look,policy,smart}.rs`, `src/capture_host/routing{.rs,/preparation.rs}`: use upstream terminal-pane routing and the later upstream rule that publisher-named sessions are suggestions. This supersedes the earlier cherry-picked auto-send treatment; user-saved rules still control automatic routing.
- `src/{audio,feedback,linux,windows,...}`: use upstream fixes where the fork had only cherry-picked older revisions. Remove duplicate declarations left by overlapping cherry-picks rather than retaining two copies of the same implementation. Keep fork test isolation for custom models so the checks do not load the owner's imports.
- `src/desktop/i18n{.rs,/...}`: combine upstream split translation tables with fork speaker, room and snapshot translations; add translations for the ported lab card strings. Move the fork's table out of the capped root file. No translation key is intentionally dropped.
- `AGENTS.md`, `docs/features/*`, `docs/cli.md`, `docs/releasing.md`, `README.md`: preserve fork policy and feature documentation while describing upstream entry points. Update the Labs integration note and link this record. Historical native evidence is retained as historical evidence, not claimed as a new native run.
- `.github/workflows/*`, `.github/release/plan.sh`: retain the manual-only desktop/full-check policy and the fork's production release boundary. Take upstream workflow fixes without restoring automatic desktop triggers. SDK publishing is now explicit dispatch from production; dev pushes only maintain its version PR.

Upstream graduations remain: platform icons, movable capsules, subtitles, follow-live transcript, vocabulary in notes, finished events and summary selection follow upstream's ordinary behavior/Settings. Upstream's retired collapsible-meeting-sections lab is removed. Those were inherited upstream labs, not the six fork-owned additions. Stop-when-call-ends retains its fork extension and old key as described above.

## Fork Labs mapping

| Fork feature | Upstream group/home | Existing key | Default |
| --- | --- | --- | --- |
| Local SpeakoFlow cleanup | Output | `labs.local_polish` | false |
| Provider quota fallback | Output | `labs.provider_fallback` | false |
| Save dictation audio | History, retention control on its card | `labs.save_dictation_audio`, `audio_retention` | false; forever |
| Profile sync | Dictation | `labs.profile_sync` | false |
| Meeting snapshots | Meetings | `labs.meeting_snapshots` | false |
| Meeting speaker names | Meetings | `labs.meeting_speaker_names` | false |
| Stop when the call ends | Meetings settings | `labs.stop_when_call_ends` compatibility alias; `meetings.stop_when_call_ends` | false |
| Shared room | Active meeting detail | recording manifest `mode` | standard |

Per-mode `post_processing.quota_fallback_provider`, `min_words` and `min_chars` retain their names and defaults of None, 0 and 0. The full fork-side commit inventory is [here](upstream-sync-20261118-fork-commits.tsv).

## Dependencies

Rune uses upstream's `eb5ae6c9f77b1a0051d8e1e5ee936ebea086bb90`, including its winit, Parley, wgpu-hal and AccessKit patches, with the matching upstream Nix hash. This includes the covered-window 100% CPU and off-main accessibility fixes.

Neither transcribe.cpp history was a superset. With the owner's authorization, new branch [`hex-combined-20261118`](https://github.com/Techie5879/transcribe.cpp/tree/hex-combined-20261118) starts at the fork's existing pin `25331db7d459d712c751421ae87ef8d9648736f7`, retains its Metal scheduling commits `4d12275` and `25331db7`, and cherry-picks exactly:

- `fe3df30` -> `36789e8`: force exact sounds-like spelling matches.
- `33b36a4` -> `17e3364`: apply exact spellings after decode without scores.
- `bcb639f` -> `c760023`: keep longer exact spellings intact.

No conflicts or hand-rewrites; no existing dependency branch or tag moved. Hex pins resulting SHA `c76002365904bfab993bb3e2f589d7a49632719e`, not a branch. The Nix source hash is `sha256-yZcKJjOrjg+y7svGzIEpIXVKpH/sm9DcQz6jMhVRxNM=`. The NAR encoder was checked against the existing upstream source hash before deriving this one. Nix itself is unavailable on this Mac, so an actual Nix build remains unverified.

The lockfile was updated minimally for Rune and transcribe.cpp. Cargo's Rune update initially reselected gpu-allocator's Windows dependency as 0.61.3. Restore upstream's intentional 0.62.2 selection, preserving the older Windows version only where a different dependency requires it. The minimal dependency update also selected the already-locked getrandom 0.3.4 for tempfile; no unrelated package version was bumped. Locked compiler/test checks verify the resulting resolution.

Dependency verification: Release CMake build with Metal and CPU completed; `ctest --test-dir build --output-on-failure --parallel 6` ran 63 tests, 62 passed and the model-dependent diarization test skipped, 0 failed. The Metal scheduling and Parakeet vocabulary unit tests passed. `scripts/ci/clang-format.sh --check` reports formatting violations in the imported upstream Parakeet files; they remain verbatim as requested. No real-model accuracy run was performed.

## Golden placeholders

127 conflicts: 126 upstream PNG placeholders and one upstream deletion, `tests/golden/rune-meetings-platform-icons-light.png`. Every retained placeholder reflects upstream UI, not a validated render of this merge. The [complete path and side list](upstream-sync-20261118-goldens.txt) is reproduced in the PR. Linux must regenerate and inspect them, including the fork-only and new scenes. GitHub Actions was not dispatched.

## Workflow trigger audit

- `.github/workflows/agent-policy.yml`: `on:` unchanged from origin/dev.
- `.github/workflows/aur-check.yml`: `on:` unchanged from origin/dev.
- `.github/workflows/build-desktop.yml`: `on:` unchanged from origin/dev.
- `.github/workflows/check-linux.yml`: `on:` unchanged from origin/dev.
- `.github/workflows/check-macos.yml`: `on:` unchanged from origin/dev.
- `.github/workflows/check-windows.yml`: `on:` unchanged from origin/dev.
- `.github/workflows/compile.yml`: `on:` unchanged from origin/dev.
- `.github/workflows/deploy.yml`: `on:` unchanged from origin/dev.
- `.github/workflows/release-typescript.yml`: SDK push trigger narrows from dev/production to dev; explicit production dispatch publishes.
- `.github/workflows/release.yml`: still dispatch-only; only the channel input description changed inside `on:`.
- `.github/workflows/test-builds.yml`: `on:` unchanged from origin/dev.
- `.github/workflows/windows-installer.yml`: `on:` unchanged from origin/dev.
- `.github/workflows/windows-installers.yml`: `on:` unchanged from origin/dev.

Desktop Compile, Test builds, Release, Check Linux, Check macOS and Check Windows remain manual/reusable only. The small PR policy check remains automatic. Production release validation gates remain in place.

## Verification

- `cargo fmt --all --check`: passed.
- `cargo clippy --locked --bin hex --all-targets -- -D warnings`: passed.
- `scripts/check`: passed. Formatting, code shape, Nix pin tests and full clippy passed; nextest ran 3,499 tests across six binaries, all passed, 35 skipped. Linux-only golden tests do not run on macOS.
- `HEX_UPDATE_LABS_JSON=1 scripts/check labs::web`: passed, 2 tests; regenerated the six added public Labs entries in `site/src/labs.json`. The full suite subsequently verified it without the update flag.
- `git diff --check`: passed; all 127 conflicted golden paths match upstream's bytes/deletion.
- `cargo check --locked --bin hex --target x86_64-unknown-linux-gnu`: not run, target is not installed.
- `cargo check --locked --bin hex --target x86_64-pc-windows-msvc`: not run, target is not installed.
- The full test build reported nonfatal linker unwind-table size warnings and Cargo's existing `scrawl-capture` unused `core-graphics-types` manifest warning. Rust clippy checks passed with `-D warnings`.

Earlier runs found and resolved duplicate declarations from overlapping cherry-picks, renamed/split API call sites, the legacy alias serialization/default mismatch, a stale generated Labs catalog, the shared quiet-onset threshold, and a missing playground path. The passing run includes those fixes.

Native macOS interaction, live microphone/call capture, installed-app behavior, Linux/Windows execution and Linux golden inspection were not performed. Automated tests and compilation are evidence for their covered paths, not native end-to-end proof.
