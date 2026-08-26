import preact from '@preact/preset-vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [preact()],
  build: {
    target: 'es2022',
    cssCodeSplit: true,
    reportCompressedSize: true,
    sourcemap: false,
    chunkSizeWarningLimit: 180,
  },
});
