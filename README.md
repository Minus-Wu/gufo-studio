# 🦉 Gufo Studio

**Gufo Studio** 是專為高效能本地推論引擎官方版 [gufo-org/gufo](https://github.com/gufo-org/gufo)（`v0.35.0+`，針對 AMD Strix Halo / Ryzen AI MAX+ 395 / `gfx1151` 架構優化）打造的原生圖形介面包裝器（Desktop Wrapper）。

本專案提供輕量直覺的系統匣常駐圖示（System Tray）與現代化對話介面（Chat UI），預設綁定 `0.0.0.0` 以 HTTP (`12223`) 與內建 HTTPS (`12443`) 雙協定提供 OpenAI 相容 API 服務，讓使用者無需繁瑣的命令列即可享受極速本地與區域網路推論。

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
   - 完整適配官方 `gufo-org/gufo` (`v0.35.0+`) 推論參數：
     - MTP 投機解碼加速（`--speculative mtp --mtp-model ...`，官方已內建動態草稿長度控制與最長 64-token Prompt Lookup）
     - 128K / 256K 原生與 400K YaRN 超長上下文（`--context 131072`，超過 262,144 時自動附加 `--enable-yarn`）
     - 雙路並發與按需映射 KV（`--sessions 2`）
     - 視覺側車側載（`--mmproj mmproj-BF16.gguf`）
     - 零採樣貪婪解碼（`--temperature 0 --think off`）

4. **網路綁定、HTTPS 加密代理與 API Key 保護**：
   - 預設綁定 `0.0.0.0`（可於「配置」頁切換為 `127.0.0.1` 僅限本機）。
   - 內建 TLS 反向代理（預設 Port `12443`）：未指定自訂憑證時會自動產生並保存自簽憑證（`gufo-studio.cert.pem` / `gufo-studio.key.pem`），亦支援載入自訂 PEM 憑證與私鑰。
   - 配合官方 `gufo-org/gufo` 將認證交由前置代理的設計，當設定 API Key（支援圖形化密碼顯示/隱藏與隨機生成 `sk-gufo-...`）時，由內建 HTTPS 反向代理強制驗證 `Authorization: Bearer <KEY>`；底層 HTTP 維持原生直連。

5. **即時日誌流與狀態儀表板 (Live Logs & Dashboard)**：
   - 即時捕捉 Gufo 的 stdout / stderr。
   - 查看 AMD ROCm 初始化、GPU / NPU 狀態、記憶體佔用與請求級別 `timings` / 快取命中細節。

6. **可調整的進階推論參數**：
   - `--max-tokens`（預設生成上限）、`--speculative`（`mtp` / `off`）、預設 `--temperature`、`--think`。
   - 磁碟 Prompt 快取：預設開啟 `--cache-disk <DIR>`（預設目錄 `gufo-cache`）與 `--cache-disk-bytes`。
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
- **carve-out 或可用記憶體不足** 時，載入與運算會受影響（官方 `v0.35.0+` 已支援按需映射 KV 與記憶體壓力釋放，但大尺寸模型仍建議保留充足空間）。
- **context × sessions** 影響 KV 配置與並發排隊行為；超過 262,144 tokens 會自動啟用 YaRN 擴展（`--enable-yarn`）。
- 聊天頁面顯示的是伺服器回傳的 `timings` / `usage`，可直接與 Gufo 日誌中的 `decode_tps`、`prefill_tps` 對照。

---

## 🛠️ 技術架構

- **後端 (Backend)**：Rust 2024 + Tauri v2 + Tokio 非同步進程管理。
- **前端 (Frontend)**：React 19 + TypeScript + Vite + Lucide Icons + React-Markdown。
- **推論伺服器**：`gufo.exe`（AMD ROCm Windows 原生建置版）。

---

## 🚀 開發與建置

### 必備環境
- Node.js 20+ & npm
- Rust 1.80+ (MSVC 工具鏈)
- Windows 10/11 64-bit

### 啟動開發伺服器
```powershell
npm install
npm run tauri dev
```

### 建置單一執行檔便攜版 (Standalone Portable EXE)
本專案支援將前端 Web 介面完整靜態內嵌至單一二進制檔案中，免安裝即可直接執行：
```powershell
npm run build:portable
```
打包完成後將直接產出 `dist-portable/gufo-studio.exe`：
- **隨處可跑**：可直接將 `gufo-studio.exe` 拷貝放置於 Gufo 的 Release 資料夾（例如與 `gufo.exe` 同目錄）。
- **自動偵測**：啟動時自動探索同目錄或鄰近目錄的 `gufo.exe` 與 `models/Qwen3.8-Flash-Next-GGUF`。
- **便攜設定**：設定將自動持久化於同目錄下的 `gufo-studio.config.json`，隨身碟即插即用。

### 建置正式發行安裝包
```powershell
npm run tauri build
```

---

## 📄 授權 (License)

本專案採用 [MIT License](LICENSE) 授權。
