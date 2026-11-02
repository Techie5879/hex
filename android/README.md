# Hex for Android

Dictate into any app. A small button appears at the screen's edge when a text field has focus.
Tap it to dictate hands-free, or hold it to talk and release to insert. Speech is transcribed
on the phone.

## What it does

- **A movable button** on ordinary text fields, in native, Compose, and web apps, beside whatever
  keyboard you use. It stays away from password and numeric fields, payment fields it can recognize, and the lock screen.
- **Tap, hold, cancel.** Tap records hands-free, then ✓ inserts and ✕ cancels. Hold records until
  you release. Slide away first to cancel. Dragging only moves the button.
- **Safe insertion** at the cursor, replacing any selection, the way a keyboard commits text. If
  the field moved on or can't take it, Hex offers the text to insert elsewhere or copy.
- **Local speech and cleanup.** Pick a model size that suits your phone, and measure it with the
  built-in benchmark. Local cleanup is rule-based, or uses SpeakoFlow if you download it.
- **Optional polish** for long dictations, sent directly to ChatGPT with the phone's own sign-in, at
  the same thresholds as desktop Hex.
- **Your dictionary and prompts** on phone and computer, synced free through a server you run.
- **Recordings and history** kept on the phone for as long as you choose, separately.

## Install

See [docs/acceptance.md](docs/acceptance.md) for installing on a phone, and
[companion/README.md](companion/README.md) to set up sync.

Android restricts Accessibility for apps installed outside an app store. If the switch is greyed
out, open Hex's App info, tap ⋮, and choose **Allow restricted settings**.

## How it fits together

See [docs/architecture.md](docs/architecture.md) and [docs/engines.md](docs/engines.md).
