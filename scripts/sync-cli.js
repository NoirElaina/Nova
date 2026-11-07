import { execSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');

const isWindows = process.platform === 'win32';
const binName = isWindows ? 'nova.exe' : 'nova';

const srcExe = path.join(rootDir, 'target', 'release', binName);
const dstBinDir = path.join(rootDir, 'src-tauri', 'bin');
const dstExe = path.join(dstBinDir, binName);
const artifactsDir = path.join(rootDir, 'release-artifacts');
const artifactsExe = path.join(artifactsDir, binName);

console.log('[sync-cli] 正在编译最新的 nova-cli release 二进制...');
try {
  execSync('cargo build -p nova-cli --release', {
    cwd: rootDir,
    stdio: 'inherit',
    env: process.env,
  });
} catch (err) {
  console.error('[sync-cli] 编译 nova-cli 失败:', err.message);
  process.exit(1);
}

if (!fs.existsSync(srcExe)) {
  console.error(`[sync-cli] 编译产物不存在: ${srcExe}`);
  process.exit(1);
}

if (!fs.existsSync(dstBinDir)) {
  fs.mkdirSync(dstBinDir, { recursive: true });
}

try {
  fs.copyFileSync(srcExe, dstExe);
  console.log(`[sync-cli] ✓ 已成功自动同步到桌面端打包目录: ${dstExe}`);
} catch (err) {
  console.error(`[sync-cli] 拷贝到 ${dstExe} 失败:`, err.message);
  process.exit(1);
}

if (fs.existsSync(artifactsDir)) {
  try {
    fs.copyFileSync(srcExe, artifactsExe);
    console.log(`[sync-cli] ✓ 已同步至发布产物目录: ${artifactsExe}`);
  } catch (err) {
    console.warn(`[sync-cli] 提示: ${artifactsExe} 可能正被运行中的进程占用，已跳过发布产物覆盖`);
  }
}
