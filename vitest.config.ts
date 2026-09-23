import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'jsdom',
    include: [
      'packages/*/src/**/*.test.{ts,tsx}',
      'apps/desktop/src/**/*.test.{ts,tsx}',
      'plugins/*/src/**/*.test.{ts,tsx}',
      'examples/*/src/**/*.test.{ts,tsx}',
      'tests/**/*.test.{ts,tsx}',
    ],
  },
  resolve: {
    alias: {
      '@eigendesk/protocol': new URL('./packages/protocol/src/index.ts', import.meta.url).pathname,
      '@eigendesk/plugin-sdk': new URL('./packages/plugin-sdk/src/index.ts', import.meta.url).pathname,
      '@eigendesk/plugin-sdk/h': new URL('./packages/plugin-sdk/src/h.ts', import.meta.url).pathname,
      '@eigendesk/plugin-sdk/markdown': new URL('./packages/plugin-sdk/src/markdown.ts', import.meta.url).pathname,
      '@eigendesk/ui-kit': new URL('./packages/ui-kit/src/index.tsx', import.meta.url).pathname,
    },
  },
});
