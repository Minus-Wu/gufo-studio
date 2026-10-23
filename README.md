# 🦉 Gufo Studio

**Gufo Studio** 是專為高效能本地推論引擎 [pixmaate/gufo](https://github.com/pixmaate/gufo)（針對 AMD Strix Halo / Ryzen AI MAX+ 395 / `gfx1151` 架構優化）打造的原生圖形介面包裝器（Desktop Wrapper）。

本專案提供輕量直覺的系統匣常駐圖示（System Tray）與現代化對話介面（Chat UI），並以本機 `12223` 埠號提供 OpenAI 相容 API 服務，讓使用者無需繁瑣的命令列即可享受極速本地推論。

---

## ✨ 核心特色

1. **系統匣常駐 (System Tray Icon)**：
   - 支援常駐於 Windows 工作列通知區。
   - 點擊托盤圖示可快速切換主視窗顯示 / 隱藏。
   - 提供右鍵選單快速啟動、停止推論引擎或優雅退出（自動釋放 GPU 顯存）。
   - 關閉主視窗時自動縮小至托盤，伺服器不中斷。

2. **極速對話介面 (Chat Interface)**：
   - 本機直連 OpenAI-compatible API (`http://127.0.0.1:12223/v1`)。
   - 支援 SSE 串流逐字輸出（Streaming Response）。
   - 即時計算推論速率（Tokens/s）與首字延遲（TTFT）。
   - 支援思考鏈（Thinking / Reasoning on/off）即時切換。
   - 支援 Markdown 語法渲染、代碼語法高亮與一鍵複製。
   - 多模態視覺支援（Vision / mmproj）：可貼上截圖或上傳圖片。

3. **專為 Qwen3.8-Flash-Next-GGUF 深度調優**：
   - 預設對接 `D:\Models\GGUF\Qwen3.8-Flash-Next-GGUF`。
   - 完整支援 `fast_start.ps1` 參數配方：
     - MTP 投機解碼加速（`--speculative mtp --mtp-policy survival`）
     - 詞彙前向預測加速（`--prompt-lookup`）
     - 128K 超長上下文（`--context 131072`）
     - 雙路並發（`--sessions 2`）
     - 視覺側車側載（`--mmproj mmproj-BF16.gguf`）
     - 零採樣貪婪解碼（`--temperature 0 --think off`）

4. **即時日誌流與狀態儀表板 (Live Logs & Dashboard)**：
   - 即時捕捉 Gufo 的 stdout / stderr。
   - 查看 AMD ROCm 初始化、GPU VRAM 記憶體佔用與模型分塊進度。

---

## 🛠️ 技術架構

- **後端 (Backend)**：Rust 2024 + Tauri v2 + Tokio 非同步進程管理。
- **前端 (Frontend)**：React 19 + TypeScript + Vite + Lucide Icons + React-Markdown。
- **推論伺服器**：`gufo.exe`（AMD ROCm Windows 原生建置版）。

---

## 🚀 開發與建置

### 必備環境
- Node.js 20+ & pnpm
- Rust 1.80+ (MSVC 工具鏈)
- Windows 10/11 64-bit

### 啟動開發伺服器
```powershell
pnpm install
pnpm tauri dev
```

### 建置正式發行版本
```powershell
pnpm tauri build
```

建置產物將輸出至 `src-tauri/target/release/gufo-studio.exe`。

---

## 📄 授權 (License)

本專案採用 [MIT License](LICENSE) 授權。
