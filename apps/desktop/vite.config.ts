import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: 'es2022',
    outDir: 'dist',
    emptyOutDir: true,
  },
  resolve: {
    alias: {
      '@workbench-zero/protocol': resolve(here, '../../packages/protocol/src/index.ts'),
      '@workbench-zero/plugin-sdk': resolve(here, '../../packages/plugin-sdk/src/index.ts'),
      // Order matters: the specific tokens.css alias must precede the
      // package alias (vite aliases are prefix matches).
      '@workbench-zero/ui-kit/tokens.css': resolve(here, '../../packages/ui-kit/src/tokens.css'),
      '@workbench-zero/ui-kit': resolve(here, '../../packages/ui-kit/src/index.tsx'),
      '@shell': resolve(here, 'src'),
    },
  },
});
