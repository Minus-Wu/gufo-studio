import { execSync } from 'child_process';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');

console.log('🚀 開始建置 Gufo Studio 單一執行檔 (Portable / Standalone Single Executable)...');

// 1. 使用 Tauri CLI 的 --no-bundle 模式進行正式發布建置
// 這會自動建置前端靜態資產，並將其全部內嵌至獨立的二進制檔案中（絕不連線 localhost 開發伺服器）
console.log('\n📦 [1/2] 執行 Tauri 生產環境建置 (靜態嵌入前端與編譯 Release 二進制檔案)...');
execSync('pnpm tauri build --no-bundle', {
  cwd: rootDir,
  stdio: 'inherit',
  shell: true,
});

// 2. 複製產出到 dist-portable 資料夾
console.log('\n📂 [2/2] 整理單一執行檔輸出目錄...');
const outputDir = path.join(rootDir, 'dist-portable');
if (!fs.existsSync(outputDir)) {
  fs.mkdirSync(outputDir, { recursive: true });
}

const sourceExe = path.join(rootDir, 'src-tauri', 'target', 'release', 'gufo-studio.exe');
const targetExe = path.join(outputDir, 'gufo-studio.exe');

if (!fs.existsSync(sourceExe)) {
  console.error(`❌ 找不到編譯產物: ${sourceExe}`);
  process.exit(1);
}

fs.copyFileSync(sourceExe, targetExe);

const stats = fs.statSync(targetExe);
const sizeMB = (stats.size / (1024 * 1024)).toFixed(2);

// 產生便攜使用說明
const readmeContent = `🦉 Gufo Studio - 單一執行檔便攜版 (Standalone Portable)
======================================================
檔案大小: ${sizeMB} MB
建置時間: ${new Date().toLocaleString()}

使用方式:
1. 免安裝: 本檔案為完全獨立的單一執行檔，前端 Web 與圖形介面已完整嵌入二進制檔案中。
2. 搭配 Gufo 使用:
   - 推薦直接將 gufo-studio.exe 拷貝至 Gufo 的 Release 資料夾 (例如與 gufo.exe 同目錄或上一層)。
   - Gufo Studio 將自動辨識同目錄下的 gufo.exe 與 models 模型資料夾。
   - 亦可直接執行，並在介面中的「配置 (Config)」面板自訂指定 gufo.exe 與模型路徑。
3. 預設服務連接埠: 12223 (OpenAI API 端點: http://127.0.0.1:12223/v1)
4. 關閉主視窗時自動縮小至系統匣常駐，右鍵托盤圖示可隨時啟動/停止推論伺服器或徹底退出。
`;

fs.writeFileSync(path.join(outputDir, 'README-PORTABLE.txt'), readmeContent, 'utf-8');

console.log('\n🎉 打包完成！');
console.log(`📁 單一執行檔輸出路徑: ${targetExe} (${sizeMB} MB)`);
console.log(`📄 說明文件: ${path.join(outputDir, 'README-PORTABLE.txt')}\n`);
