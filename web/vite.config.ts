import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// 由 recallcard ui 同源托管，因此不需要代理或跨域配置。
export default defineConfig({
  plugins: [react()],
  build: { outDir: 'dist', emptyOutDir: true, assetsDir: 'assets' },
  server: { port: 5173 },
});
