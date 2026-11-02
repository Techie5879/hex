# Real-phone acceptance

Nothing below has been verified on a phone. Emulator runs can't prove any of it. Do this once, on
the target phone, with the release APK. Your dictionary and prompts arrive through sync. The APK
doesn't contain them.

## 1. Install

1. Copy the APK to the phone and install it, or use `adb install`.
2. Open Hex and go through setup. If the Accessibility switch is greyed out, follow the App info
   step on that screen.
3. Keep the phone's existing keyboard. Hex doesn't replace it.

**Pass:** setup finishes, and Hex says "Ready to dictate".

## 2. Microphone from the background

The biggest platform risk. It worked on the Android 16 emulator image, but Android's documentation
lists no exemption here for accessibility services, so Nothing OS has to confirm it.

1. In Chrome, tap a text field, then tap the Hex button.
2. Check the status bar's microphone indicator and Hex's notification appear.
3. Tap ✓. Check both disappear soon after, and that the text lands.

**Pass:** recording starts from the button with Hex in the background. **If it fails** with "Android
blocked the microphone in the background", send the log line tagged `HexCapture`.

## 3. Choose engines

Settings › Models: download Whisper base.en Q8 and small.en Q5, optionally Parakeet TDT-CTC and
SpeakoFlow. Then run Settings › Models › Benchmark, twice, a few minutes apart. Choose the most
accurate model that stays under about 2 s for a short clip, and watch for thermal status above
Moderate.

## 4. Fields

Use a page with a textarea, a text input, a password input, and a contenteditable.

| Do | Expect |
|---|---|
| Tap the textarea | Button at the edge, above the keyboard |
| Tap the single-line input | Button stays |
| Tap the password input | Button leaves |
| Tap back | Button returns |
| Tap the contenteditable | Button shows; dictated text keeps surrounding formatting |
| Select a word, dictate | Selection is replaced |
| Type a word so Gboard underlines it, then dictate | The word stays and the text lands after it |
| Rotate with a field focused | Button stays on its edge, above the keyboard |

Repeat a dictation in Gmail, Messages, and a Compose app such as Google Keep.

## 5. Gestures

| Do | Expect |
|---|---|
| Tap | Hands-free recording; ✓ inserts, ✕ cancels |
| Hold, speak, release | Inserts |
| Hold, slide far away, release | Cancels, nothing inserted |
| Drag | Moves to the nearest edge. The microphone indicator never lights, and it never triggers Back |
| Dictate, then lock the screen | Stops, nothing is lost, History has it |

With TalkBack on (Settings › Accessibility):

| Do | Expect |
|---|---|
| Focus a text field, then explore to the button | It announces "Hex dictation" and how to use it |
| Double-tap the button | Hands-free recording starts, without a hold |
| Find and double-tap Finish, then repeat with Cancel | Each is named, and acts |
| Reach Copy and Insert on a "Text ready" result | Each is named, and acts |
| Use the button's actions menu | "Move to the other edge" works |

Turn TalkBack off when you're done, as Hex's overlays behave differently with it on.

## 6. Sync with your Mac

1. On the companion machine: enable HTTPS on your tailnet once (the Tailscale admin console, under DNS), then follow
   `companion/README.md`. `opencode service start` keeps polish available.
2. Tailscale connected on the phone. Settings › Sync and polish: enter the URLs and the token.
3. Turn on Sync. Your private dictionary and prompts appear.
4. On the Mac: Labs › Profile sync on, then `hex profile connect <url> --token-file <file>` and
   `hex profile sync`.
5. Add a word on each, sync both, and see both words on both. Delete one and see it stay deleted.
   Edit the polish prompt on both while one is offline, and see a conflict that asks you to choose.

## 7. Long-text polish

Needs OpenCode's service running on the companion machine. Dictate 150+ words and check History shows
`polish` ran. Then stop OpenCode and repeat: it should keep the local text, and name the failed step
in History.

## 8. Retention

Set recordings to Forever, dictate, play it back from History. Set text to Off: new dictations add
no text and existing ones stay. Confirm that audio and text settings act independently.

## 9. Leave it running

Use it for a day. If the button stops appearing, set Hex's battery use to Unrestricted, and note
whether Nothing OS stops accessibility services.

## Report

Send: the benchmark screen, any failed step, and logs from
`adb logcat -s HexDictation HexCapture HexA11y HexInsert`. They contain no transcripts.
