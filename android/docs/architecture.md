# Architecture

```ts
Accessibility service      // focus facts, overlay, insertion
├── FieldEligibility       // core: fails closed
├── OverlayController      // TYPE_ACCESSIBILITY_OVERLAY, not focusable
│   └── BubbleView + GestureClassifier   // tap, hold, drag decided before any mic
└── TextInserter           // input connection, then ACTION_SET_TEXT, then offer to copy

DictationCoordinator       // one dictation, gesture to text
├── DictationMachine       // core: every state and effect
├── CaptureForegroundService + AudioCapture   // PCM straight to disk
├── AudioArchive           // Ogg Opus via MediaCodec, on-device only
├── EngineClient ──binder──> EngineService (:engine process)
│                            ├── NativeAsr (transcribe.cpp)
│                            └── NativeLlm (llama.cpp, SpeakoFlow)
└── CleanupPipeline        // core: local, then size gate, then polish
    └── CodexPolisher ──HTTPS──> ChatGPT Codex API, with this phone's own sign-in

ProfileRepository ←→ ProfileSync ──HTTPS, ETag/If-Match──> companion or WebDAV
```

## Decisions

**Accessibility service plus overlay, not a keyboard.** Replacing the keyboard just to get a button
is the thing to avoid. An accessibility service is the supported way to see which field has focus
and to draw over other apps without the overlay permission. Android 13's `flagInputMethodEditor`
gives the service an `AccessibilityInputConnection` beside the real keyboard, so it can commit text
like one, and read the `EditorInfo` a keyboard would. It's the reason `minSdk` is 33.

**Reading focus.** Ask the active window's root, not the service. The service-wide search stops at
Chrome's outer view, which hides the web field's password flag, id, and placeholder. Chrome's tree
can also lag behind its editor, so the button waits about a second before trusting the editor alone.

**Why not `ACTION_SET_TEXT` first.** It replaces a field's whole content. That loses rich-text
formatting, breaks undo, and races the keyboard's composing text. `commitText` has none of these
problems, so `ACTION_SET_TEXT` is only for fields without a connection, and never for rich web
editors.

**Inference in its own process.** ggml aborts on some failures, and a model can trigger the
low-memory killer. In `:engine`, either surfaces as a dead binder, not a dead button. The capture is
on disk, so Retry works. A `SIGILL` on a CPU without i8mm stays contained the same way.

**Capture to disk first.** PCM goes straight to a file, synced each second. A crash loses at most a
second, and the next start lists the audio in History, ready to transcribe.

**Microphone rules.** The foreground service exists for the length of one dictation, and starts
from the service's handling of a touch on its own overlay. Android's docs list no accessibility
exemption from while-in-use restrictions. It works on the Android 16 emulator image, and the phone
must still confirm it. If Android refuses, Hex says so and doesn't pretend to record.

**Edge gestures.** With gesture navigation, a swipe from the screen's edge is Back. The button
excludes its own rectangle, or dragging it would leave the app.

**Secure fields.** Password and sensitive-data signals from the node and from `EditorInfo` each hide
the button, as do number, phone, and date fields. Payment forms often use plain text inputs, so
identifiers are checked as a further reason to hide, and a person can exclude any app.

**Sync.** Devices merge, and the server only compares and swaps one document, so a small companion
and any WebDAV server work alike. See `companion/README.md` for what was rejected and why.

**Merging.** Three-way against the last agreed document. Concurrent spelling edits union, deletes
leave tombstones and win or lose by stamp, and a concurrent prompt edit keeps the newest and records
the other as an open conflict for a person to settle, so nothing silently disappears.

**Polish.** The phone calls the Codex API directly, with a ChatGPT login made on the phone through a
device code. The APK holds no credential. The login is the phone's own session because refresh tokens
rotate: copying one from another machine would let either device sign the other out. Tokens are
wrapped by a Keystore key, and a refresh finishes and saves on its own thread even if the polish that
asked for it has given up. A 401 renews the login and retries once inside the deadline. Anything else,
including a 429, keeps the local text. With one provider there's nothing to fall back to, so the
profile's fallback setting is left for desktop. Statuses decide this, and error messages aren't read.
Only `response.completed` makes a reply usable. This API isn't documented for third-party apps, so it
may change; if it does, polish fails and the local text stays.

## Known limits

- A payment field with a plain type and an uninformative id and placeholder can't be told from any
  other text field. Android doesn't expose HTML autocomplete to accessibility services. Hex then shows
  its button, and records only if you press it. Exclude such apps.
- A device whose clock runs far ahead stamps its edits far ahead, and they win conflicts until the
  other device catches up. A conflict still records the other value.
- `FLAG_SECURE` windows can't be detected without screenshots, which Hex doesn't take.
- Android can stop an accessibility service, or block one in Advanced Protection mode.
