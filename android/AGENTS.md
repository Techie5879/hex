# Hex for Android: agent guide

Scoped to `android/`. The root `AGENTS.md` governs the desktop. Its Rune, goldens, and
macOS rules don't apply here. Its privacy and product rules do.

## Privacy

- This repository is public. Never commit personal dictionary entries, prompts, audio,
  screenshots of real accounts, hostnames, tokens, or logs. Tests use synthetic data only.
- The APK holds no credentials and no models. A person enters a sync token and signs in to ChatGPT
  at runtime.
- Never log transcripts, field text, or tokens.
- Never weaken a privacy gate to make automation pass. See `docs/testing.md`.

## Layout

- `core/`: Pure Kotlin, testable off-device: profile merge, sync, cleanup pipeline, field
  eligibility, insertion, gestures, and the dictation state machine. Prefer logic here.
- `app/`: Android: accessibility service, overlay, capture, engine client, Compose UI.
- `native-asr/`, `native-llm/`: JNI over transcribe.cpp and llama.cpp. Each is its own module
  because each brings its own ggml, and both load in one process.
- `testapp/`: Editors to dictate into, plus on-device tests. Never shipped.
- `../src/profile_sync/`: Portable profile and in-app desktop host. `docs/sync.md` covers pairing and migration.

## Rules that keep it correct

- Every path to the microphone goes through `DictationMachine`. Only an explicit tap or hold on
  a shown bubble starts capture. A drag never does.
- The bubble can't hide while the microphone runs.
- A field turning sensitive during capture ends capture at once.
- Insertion re-checks the field at the last moment.
- A capture stays on disk until it has become text or the person discards it.
- Each cleanup stage keeps its input on any failure.
- Merge rules must stay identical in `core` and `src/profile_sync.rs`. Change
  `tests/fixtures/profile-sync/vectors.json` first, and both suites run it.
- New models need a pinned revision and SHA-256 in `ModelCatalog`. No model in the APK or git.

## Commands

```sh
cd android
./gradlew :core:test
./gradlew :app:assembleDebug -PhexAbis=x86_64     # for the emulator
./gradlew :app:assembleRelease -PhexAbis=arm64-v8a # for the phone
cd .. && cargo test --locked --bin hex profile_sync
```

`docs/testing.md` covers the emulator, its limits, and on-device tests.
