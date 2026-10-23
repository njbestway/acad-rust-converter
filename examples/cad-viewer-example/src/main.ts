import 'element-plus/dist/index.css'

import ElementPlus from 'element-plus'
import { createApp } from 'vue'

// The point of this example: make acad-rust (acadrust WASM, MPL-2.0) the app's
// default DWG parser BEFORE the viewer mounts. Registration is last-writer-wins
// on the AcDbDatabaseConverterManager DWG slot, so this call overrides whatever
// the host packages registered at import time.
//
// The worker + its sibling wasm deploy into ./assets via viteStaticCopy (see
// vite.config.ts); the worker resolves `acad-rust_bg.wasm` relative to its own
// URL, so pointing only at the worker script is enough for the WASM to load.
import { i18n } from '@mlightcad/cad-viewer'

import App from './App.vue'
import { registerAcadRustConverter } from './registerAcadRust'

registerAcadRustConverter('./assets/acad-rust-parser-worker.js')

const app = createApp(App)
app.use(ElementPlus)
app.use(i18n)
app.mount('#app')
