# Speech and cleanup engines

**No figure on this page was measured on a phone.** Emulator timing proves nothing about phone speed.
For scale only, on a 16-core Xeon VM with an SSE4.2 build: base.en took about 1.3 s for 11.5 s of audio
at 280 MB peak, and small.en plus SpeakoFlow took about 16 s from cold at about 1.1 GB peak.
Settings › Models › Benchmark measures the installed models on the real device: speed, peak
memory of the engine process, and thermal status. Choose defaults from that run.

## Runtime

Hex uses [transcribe.cpp](https://github.com/anomalyco/transcribe.cpp) at the same revision as
desktop, which gives Whisper, Parakeet, and Moonshine from one API and one model format. Vocabulary
goes in through its generic `vocabulary` field, which it supports for Whisper. CPU only for now, on
arm64 with dotprod, fp16, and i8mm, all present on the Snapdragon 8+ Gen 1. ggml also has Vulkan and
Hexagon backends. Neither is enabled, because neither is proven on this hardware.

## Speech catalog

Sizes are exact downloads. Nothing ships in the APK.

| Model | Size | Vocabulary | Expectation to test |
|---|---|---|---|
| Whisper tiny.en Q8 | 46 MB | Yes | Fixed 30 s encoder window: latency barely depends on clip length |
| Whisper base.en Q8 (suggested start) | 85 MB | Yes | Same, about 2x tiny |
| Whisper small.en Q5 | 194 MB | Yes | Better on names, encoder several times slower |
| Parakeet TDT-CTC 110M Q8 | 135 MB | Not here | Scales with clip length; may suit short dictations |
| Moonshine base Q8 | 77 MB | No | Scales with clip length; may suit short dictations |
| Whisper large-v3-turbo Q5 | 620 MB | Yes | Desktop's family. Probably too slow; prove it |

Desktop runs large-v3-turbo Q8 (886 MB). Don't assume it's fast or unusable on the phone. Benchmark it.

## Local cleanup

| Mode | Size | Notes |
|---|---|---|
| Filler-word rules | None | Instant. Same rules as desktop. Removes hesitations only |
| SpeakoFlow Mini Q4_K_M | 542 MB | Same model as desktop, via llama.cpp, greedy, with vocabulary context |

Q4_K_M is the smallest published quantization, and there's no smaller validated cleanup model. A
run that passes its deadline (8 s by default, adjustable) or stops early keeps the input. Its output
passes through the same guard as desktop, so changed quotes, numbers, or identifiers are rejected.

As on desktop, cleanup can't guarantee fidelity. Its publisher reports edit accuracy of 48.8% and
content damage of 10.7% on its own hard test set. Read important text before sending it.

## Silence

Models invent text for silence, so a 1.2 MB Silero detector ships in the APK, fetched at a pinned
digest at build time. A clip with no speech says so, and types nothing. Desktop uses the same model
and settings.

## Latency budget

Each dictation waits for speech, then local cleanup, then polish if the size gate passes. Bounds:

- Speech: at least 20 s, or 8× the clip's length.
- Local cleanup: 8 s.
- Polish: 8 s, including any sign-in renewal and retry.

## What to measure on the phone

For each model: first and warm runs on the 11 s synthetic clip, peak engine memory, and thermal
status after a few minutes. Dictation should feel immediate at under about 2 s for a short clip.
