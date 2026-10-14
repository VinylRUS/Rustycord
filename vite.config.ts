import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { readFileSync } from 'node:fs';

// Версия приложения — из package.json (единственный источник правды:
// обновляется при релизе в 4 местах, UI берёт её отсюда автоматически).
const pkg = JSON.parse(
  readFileSync(new URL('./package.json', import.meta.url), 'utf-8'),
) as { version: string };

// Tauri expects a fixed port + no clearing of the console output.
// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [svelte()],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // Do not watch the Rust backend while serving the frontend.
      ignored: ['**/src-tauri/**'],
    },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: {
    target: 'es2022',
    minify: 'esbuild',
    sourcemap: false,
  },
});
