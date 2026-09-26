---
name: qwen3asr
description: Transcribe local audio or video with Qwen3ASR, create word-aligned SRT/VTT subtitles, manage local speech models, and operate the Qwen3ASR MCP server. Use for speech recognition, not speech synthesis, translation, or speaker diarization.
---

<!-- qwen3asr-managed-skill -->

# Qwen3ASR

Use the installed Rust `qwen3asr` CLI or the `qwen3asr` MCP server. Inference uses a bundled C++ / GGML worker. Python and PyTorch are not required at runtime.

## CLI workflow

1. Run `qwen3asr doctor --json` and `qwen3asr models status` when environment readiness is unknown.
2. Run `qwen3asr transcribe "ABSOLUTE_INPUT" --output-dir "ABSOLUTE_OUTPUT_DIR" --json`. The first recognition automatically downloads the default 0.6B Q8 GGUF ASR model plus Q8 ForcedAligner from the pinned audio.cpp model collection. Honor offline or download constraints; `--offline` requires an already prepared runtime and model cache.
3. Read the returned JSON and verify the artifact paths exist. A started process or download receipt is not completed transcription.

Default outputs are TXT, SRT and JSON; select formats with repeated `--format srt --format vtt --format json`. Add `--traditional` for traditional Chinese text. Use `--language zh` only when Chinese is known; otherwise preserve automatic language detection. Domain vocabulary can be supplied with `--prompt "Vocabulary: ..."`.

Existing outputs are protected. Use a new output directory by default; only pass `--overwrite` when replacing those files is authorized. Input audio/video stays local during recognition. Public model downloads require internet but do not upload the input audio.

## Model and memory choices

- Default `--device auto` prefers NVIDIA CUDA and falls back to CPU if unavailable or out of GPU memory. `--device cuda` explicitly requires GPU; `--device cpu` forces CPU.
- Default model is `0.6b-q8`, batch size is one, and audio is decoded in 25-second blocks. `--model 1.7b` offers the larger model with higher memory use. Do not launch concurrent GPU jobs; the CLI serializes recognition for one data directory.
- `--chunk-seconds 15` lowers audio working memory. Aligned mode adds one second of context on both sides of a core boundary, then assigns words back by their timestamps; decoding stays capped at 60 seconds. Fixed boundaries in text-only/segment mode may split words. `--cpu-threads 4` bounds CPU parallelism.
- ASR and alignment models run in separate passes. JSON `metrics.peak_process_rss_bytes` contains measured native worker peak working set, not total process-tree RAM or GPU VRAM. The ASR worker exits before the alignment worker loads.
- `qwen3asr models download 0.6b-q8 --with-aligner` prefetches weights. `models verify 0.6b-q8` checks downloaded model files. `config show` reveals defaults; `config set KEY VALUE` changes persistent defaults only when requested.

## Timestamp limits

`--timestamps align` produces genuine ForcedAligner timestamps for Chinese, English, Cantonese, French, German, Italian, Japanese, Korean, Portuguese, Russian and Spanish. For other ASR languages use `--timestamps none` for text only, or explicitly select `--timestamps segment` for approximate block timing and report that limitation. Never present segment timing as word alignment.

If alignment fails, inspect the reported `.asr-recovery.jsonl` file before rerunning expensive recognition. If the token limit is reached, reduce chunk size or increase `--max-new-tokens`; do not report truncated output as complete. This release has no live microphone streaming, speaker diarization or translation.

## MCP workflow

The stdio server starts with `qwen3asr mcp` and exposes `info`, `models_list`, `models_status`, `transcription_start`, `transcription_status`, and `transcription_cancel`.

- Start a job with `inputs` containing absolute local audio/video paths, optional `output_dir`, and optional `formats`.
- Save the returned `job_id`. Check `transcription_status` until succeeded, failed or cancelled; a queued/running state is not completion. Jobs are process-local to the active MCP server.
- On success inspect the returned artifact paths; on failure inspect bounded stderr and any transcript recovery path. Cancellation terminates the job process tree.
- If cancellation reports `cancellation_unconfirmed`, the queue stays paused to prevent overlapping native workers. Inspect any orphaned worker/FFmpeg processes before restarting the server; the old in-memory queue must then be submitted again. Do not report this state as successful cancellation.
- `agents install --target codex|agy|claude` installs the named local client configuration and Skill with backups; use `--dry-run` to inspect changes. Only run agent installation when the user requested the integration.

