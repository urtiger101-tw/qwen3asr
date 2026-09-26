# Native inference selection

Research was checked on 2026-09-27. Repository capabilities change; the application pins a source commit and tests the resulting Windows binaries.

| Project | Observed route | Decision for this application |
|---|---|---|
| [0xShug0/audio.cpp](https://github.com/0xShug0/audio.cpp) | C++ GGML; Qwen3-ASR, ForcedAligner, CPU and CUDA; GGUF and Hugging Face models; C API | Selected. A small persistent C++ JSONL worker allows Rust to control memory lifetime and cancellation. |
| [lumosimmo/qwen3-asr-rs](https://github.com/lumosimmo/qwen3-asr-rs) | Candle Rust ASR and ForcedAligner | Source at `7984967e8701cc90bb17d06453901036e36ca578` looks up `thinker.*` while current cached official `*-hf` tensors use `model.*`; no remap was found. Windows CUDA was not established; offline CLI does not supply the required aligned SRT workflow. No live inference claim is made for this candidate. |
| [second-state/qwen3_asr_rs](https://github.com/second-state/qwen3_asr_rs) | Rust / LibTorch and MLX | Documented CUDA binaries target Linux, and no ForcedAligner path was found in the assessed README. Windows integration would need additional work. |
| [huanglizhuo/QwenASR](https://github.com/huanglizhuo/QwenASR) | Native Rust ASR and alignment | CPU-only scope did not satisfy NVIDIA-first operation. |
| [eclipse005/qwen-aligner-rs](https://github.com/eclipse005/qwen-aligner-rs) | Rust CPU/CUDA aligner | Alignment only; still needs a separate ASR engine. |
| [predict-woo/qwen3-asr.cpp](https://github.com/predict-woo/qwen3-asr.cpp) | Native C++ inference, Apple-oriented acceleration | Less direct fit to the required Windows CUDA + ForcedAligner package. |

These are task-specific implementation findings, not quality rankings. The Rust alternatives were assessed from source and documentation; they were not all built or benchmarked.

## Integration

The shipped engine uses audio.cpp commit `c7dbd4a481db6aa8f6235ace03cdccfb07961f59`, with a checked-in patch applied by `scripts/build-native.ps1`. The CLI, downloads, settings, media slicing, Traditional Chinese conversion, subtitles, MCP server and client integration are Rust. Inference and forced alignment are C++ / GGML. FFmpeg is a separate native media decoder. No Python or PyTorch process participates in inference.

One ASR worker remains alive for the complete recognition pass and then exits. A separate aligner worker starts afterward. This avoids both repeated model loads per chunk and simultaneous ASR/aligner model residency. JSONL responses and diagnostic capture have size limits; cancellation terminates the worker tree. CPU and CUDA distributions are separate so missing CUDA DLLs cannot prevent CPU fallback.

## Models

Default Q8 GGUF files are published by the audio.cpp community, derived from Qwen models. The catalog pins repository `audio-cpp/audio.cpp-gguf` revision `0a104324546d2622985e3c676a4b5550cc772127` and verifies exact byte counts and SHA-256. Official safetensors ASR aliases remain available. Weights are downloaded by the CLI, not embedded in the installer or repository.

Compatibility and actual runtime measurements belong in the release validation report. Unit tests alone do not establish Windows CUDA correctness or transcription quality.
