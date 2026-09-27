# Changelog

## 0.2.1

- Publish a compiled Windows x64 Inno Setup installer through GitHub Releases.
- Default to all-users installation with system PATH; retain an explicit current-user option.
- Offer model preparation during installation, with download failures reported as installation errors.
- Download and verify the pinned FFmpeg package directly from upstream during installation instead of embedding it in the public installer.

## 0.2.0

- Replace private Python/PyTorch inference with native C++ / GGML workers controlled by the Rust CLI.
- Add pinned, SHA-256-verified Q8 GGUF ASR and ForcedAligner downloads; preserve official safetensors ASR options.
- Keep models resident across audio chunks, release ASR before alignment, and separate CPU/CUDA worker distributions for fallback.
- Port subtitle rendering, punctuation restoration and Traditional Chinese conversion to Rust.
- Preserve the single-command CLI, SRT/VTT/JSON/TXT, batch processing, MCP, Skills and Inno Setup integration.

## 0.1.0

Initial Rust CLI with a managed Python/PyTorch inference backend.
