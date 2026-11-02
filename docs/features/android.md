# Hex for Android

A separate native app in [`android/`](../../android/README.md). Design:
[`android/docs/architecture.md`](../../android/docs/architecture.md). Checks and their limits:
[`android/docs/testing.md`](../../android/docs/testing.md).

```ts
Set up                                    // android.setup
├── Disclosure -> Agree -> Microphone + Notifications -> Accessibility -> Model -> Retention -> Try it
└── Accessibility switch greyed out -> App info › ⋮ › Allow restricted settings

Focus a text field                        // android.bubble.eligibility; core FieldEligibility
├── Ordinary text, multiline, search, web, Compose -> button at the edge, above the keyboard
├── Password, visible password, PIN, data-sensitive -> no button
├── Number, phone, date -> no button
├── Payment-looking id or hint -> no button
├── Excluded app, lock screen, keyboard's own field, notification shade -> no button
└── Focus moves -> button follows; a drag moves it along either edge

Dictate                                   // android.dictation; core DictationMachine
├── Tap -> hands-free: ✓ inserts, ✕ cancels, 2 minutes of silence stops and transcribes
├── Hold -> records past 220 ms -> release inserts; slide 90 dp away -> release cancels
├── Screen off, or a sensitive field takes focus -> stops, keeps what was said, inserts nothing there
├── Under 0.3 s or silent -> says so, keeps nothing
└── Another app takes the mic -> stops, keeps what was said

Process                                   // android.pipeline; core CleanupPipeline
├── Speech model in :engine -> transcript, with the dictionary as context
├── Local cleanup -> SpeakoFlow if chosen, else filler rules; failure keeps input
├── Size gate on its output -> >=150 words or >=3000 characters
└── Polish through the companion -> 8 s, typed quota-only fallback, failure keeps local text

Insert                                    // android.insert; TextInserter
├── Same field and still safe -> commitText at the caret -> confirmed by reading back
├── No connection -> ACTION_SET_TEXT with selection, never on rich web editors
├── Focus moved -> "Text ready": Insert, Copy, or dismiss
└── Can't confirm -> "Couldn't insert": Insert, Copy, or dismiss

Recover                                   // android.recovery
├── Engine died or timed out -> Retry from saved audio, also in History
└── App killed mid-capture -> next start lists the recording in History, ready to transcribe

Keep                                      // android.retention
├── Recordings: Ogg Opus, on-device; off, day, week, month, forever (default forever)
├── Text history: the same choices, independent (default forever)
└── Fields that ask keyboards not to learn -> neither is kept

Sync                                      // android.sync; android/companion, src/profile_sync
├── Opt-in. Dictionary, prompts, and cleanup policy only
├── Offline edits stay; tombstones for deletes; prompt conflicts ask a person
└── Desktop: Labs › Profile sync, then `hex profile connect` and `hex profile sync`
```

## Observed

On an Android 16 (API 36) emulator with Gboard and Chrome, 2026-11-02: every flow above, including
real speech through the capture path, Whisper and SpeakoFlow on the device, Chrome's text, password,
card, phone, and rich fields, crash and engine-death recovery, and sync with the desktop CLI. Polish
ran against a fake provider for each outcome, and once against real OpenCode. HTTPS was a local proxy
with a test certificate, because Tailscale HTTPS needs enabling on the tailnet. Not observed: any real
phone, physical microphone, speed, heat, battery policy, or Tailscale serve. See
[`android/docs/acceptance.md`](../../android/docs/acceptance.md).
