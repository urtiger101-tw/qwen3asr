# 0.2.0 原生版本驗收

驗收日期：2026-09-27。環境：Windows x64、NVIDIA GeForce RTX 3060 Ti 8 GB、32 GB RAM。這些是本機實測結果，不代表所有硬體或所有語言的辨識品質。

## 執行結果

| 項目 | 結果 |
|---|---|
| Rust 測試 | PASS，42 項，含字幕完整覆蓋、模型 manifest、取消與子程序回收 |
| 格式、Clippy、release build | PASS；Clippy 使用 `-D warnings` |
| CPU / CUDA 原生推論 | PASS；Q8 GGUF 與官方 HF ASR 模型均實跑 |
| NVIDIA 不可用時自動退回 CPU | PASS；以 `CUDA_VISIBLE_DEVICES=-1` 驗證 |
| 45 秒影片與跨區塊字幕 | PASS；105 個對齊詞、14 個字幕 cue；CPU / CUDA 文字一致，交界詞無重複 |
| 中文路徑、空白路徑、繁中輸出 | PASS；已安裝版本實跑 |
| 無語音輸入 | PASS；原生 worker 回傳空文字、語言與字詞陣列 |
| 安裝版獨立執行 | PASS；PATH 限制為安裝目錄與 Windows 系統目錄，CUDA 辨識及 SRT 輸出正常 |
| 安裝版 MCP | PASS；官方 MCP SDK 連接 6 tools，完成辨識、取消真實 C++ worker、佇列自動接續 |
| Inno Setup 升級、卸載、重裝 | PASS；各次退出碼 0，模型快取保留 |
| PATH / Codex / AGY | PASS；PATH 僅一筆、其他設定不變；卸載移除自有整合，重裝後內容一致 |
| 安裝內容 | PASS；主程式與 40 個原生檔案 hash 一致，Codex / AGY Skills 一致，未包含 Python runtime |
| 公開原始碼檢查 | PASS；未包含模型、二進位檔、私密設定或憑證 |

MCP 是以官方 SDK 驗證協定與實際工作流程；不宣稱已在每一款 Agent 客戶端的 GUI 中操作。重裝整合後，客戶端可能需要重新載入 MCP 或重啟。

## 記憶體與效能觀察

| 測試 | 完整辨識與對齊耗時 | 原生 worker 最高 working set |
|---|---:|---:|
| 4.20 秒中文，Q8 CPU、自動語言 | 32.29 秒 | 2,354,212,864 bytes（約 2.19 GiB） |
| 45 秒英文影片，Q8 CUDA | 6.32 秒 | 1,449,996,288 bytes（約 1.35 GiB） |

同一中文 fixture 的舊 Python / PyTorch CPU 路徑量測值為 5,051,416,576 bytes（約 4.70 GiB）。新預設 Q8 原生路徑減少約 53%。這同時包含推論框架與量化模型的變更，不是控制所有條件的引擎比較。數值是程序 working set，**不等於 GPU VRAM 使用量**，也不是資源上限承諾。

## 本機安裝包

- 檔名：`qwen3asr-0.2.0-setup.exe`
- 大小：567,169,414 bytes
- SHA-256：`f0074cb8ab5e26fb8c65775d089ebc3c3299efcb2cd0c25d1c828944ff9b47a7`
- 本 repository 公開原始碼；本次沒有公開發布第三方二進位安裝包。相關散布範圍見 [FFmpeg 文件](ffmpeg-distribution.md)。

CUDA 安裝包以 compute capability 8.6 為最低要求；其他架構可從原始碼重建，較舊裝置預設使用 CPU。模型權重不隨 repo 或安裝包散布，首次使用另外下載並校驗。

GitHub Actions 執行 Windows Rust 格式、測試、Clippy 與 release build；雲端 CI 不執行上述實體 GPU、模型或安装器驗收。
