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
└── Polish, direct with the phone's ChatGPT sign-in -> 8 s, no fallback, failure keeps local text

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

Sync                                      // android.sync; src/profile_sync
├── Opt-in. Dictionary, prompts, and cleanup policy only
├── Offline edits stay; tombstones for deletes; prompt conflicts ask a person
├── Mac: profile-only loopback listener inside Hex while running, Labs on, and a device paired
├── Phone: sync after edits, on opening, manually, and approximately hourly; offline edits retry later
└── Pair: `hex profile device add`; migration and optional WebDAV: android/docs/sync.md
```

## Observed

On an Android 16 (API 36) emulator with Gboard and Chrome, 2026-11-02: real speech through the capture
path, Whisper and SpeakoFlow on the device, tap, hold, cancel, and drag, Chrome's text, password, card,
phone, and rich fields, crash and engine-death recovery, retention and expiry, and sync with the
desktop CLI. Sync's HTTPS
was a local proxy with a test certificate, because Tailscale HTTPS needs enabling on the tailnet. Not
observed: any real phone, physical microphone, speed, heat, battery policy, Tailscale serve, or
TalkBack. See
[`android/docs/acceptance.md`](../../android/docs/acceptance.md).

Integrated-host update, 2026-11-02: the Linux desktop suite passed 1,536 tests
with 22 opt-in tests ignored; Android core passed 85 tests. The rebuilt app opened
on the Android 16 emulator, whose scheduler reported a one-hour maintenance job.
The host's authentication, conditional writes, interrupted apply, migration, and
editor-race regressions passed. Native Mac hosting and migration remain unverified:
the linked Mac was unreachable and the fork's GitHub Actions was disabled. The
installed Mac companion was therefore not changed.
