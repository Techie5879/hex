# Hex

Hex is local voice dictation for Mac, Windows and Linux. Hold a key, speak, and
it pastes.

## How it works

- hold the shortcut, speak, then let go
- Hex transcribes on your device and pastes into whatever app you're in
- speech transcription runs on-device; optional OpenCode rewriting sends text to your configured provider
- experimental features live in Labs, at the bottom of the sidebar

## Downloads

Stable is the current release. Dev is the newest build of `dev` and updates
itself from the dev channel.

| Platform | Stable | Dev |
| --- | --- | --- |
| macOS (Apple silicon) | [DMG](https://anoma.ly/hex/download/macos-arm64-dmg) | [DMG](https://anoma.ly/hex/download/dev/macos-arm64-dmg) |
| Windows x64 | [Installer](https://anoma.ly/hex/download/windows-x64-setup) | [Installer](https://anoma.ly/hex/download/dev/windows-x64-setup) |
| Windows ARM64 | [Installer](https://anoma.ly/hex/download/windows-arm64-setup) | [Installer](https://anoma.ly/hex/download/dev/windows-arm64-setup) |
| Linux x86_64 | [Executable](https://anoma.ly/hex/download/linux-x64) | [Executable](https://anoma.ly/hex/download/dev/linux-x64) |
| Linux x86_64 (Debian, Ubuntu) | [.deb](https://anoma.ly/hex/download/linux-x64-deb) | [.deb](https://anoma.ly/hex/download/dev/linux-x64-deb) |
| Linux x86_64 (Fedora, openSUSE) | [.rpm](https://anoma.ly/hex/download/linux-x64-rpm) | [.rpm](https://anoma.ly/hex/download/dev/linux-x64-rpm) |
| Linux ARM64 | [Executable](https://anoma.ly/hex/download/linux-arm64) | [Executable](https://anoma.ly/hex/download/dev/linux-arm64) |
| Linux ARM64 (Debian, Ubuntu) | [.deb](https://anoma.ly/hex/download/linux-arm64-deb) | [.deb](https://anoma.ly/hex/download/dev/linux-arm64-deb) |
| Linux ARM64 (Fedora, openSUSE) | [.rpm](https://anoma.ly/hex/download/linux-arm64-rpm) | [.rpm](https://anoma.ly/hex/download/dev/linux-arm64-rpm) |

Or install with:

- Homebrew: `brew install --cask anomalyco/tap/hex`
- AUR: `yay -S hex-bin`
- install script (Linux): `curl -fsSL https://anoma.ly/hex/install-linux.sh | sh`
- Nix: `nix profile add github:anomalyco/hex`

More docs are in [`docs/`](docs/README.md).

## Contributing

We don't take pull requests. [Open an issue](https://github.com/anomalyco/hex/issues)
describing the bug or idea, and our agents usually turn it around fast.

MIT licensed.


## This fork

Owner-built local releases keep upstream automatic updates off. The fork adds
[local SpeakoFlow cleanup, optional size-gated Luna rewriting, and typed quota-only
provider fallback](docs/local-cleanup.md). Your configured speech model and vocabulary
still apply after upstream updates.

## Privacy and local data

Dictation audio is not retained by default. On macOS, Labs > Save dictation audio
can keep new clips as local Ogg/Opus recordings using ffmpeg. History's separate
Audio menu controls their retention and defaults to Forever. The archive lives
in `~/Library/Application Support/hex/dictation-audio/`; turning saving off stops
new files without deleting existing ones. Audio retention is independent of
transcript history, and no legacy audio is recovered. See the
[recording format and retention behavior](docs/features/dictation.md#compressed-dictation-audio).

Local logs can contain transcript text and application context. Review them
before sharing. OpenCode rewriting sends its documented text context to the
selected provider; ordinary speech recognition stays local.
