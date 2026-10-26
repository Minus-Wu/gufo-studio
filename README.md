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
   - 支援 SSE 串流逐字輸出（Streaming Response），以 requestAnimationFrame 節流重繪。
   - 回覆完成後顯示**伺服器量測**的解碼速率、Prefill 速率、TTFT、輸入/輸出 tokens、快取命中與 MTP 接受率；串流中顯示的 `≈ tok/s` 為前端以 SSE chunk 數估算的近似值。
   - 思考鏈開關以 `chat_template_kwargs.enable_thinking` 逐請求控制（預設值跟隨伺服器設定的 `--think`）。
   - 歡迎訊息與錯誤提示不會送入模型對話歷史；先前回覆的 `reasoning_content` 會一併回傳。
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

4. **API Key 安全金鑰保護 (Authorization Bearer Token)**：
   - 支援原生 `--api-key <KEY>` 伺服器安全機制。
   - 提供圖形化密碼顯示/隱藏切換、隨機高強度金鑰生成（`sk-gufo-...`）。
   - 保障區域網路（LAN）或本機服務存取安全，內建 Chat 介面與健康檢查探測自動附加金鑰。

5. **即時日誌流與狀態儀表板 (Live Logs & Dashboard)**：
   - 即時捕捉 Gufo 的 stdout / stderr。
   - 查看 AMD ROCm 初始化、GPU VRAM 記憶體佔用與模型分塊進度。
   - 日誌中的啟動命令會自動遮蔽 `--api-key` 的值。

6. **可調整的進階推論參數**：
   - `--max-tokens`（預設生成上限）、`--draft-tokens`、`--mtp-draft-vocab full|latin`、`--prefill-chunk`、`--prompt-lookup`、預設 `--temperature`。
   - 磁碟 Prompt 快取：`--cache-disk <DIR>` 與 `--cache-disk-bytes`。
   - 數值為 0 表示不傳該參數、交給 Gufo 預設值。

---

## ⚙️ 設定與程序管理

- **單一設定來源**：所有設定存於執行檔旁的 `gufo-studio.config.json`。舊版存在 WebView localStorage 的設定會在首次啟動時自動遷移並清除；「還原預設配置」會同時寫回設定檔。舊版設定檔缺少的新欄位會自動補上預設值。
- **就緒判斷**：以 Gufo 的 `/ready` 端點（舊版退回 `/health`）與日誌中的 `event=listening` 判斷模型載入完成，不設硬性啟動逾時（大型模型在較小 carve-out 下可能需數分鐘）。
- **安全停止**：只會停止由 Gufo Studio 啟動的子程序，或「在設定埠號上監聽、且映像檔名稱符合 gufo 的程序」，不再以 `taskkill /IM` 依名稱終止所有同名程序。停止時先送出 Ctrl+C 讓 Gufo 優雅釋放 GPU 記憶體，15 秒內未結束才強制終止。
- 視窗位置/大小的儲存已去抖動（debounce），不會在拖曳時持續寫檔。

## 📊 效能預期（與 Gufo 宣稱數據的差異）

Gufo README 的峰值數據（例如 prefill ≈ 1600 tok/s、單路 decode ≈ 59 tok/s、8 路並發總吞吐 ≈ 157 tok/s）是在 Linux、96GB carve-out、重複性高的基準文字下量測。Windows 上以一般中英文混合內容實測較常見的範圍為：

| 指標 | 典型範圍 (Windows, Flash-Next, MTP) |
|------|-------------------------------------|
| Prefill | 約 680–840 tok/s |
| 單路 Decode | 約 32–35 tok/s |

影響因素：
- **carve-out 小於 96GB** 時，載入時間變長（64GB 約 2 分鐘）且 decode 變慢。
- **context × sessions** 決定預先配置的 KV 快取大小；Flash-Next 256K 單會話約需 89 GiB。
- `--mtp-policy survival` 只在 temperature > 0 時生效。
- `--mtp-draft-vocab latin` 只適合英文/程式碼輸出，中文輸出會降低 MTP 接受率。
- 聊天頁面顯示的是伺服器回傳的 `timings` / `usage`，可直接與 Gufo 日誌中的 `decode_tps`、`prefill_tps` 對照。

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

### 建置單一執行檔便攜版 (Standalone Portable EXE)
本專案支援將前端 Web 介面完整靜態內嵌至單一二進制檔案中，免安裝即可直接執行：
```powershell
pnpm build:portable
```
打包完成後將直接產出 `dist-portable/gufo-studio.exe`：
- **隨處可跑**：可直接將 `gufo-studio.exe` 拷貝放置於 Gufo 的 Release 資料夾（例如與 `gufo.exe` 同目錄）。
- **自動偵測**：啟動時自動探索同目錄或鄰近目錄的 `gufo.exe` 與 `models/Qwen3.8-Flash-Next-GGUF`。
- **便攜設定**：設定將自動持久化於同目錄下的 `gufo-studio.config.json`，隨身碟即插即用。

### 建置正式發行安裝包
```powershell
pnpm tauri build
```

---

## 📄 授權 (License)

本專案採用 [MIT License](LICENSE) 授權。
