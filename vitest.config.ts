import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'jsdom',
    include: ['packages/*/src/**/*.test.ts', 'apps/desktop/src/**/*.test.{ts,tsx}'],
  },
  resolve: {
    alias: {
      '@eigendesk/protocol': new URL('./packages/protocol/src/index.ts', import.meta.url).pathname,
      '@eigendesk/plugin-sdk': new URL('./packages/plugin-sdk/src/index.ts', import.meta.url).pathname,
      '@eigendesk/ui-kit': new URL('./packages/ui-kit/src/index.ts', import.meta.url).pathname,
    },
  },
});
