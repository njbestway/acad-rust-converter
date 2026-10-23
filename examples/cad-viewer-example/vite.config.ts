import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'
import { viteStaticCopy } from 'vite-plugin-static-copy'

// The acad-rust npm artifact ships its runtime assets (worker + its sibling
// wasm, plus the main-thread transcoder) unbundled under `dist/` — they are
// fetched by URL at runtime, so a consumer must copy them next to the app.
// `@njbestway/acad-rust-converter` is a `workspace:*` dep here, so pnpm links
// node_modules/@njbestway/acad-rust-converter to packages/acad-rust-converter;
// its `dist/` is produced by `pnpm --filter @njbestway/acad-rust-converter build`.
const ACADRUST_DIST = './node_modules/@njbestway/acad-rust-converter/dist'

// Canonical filenames (mirror of tools/worker-assets.mjs and the converter's
// internal constants). The wasm is a sibling of whichever script loads it, so
// worker + wasm + main-thread all deploy into the SAME `assets/` directory —
// the worker resolves `acad-rust_bg.wasm` relative to its own URL.
const ACADRUST_PARSER_WORKER_FILE = 'acad-rust-parser-worker.js'
const ACADRUST_WASM_FILE = 'acad-rust_bg.wasm'
const ACADRUST_MAIN_THREAD_FILE = 'acad-rust-converter-main-thread.js'

export default defineConfig({
  base: './',
  plugins: [
    vue(),
    viteStaticCopy({
      targets: [
        ACADRUST_PARSER_WORKER_FILE,
        ACADRUST_WASM_FILE,
        ACADRUST_MAIN_THREAD_FILE
      ].map(file => ({
        src: `${ACADRUST_DIST}/${file}`,
        dest: 'assets',
        rename: { stripBase: true }
      }))
    })
  ]
})
