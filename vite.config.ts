import { defineConfig } from 'vitest/config';
export default defineConfig({
  root: 'ui', clearScreen: false,
  server: { port: 1420, strictPort: true, host: '127.0.0.1' },
  build: { outDir: '../dist', emptyOutDir: true, target: 'es2022' },
  test: { environment: 'jsdom', include: ['**/*.test.ts'] },
});
