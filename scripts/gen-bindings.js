import { execSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(__dirname, '..');
const exportDir = path.join(rootDir, 'src', 'lib');

fs.mkdirSync(exportDir, { recursive: true });

console.log('[gen-bindings] 从 nova-protocol 生成 TypeScript 类型...');
try {
  execSync('cargo test -p nova-protocol', {
    cwd: rootDir,
    stdio: 'inherit',
    env: { ...process.env, TS_RS_EXPORT_DIR: exportDir },
  });
} catch (err) {
  console.error('[gen-bindings] 生成失败:', err.message);
  process.exit(1);
}

const target = path.join(exportDir, 'protocol.ts');
if (!fs.existsSync(target)) {
  console.error(`[gen-bindings] 未找到生成产物: ${target}`);
  process.exit(1);
}

console.log(`[gen-bindings] 已生成 ${path.relative(rootDir, target)}`);
