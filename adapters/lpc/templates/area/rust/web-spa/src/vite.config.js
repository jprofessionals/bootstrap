import { defineConfig } from 'vite'

export default defineConfig({
  base: process.env.MUD_BASE_URL || './',
  build: {
    outDir: 'dist',
    emptyOutDir: true
  }
})
