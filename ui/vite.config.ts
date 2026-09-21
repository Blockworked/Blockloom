import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';

// Fixed dev port for the browser dev-bridge loop (`just dev-ui` +
// `just dev-backend`), and file-watching that ignores the Rust side.
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  resolve: {
    dedupe: ['vue', 'blockstitch'],
  },
  optimizeDeps: {
    // blockstitch keeps its canvas host in module-level state. Prebundling it
    // beside its source modules creates two hosts during the dev loop.
    exclude: ['blockstitch'],
  },
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    outDir: 'dist',
  },
});
