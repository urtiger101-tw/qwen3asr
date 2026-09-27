# Third-party components

Qwen3ASR application code is MIT licensed. Third-party components retain their own licenses; this file does not relicense them.

| Component | License / provenance |
|---|---|
| audio.cpp | Apache-2.0; source commit `c7dbd4a481db6aa8f6235ace03cdccfb07961f59` from https://github.com/0xShug0/audio.cpp. Local changes are recorded in `native/patches/audio-cpp-c7dbd4a.patch`. |
| GGML, cJSON, libyaml | MIT; embedded versions from the pinned audio.cpp tree. Original copyright and license texts accompany each native engine. |
| SentencePiece and bundled libraries | Apache-2.0 / BSD / other original notices, copied from the pinned source to each native engine's `licenses` directory. |
| Rust dependencies | Exact versions in `Cargo.lock`; original notices and registry source links in `native/notices/RUST-LICENSES.md`. The `webpki-roots` MPL-2.0 source is available at its listed crate URL; no modifications are made to that crate. |
| opencc-fmmseg | MIT Rust conversion implementation, with OpenCC-derived dictionaries. Original OpenCC Apache-2.0 terms are also included in `native/notices/OpenCC-APACHE-2.0.txt`. |
| FFmpeg / ffprobe | Separate native executables, not linked into the Rust application or embedded in the public installer. The installer downloads the fixed upstream archive and verifies its SHA-256; the original license is installed with the tools. See `docs/ffmpeg-distribution.md` for provenance. |
| NVIDIA CUDA redistributables | NVIDIA CUDA Toolkit license, not MIT. Only the runtime/math DLLs needed by the CUDA engine are redistributed; the GPU driver is not included. Their license and provenance accompany the CUDA engine. See https://docs.nvidia.com/cuda/eula/index.html. |
| Microsoft Visual C++ runtime, if included | Microsoft Visual Studio redistributable terms, not MIT. |
| Qwen3-ASR and Qwen3-ForcedAligner weights | Apache-2.0 according to the model repositories. Downloaded separately, never embedded in this source repository or installer. Community Q8 GGUF conversion source: https://huggingface.co/audio-cpp/audio.cpp-gguf. |

The binary installer includes component licenses beside its native engines. `native/notices` is also distributed as `native/licenses` for the Rust executable and Chinese conversion data. Model manifests record pinned source URLs, revisions and checksums. Source builds must retain applicable notices when replacing any bundled component.
