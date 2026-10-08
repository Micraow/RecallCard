import { defineConfig } from 'vite';
import { readFileSync } from 'node:fs';

const manifest = readFileSync(new URL('../src-tauri/Cargo.toml', import.meta.url), 'utf8');
const version = manifest.match(/^version\s*=\s*"([^"]+)"/m)?.[1] ?? 'unknown';

export default defineConfig({
  root: new URL('.', import.meta.url).pathname,
  base: './',
  define: { __APP_VERSION__: JSON.stringify(version) },
  server: { host: '127.0.0.1', port: 1420, strictPort: true },
  build: { outDir: '../dist', emptyOutDir: true, target: 'es2022' },
});
