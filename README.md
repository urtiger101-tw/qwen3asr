# Qwen3ASR

Windows 本機語音辨識 CLI，以 **Rust 主程式 + C++ / GGML 推論**實作。模型使用 Qwen3-ASR 和 Qwen3-ForcedAligner，原生引擎基於固定版本的 [audio.cpp](https://github.com/0xShug0/audio.cpp)。**執行辨識不需要 Python、PyTorch、pip 或雲端 API。**

## 安裝與開始

下載 [Windows 已編譯安裝版](https://github.com/urtiger101-tw/qwen3asr/releases/latest)，執行 `qwen3asr-0.2.1-setup.exe`。不需要安裝 Rust、C++、Python 或 CUDA Toolkit。

安裝器預設選擇「所有使用者」，經 Windows UAC 確認後安裝到 Program Files 並加入**系統 PATH**。保留「下載模型」即可在安裝期間準備辨識與對齊模型；只安裝給目前使用者時可切換安裝範圍，使用使用者 PATH。完成後開啟新的終端機。

```powershell
qwen3asr recording.mp3
qwen3asr meeting.mp4 --language zh --traditional
qwen3asr doctor --json
```

原生 CPU/CUDA 引擎已編譯並包含在安裝包。安裝期間從固定上游下載 FFmpeg／FFprobe（約 171 MB），驗證 SHA-256 後安裝；模型下載工作另外準備 `0.6b-q8` ASR 與 `aligner-q8`（合計約 2.3 GB），也驗證固定版本與 SHA-256。若取消勾選模型下載，第一次辨識會自動補下載。預留至少 5 GB 可用磁碟空間，另加輸出與既有模型快取。辨識時音訊留在本機，準備完成後支援離線使用。

預設輸出原檔旁的 UTF-8 TXT、帶精準時間軸的 SRT、JSON。JSON 包含逐字時間、字幕、實際裝置、量測記憶體、警告與檔案路徑。既有輸出受到保護，覆寫必須加 `--overwrite`。

```powershell
# 預先下載，以後離線使用
qwen3asr setup --download-model
qwen3asr recording.wav --offline --output-dir .\subtitles

# 大模型 / 官方未量化 safetensors
qwen3asr recording.wav --model 1.7b
qwen3asr recording.wav --model 0.6b

# 影片與多種字幕格式
qwen3asr meeting.mp4 --format srt --format vtt --format json

# CPU、較短區塊與領域詞彙
qwen3asr recording.wav --device cpu --chunk-seconds 15 --cpu-threads 4 --prompt "Vocabulary: Qwen, NVIDIA."

# 逐檔批次，失敗時仍繼續其他檔案
qwen3asr transcribe first.wav second.mp3 --output-dir .\results --continue-on-error --json
```

## 模型與設定

| 名稱 | 來源與格式 | 用途 |
|---|---|---|
| `0.6b-q8`（預設） | audio.cpp 發布的 Qwen3-ASR Q8 GGUF | 較小檔案、原生 CPU/CUDA 辨識 |
| `0.6b` / `1.7b` | Qwen 官方 `*-hf` safetensors | 未量化 ASR；較大模型需要更多記憶體 |
| `aligner-q8` | audio.cpp 發布的 Qwen3-ForcedAligner Q8 GGUF | 預設逐字時間對齊 |
| `aligner` | Qwen 官方 `*-hf` safetensors | 保留官方模型下載／校驗；預設對齊使用 GGUF |

社群 GGUF 轉換來源、檔案大小、固定 commit 與 SHA-256 都記錄在下載 manifest；它們不是 Qwen 官方直接發布的 GGUF。`--model` 可指定相容本機 GGUF 或官方 `*-hf` 資料夾。

```powershell
qwen3asr models list
qwen3asr models download 0.6b-q8 --with-aligner
qwen3asr models verify 0.6b-q8
qwen3asr models status
qwen3asr config show
qwen3asr config set model 1.7b
qwen3asr config set device auto
qwen3asr config set cache_dir D:\ASR-models
qwen3asr config set traditional true
qwen3asr config reset
qwen3asr languages
```

`config set` 儲存之後命令的預設，單次命令選項優先。資料預設位於 `%LOCALAPPDATA%\Qwen3ASR`；`QWEN3ASR_HOME` 可更改位置。模型支援續傳、固定 revision、大小與 SHA-256 校驗；不執行模型中的遠端程式碼。升級保留既有設定與模型，舊設定 `model=0.6b` 仍使用原官方模型；要改用新預設可執行 `config set model 0.6b-q8`。

## 記憶體與時間軸

- `--device auto` 預設使用 NVIDIA CUDA；CUDA 不可用、載入失敗或推論失敗時，改用獨立 CPU 引擎。`--device cuda` 嚴格要求 CUDA，失敗即回報。本機安裝包的 CUDA 核心要求 compute capability 8.6+（例如 RTX 30 系列），含 PTX；較舊 GPU 使用 CPU，亦可從原始碼指定其他架構重建。NVIDIA 顯示驅動仍須由系統提供。
- 每次只解碼一段 mono 16 kHz 音訊。預設核心區塊 25 秒，精準對齊模式在交界各增加 1 秒上下文，每段仍最多 60 秒。
- ASR 模型載入一次、依序辨識所有區塊；退出 ASR worker 釋放模型後，才載入對齊模型。兩個模型不會同時駐留。
- 同一資料目錄的辨識工作互斥，MCP 自動排隊。字幕文字與時間資料隨音檔長度增加，但原始音訊不會整段讀入 RAM。
- JSON `metrics.peak_process_rss_bytes` 是原生 worker 的實測最高 working set；不是整個系統或 GPU VRAM 上限。無量測值時為 null，不虛構 GPU 用量。
- ForcedAligner 支援 Chinese、English、Cantonese、French、German、Italian、Japanese、Korean、Portuguese、Russian、Spanish。其餘 ASR 語言請選 `--timestamps none`，或明確選 `--timestamps segment` 使用附警告的區塊級近似時間。
- `none`／`segment` 的固定邊界可能切字；`align` 用重疊上下文和逐字時間分配減少重複。字幕仍需人工校對。這版沒有說話人分離、翻譯或即時麥克風串流。
- 對齊失敗或遺漏辨識字詞時保留 `.asr-recovery.jsonl`，不交付截短字幕。達 token 上限明確報錯；可縮短區塊或增加 `--max-new-tokens`。
- 每個輸出檔採原子替換，一般寫入失敗會回復同批已寫入檔案。強制終止或斷電不保證整批檔案的一致性；使用新的輸出目錄可保留上一批成果。

## Agent、Skill 與 MCP

目前使用者安裝可選 Codex、AGY、Claude 整合；所有使用者安裝後，各帳戶在自己的終端機使用同一支 CLI 設定：

```powershell
qwen3asr agents install --target codex --dry-run
qwen3asr agents install --target codex
qwen3asr agents install --target agy
qwen3asr agents install --target claude
qwen3asr mcp
```

MCP 使用 stdio，不開網路埠，提供查詢環境／模型、啟動辨識、查狀態與取消。整合只修改具名設定並保留備份，隨附操作 Skill。詳見 [MCP 文件](docs/agents.md)。

## 建置、驗證與授權

```powershell
# Windows x64：Rust、Git、CMake、Ninja、Visual Studio C++；CUDA build 另需 CUDA Toolkit
pwsh -NoProfile -File scripts/build-native.ps1 -Backend both
pwsh -NoProfile -File scripts/prepare-ffmpeg.ps1
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
.\target\release\qwen3asr.exe doctor --json
pwsh -NoProfile -File scripts/package.ps1
```

原生來源固定到 audio.cpp commit `c7dbd4a481db6aa8f6235ace03cdccfb07961f59`，所需相容性修正以 patch 隨源碼提供。CPU/CUDA worker 與 DLL 分開封裝，避免沒有 CUDA 的電腦無法啟動 CPU 路徑。FFmpeg 準備、建置選項與 Inno Setup 見 [安裝文件](docs/installation.md)。MCP SDK 驗收腳本使用 Python 作為開發測試工具；發行版推論不依賴它。

本專案 Rust／worker 程式碼採 MIT；audio.cpp、GGML、模型、FFmpeg 與 CUDA 元件保有各自授權，詳見 [第三方聲明](THIRD_PARTY_NOTICES.md)。安裝包未經程式碼簽章。原生引擎選型與相容性研究見 [研究紀錄](docs/native-research.md)。

模型來源：[Qwen3-ASR 官方集合](https://huggingface.co/collections/Qwen/qwen3-asr)、[官方 ASR](https://huggingface.co/Qwen/Qwen3-ASR-0.6B-hf)、[官方 ForcedAligner](https://huggingface.co/Qwen/Qwen3-ForcedAligner-0.6B-hf)、[audio.cpp GGUF](https://huggingface.co/audio-cpp/audio.cpp-gguf)。
