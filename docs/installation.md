# Windows 安裝

執行建置產生的 `qwen3asr-0.2.0-setup.exe`。本 repository 目前公開原始碼，安裝包在本機建置；公開散布第三方二進位檔前，另須完成 [FFmpeg 來源與授權材料](ffmpeg-distribution.md)。預設安裝給目前 Windows 使用者；需要共用程式檔案時，可在安裝程式選擇所有使用者，Windows 會要求系統管理員權限。安裝完成後開啟新的終端機：

```powershell
qwen3asr doctor --json
qwen3asr "D:\Recordings\meeting audio.wav"
```

安裝程式可將安裝資料夾加入 PATH。current-user 安裝只修改該使用者的 PATH；all-users 安裝只修改 system PATH。解除安裝時只移除安裝程式自己新增的路徑項目，保留同一 PATH 值中的其他內容與原先已存在的 Qwen3ASR 路徑。

安裝包包含 Rust CLI、CPU 與 CUDA 原生推論 worker、隨附原生 DLL、FFmpeg/FFprobe、Skill、文件與第三方授權聲明。執行推論不需要 Python、PyTorch、`uv`、`pip` 或雲端 API。第一次辨識會下載固定版本並驗證 SHA-256 的 Q8 ASR 與 ForcedAligner 模型，合計約 2.3 GB；模型另存於使用者資料目錄，並未放進安裝包。預留至少 5 GB 可用空間。準備完成後可離線辨識。

預設使用 NVIDIA CUDA，無法使用 CUDA 時會改用 CPU；`--device cuda` 會要求 CUDA，`--device cpu` 可強制使用 CPU。NVIDIA 顯示驅動仍須由系統提供。常用命令如下：

```powershell
# 預先下載模型，以便之後離線使用
qwen3asr setup --download-model
qwen3asr "D:\Recordings\meeting.wav" --offline --output-dir .\subtitles

# 影片、多種輸出格式與傳統中文
qwen3asr "D:\Recordings\meeting.mp4" --format srt --format vtt --format json --traditional

# 指定 CPU、區塊長度與輸出資料夾
qwen3asr "D:\Recordings\meeting.wav" --device cpu --chunk-seconds 15 --output-dir .\subtitles
```

預設會在來源檔旁輸出 UTF-8 TXT、SRT 與 JSON；已有輸出不會被覆寫，除非明確加上 `--overwrite`。`qwen3asr doctor --json` 可檢查原生 runtime 與模型狀態；`qwen3asr models status` 顯示本機模型。

## 整合與解除安裝

Codex、AGY 與 Claude 整合是僅限 current-user 安裝的選用工作。安裝器只為目前帳戶執行 `qwen3asr.exe agents install --target ...`；成功後會記錄此安裝路徑和目標。解除安裝會在刪除程式檔之前，對有此安裝所有權標記的目標執行同一份 CLI 的 `agents uninstall`。CLI 會核對自己的 client ownership sidecar，保留已變更或屬於其他安裝的 client 設定。失敗會寫入安裝／解除安裝記錄並顯示通知；請檢查記錄，必要時手動修復仍指向舊路徑的 client 設定。

all-users 安裝不會設定這些整合，因為安裝程式無法安全代替每個 Windows 帳戶修改 client 設定。需要整合的帳戶可在自己的登入工作階段執行 `qwen3asr agents install --target codex`、`agy` 或 `claude`；移除共用程式前，各帳戶應各自執行對應的 `agents uninstall`。

若 PATH 更新或所選整合安裝失敗，Setup 會完成程式檔安裝後以結束碼 `10` 結束；CLI 仍可從安裝資料夾執行。靜默安裝時也會回傳 `10`，請查看 Setup log 中的失敗項目。

## 升級與安裝路徑

從 0.1.x 升級時，建議直接在原安裝位置執行 0.2.0 安裝程式，以維持 PATH 與 agent client 裡記錄的絕對路徑。0.2.0 不再使用舊版 Python runtime、`uv` 或舊 Python runtime/cache；升級不會遞迴清除這些檔案或使用者資料，因此它們可能留在磁碟上但不會被新版本載入。原有使用者資料會保留。0.2.0 不會安裝或重新下載 PyTorch；第一次辨識只下載新的原生模型檔案。

不支援直接搬移已安裝的程式資料夾，因為 PATH 與 agent client 設定可能保存舊位置。若要更改安裝位置，先在原位置解除各 agent 整合並解除安裝，再安裝到新位置並重新設定需要的整合。模型與使用者資料保留在使用者資料目錄，可供新位置的安裝續用。

使用 `qwen3asr --help` 查看 CLI 命令與選項。安裝路徑及音訊路徑可包含空白；從 PowerShell 或 Command Prompt 呼叫時請加引號。

## 從原始碼封裝安裝程式

Windows x64 發行封裝需要 Rust toolchain、PowerShell 7.4+、Inno Setup 7、已完成的 `native/bin/cpu` 與 `native/bin/cuda` worker stage，以及同一 native stage 下的 `ffmpeg.exe`、`ffprobe.exe` 和 `licenses`。封裝時缺少任何必要輸入都會直接失敗，不會改用系統 PATH 上的 FFmpeg。預設從 PATH、環境變數或 `Program Files`／`LOCALAPPDATA` 尋找 `ISCC.exe`；也可傳入其他 stage 路徑：

`scripts/prepare-ffmpeg.ps1` 會直接從上游下載固定版本的 static LGPL 工具並核對 SHA-256。`QWEN3ASR_INSTALL_DIR` 是開發用的原生資源目錄覆寫值，通常不需設定；一般執行檔會自動尋找安裝目錄。

```powershell
pwsh -NoProfile -File .\scripts\package.ps1 `
  -NativeRoot D:\staging\native\bin `
  -FfmpegPath D:\staging\native\bin\ffmpeg.exe `
  -FfprobePath D:\staging\native\bin\ffprobe.exe
```

封裝腳本會先編譯並檢查 Rust CLI，再驗證 worker 與媒體工具，並把每次完整 stage 保留在 `.ag-artifacts\packaging-stage` 供檢查。Inno Setup compiler 可由 PATH、`ISCC_PATH`／`ISCC_EXE`／`INNO_SETUP_HOME`／`INNO_SETUP_ROOT`，或環境變數導出的標準安裝位置找到；也可明確傳入 `-IsccPath`。
