# Windows 安裝

從 [GitHub Releases](https://github.com/urtiger101-tw/qwen3asr/releases/latest) 下載 `qwen3asr-0.2.1-setup.exe`。這是已編譯的 Windows x64 Inno Setup 安裝程式，不需要 Rust、Visual Studio、Python 或 CUDA Toolkit。NVIDIA 顯示驅動仍須由系統提供。

## 安裝與開始使用

1. 執行安裝程式，選擇「所有使用者」並同意 Windows UAC。程式預設安裝至 `C:\Program Files\Qwen3ASR`。
2. 保留加入 PATH 與下載模型的選項。所有使用者安裝寫入系統 PATH；「僅目前使用者」則使用 LocalAppData 與使用者 PATH。
3. 安裝器先從固定上游取得 FFmpeg／FFprobe，再驗證 SHA-256。下載模型工作為啟動安裝的帳戶準備 ASR 與 ForcedAligner，顯示 CLI 下載進度；現有且完整的快取可以重用。
4. 完成後開啟新的終端機：

```powershell
qwen3asr --version
qwen3asr doctor --json
qwen3asr models status
qwen3asr "D:\Recordings\meeting audio.wav" --traditional
```

公開安裝包內含已編譯的 Rust CLI、C++ CPU/CUDA 推論引擎及所需 DLL、Skill、文件與授權聲明。FFmpeg 直接由安裝器從上游下載約 171 MB；新設定的預設 Q8 ASR／ForcedAligner 另外下載約 2.3 GB。模型保存在 `%LOCALAPPDATA%\Qwen3ASR\models`，既有設定如指定其他模型，下載工作會遵循該設定。預留至少 5 GB 磁碟空間；下載完成後辨識可完全離線。模型不放在 Program Files，各 Windows 帳戶有自己的設定及快取。

若取消模型下載工作，第一次辨識會自動下載，也可稍後手動準備：

```powershell
qwen3asr setup --download-model
qwen3asr "D:\Recordings\meeting.mp4" --offline --format srt --format vtt --format json
```

預設使用 NVIDIA GPU；CUDA 不可用時改用 CPU。隨附 CUDA 核心需要 compute capability 8.6+。`--device cpu` 可強制 CPU；`--device cuda` 要求 CUDA，失敗直接回報。推論不會使用 Python／PyTorch，也不會上傳音訊。

預設在來源檔旁輸出 TXT、SRT、JSON。已有輸出受保護，覆寫須明確加 `--overwrite`。

## 系統 PATH 與使用者 PATH

所有使用者安裝固定使用 `C:\Program Files\Qwen3ASR`（依系統 Program Files 位置決定），只將此目錄加入 Machine PATH，並要求 Windows 管理員確認；不可用 `/DIR` 改成其他目錄。僅目前使用者安裝可選擇目錄，只修改 User PATH，不需要提升權限。安裝器不會把 LocalAppData 目錄加入 Machine PATH。

解除安裝只移除該安裝自行加入的 PATH 項目；其他項目的順序與內容保留。若原本已有同一路徑，卸載保留原有項目。安裝完成前已開啟的終端機可能仍使用舊環境，請開新視窗。

## Codex、AGY 與 Claude

目前使用者安裝可勾選需要的 agent 整合，會加入 MCP 設定和 Skill。所有使用者安裝不代替其他帳戶修改設定；登入後各自執行：

```powershell
qwen3asr agents install --target codex
qwen3asr agents install --target agy
qwen3asr agents install --target claude
```

重新載入 MCP 或重啟客戶端即可取得新程序。移除共用安裝前，各帳戶執行 `qwen3asr agents uninstall --target ...`。目前使用者安裝所建立的整合，卸載時會核對安裝所有權並清理；不屬於此安裝或被使用者改動的設定保留。

## 失敗與復原

下載媒體工具失敗或 SHA-256 不符時，安裝會停止並顯示錯誤。PATH、模型準備或所選整合失敗時，安裝器會顯示具體失敗並回傳非零退出碼；程式檔可能已經安裝，請依安裝記錄修復，不應把它當成模型已就緒。模型下載支援續傳，重新執行 `qwen3asr setup --download-model` 可恢復。

使用 `/LOG="完整路徑"` 保存安裝記錄。`/ALLUSERS` 選擇系統範圍並要求 UAC；`/CURRENTUSER` 選擇使用者範圍。離線媒體準備可透過 `/FFMPEGARCHIVE="固定版本 zip 的完整路徑"`，安裝器仍要求完全相同的 SHA-256；模型須事先準備，或安裝時取消模型下載。

互動安裝會顯示隨附 CUDA 元件的 NVIDIA 授權條款並要求同意。使用 `/SILENT` 或 `/VERYSILENT` 部署前，請先閱讀安裝包隨附的 CUDA EULA；只有同意時才傳入 `/ACCEPTCUDAEULA`，未提供此參數的靜默安裝會停止。

## 升級、搬移與解除安裝

相同安裝範圍的升級可重用原路徑；設定及模型保留。0.1.x 的 Python 快取可能仍留在磁碟，但原生版不會載入或重新下載 Python／PyTorch。

切換安裝範圍時，先確保新的安裝可使用，再解除舊整合及舊安裝，最後從新路徑重新設定需要的整合。不要直接搬移程式資料夾；PATH 與 client 設定可能保存絕對路徑。模型與設定獨立保存，不因解除安裝程式檔而刪除。

## 從原始碼封裝

開發建置需要 Windows x64、Rust、PowerShell 7.4+、CMake、Ninja、Visual Studio C++、CUDA Toolkit 和 Inno Setup 7。一般使用者下載 Releases 安裝包即可。

```powershell
pwsh -NoProfile -File scripts/build-native.ps1 -Backend both
pwsh -NoProfile -File scripts/package.ps1
```

`package.ps1` 驗證 Rust 版本、CPU/CUDA worker 的 source commit、patch／worker 雜湊、DLL 與 probe，將完整 stage 留在 `.ag-artifacts\packaging-stage`。公開安裝包不嵌入 FFmpeg 執行檔或其 archive；安裝期間從上游下載。開發用 `scripts/prepare-ffmpeg.ps1` 仍可準備本機辨識環境。可用 `-NativeRoot` 指定原生 stage、`-IsccPath` 指定編譯器。
