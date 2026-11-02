# Testing

## Layers

| What | Where | Proves |
|---|---|---|
| Pure logic | `core` unit tests | Thresholds, typed quota vs timeout, merges, tombstones, eligibility, gestures, Unicode insertion, retention |
| Merge parity | `tests/fixtures/profile-sync` | Kotlin and Rust reach the same document |
| Companion | `companion/*.test.ts` | Auth, compare-and-swap, typed error passthrough, deadlines |
| Real editors | `testapp` on an emulator | Button visibility, selection replacement, Unicode, line flattening, Compose, lock screen |
| Whole pipeline | `scripts/e2e-dictation.sh` | Tap the real button, speech reaches the mic path, text lands in a field |
| Web fields | `scripts/web-fixture` in Chrome | Password, card, and phone fields hide the button; rich text keeps formatting |

## Emulator

Use KVM. On a cloud VM, turn on nested virtualization first. Without it the emulator can run in
software, but only with `ro.hw_timeout_multiplier` raised, and a boot takes 20 minutes or more.

```sh
export QEMU_AUDIO_DRV=wav QEMU_WAV_PATH=/dev/null   # qemu_in.wav in the working directory is the mic
emulator -avd hex36 -no-window -no-boot-anim -gpu swiftshader_indirect -allow-host-audio
```

**The microphone.** The emulator's WAV driver feeds `qemu_in.wav` (44.1 kHz stereo) to `AudioRecord`
through the real capture path. Make it long, because `avd rewindaudio` isn't reliable once the file
has ended. It clips a little, so transcripts are rougher than the voice deserves. This proves the
path, not a physical microphone. PulseAudio didn't work on a headless host.

**Timing means nothing for a phone.** The emulator is x86_64 with an SSE4.2 build under a VM.

**Builds.** `:app:assembleEmulator` is the release code with the debug hooks. Unshrunk Compose
installs and runs too slowly to be worth testing, and the phone runs shrunk code.

## Automation and accessibility services

A plain `UiAutomator` session suppresses every other accessibility service for as long as it's
connected. This includes agent-device's snapshot helper. The result looks exactly like the button
being broken, but Hex isn't at fault. So:

- Instrumented tests use `UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES`.
- Overlay checks use `dumpsys window` for the `HexBubble` window, plus screenshots.
- Don't use accessibility snapshots while checking the overlay.
- `executeShellCommand` has no shell. Quotes reach the command as literal characters.
- Never weaken a privacy gate to make automation work.

## Debug hooks

`app/src/debug` adds a receiver that only the shell can reach, and that doesn't exist in release builds. It sets options and asks
the real inserter to place text, which still re-checks the field.

## What emulator tests can't prove

Phone speed, memory, heat, a physical microphone, battery policy on Nothing OS, restricted settings on
a sideload, and real Gboard behavior. `acceptance.md` covers these.
