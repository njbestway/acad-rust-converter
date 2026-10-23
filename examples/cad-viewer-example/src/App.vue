<template>
  <div class="page">
    <header class="bar">
      <h1>acad-rust → cad-viewer</h1>
      <input type="file" accept=".dwg" @change="onPick" />
      <button :disabled="!picked || benching" @click="benchPicked">
        {{ benching ? 'Parsing…' : 'Parse (WASM worker)' }}
      </button>
      <button :disabled="!picked" @click="openInViewer">Open in viewer</button>
      <span v-if="error" class="error">{{ error }}</span>
    </header>

    <div v-if="bench" class="bench">
      <table>
        <tbody>
          <tr>
            <td>File</td>
            <td>{{ bench.fileName }}</td>
          </tr>
          <tr>
            <td>
              Open + map → AcDbDatabase (acadrust WASM in a Web Worker, sectioned)
            </td>
            <td>{{ bench.totalMs.toFixed(0) }} ms</td>
          </tr>
          <tr>
            <td>Model-space entities</td>
            <td>{{ bench.entities }}</td>
          </tr>
          <tr>
            <td>Unmapped entity types (counted, skipped)</td>
            <td>{{ bench.unknownEntities }}</td>
          </tr>
        </tbody>
      </table>
      <p class="note">
        Model-space entities should match the acadrust 0.6.6 native reference for
        <code>高山.dwg</code> — <b>2,199</b> model-space (of <b>244,756</b> total
        document entities; the total is the figure asserted by the native
        <code>dump_fixture</code> meta, which this WASM worker decodes
        identically).
      </p>
    </div>

    <p v-else-if="!viewerFile" class="hint">
      Pick a <code>.dwg</code> file. This app registers
      <code>@njbestway/acad-rust-converter</code> (Architecture B: acadrust
      <code>CadDocument</code> sections → <code>AcDbDatabase</code>) on the DWG
      slot at startup (last-writer-wins), so <b>every</b> DWG open — including
      the viewer's built-in Open dialog below — runs through the acadrust WASM
      inside a Web Worker. The <code>.wasm</code> and worker load from the
      <code>./assets/</code> directory copied out of the npm package's
      <code>dist/</code> by <code>viteStaticCopy</code>. <b>Parse</b> times a
      standalone worker parse and reports the model-space entity count;
      <b>Open in viewer</b> hands the file to <code>MlCadViewer</code>.
    </p>

    <section class="viewer">
      <MlCadViewer v-if="viewerFile" locale="default" :local-file="viewerFile" />
    </section>
  </div>
</template>

<script setup lang="ts">
import { AcDbAcadRustNativeConverter } from '@njbestway/acad-rust-converter'
import { MlCadViewer } from '@mlightcad/cad-viewer'
import {
  acdbHostApplicationServices,
  AcDbDatabase
} from '@mlightcad/data-model'
import { ref } from 'vue'

// Same deployed worker URL the app registers on the DWG slot (see main.ts); the
// worker fetches its sibling `acad-rust_bg.wasm` relative to this URL.
const PARSER_WORKER_URL = './assets/acad-rust-parser-worker.js'

interface BenchResult {
  fileName: string
  totalMs: number
  entities: number
  unknownEntities: number
}

const picked = ref<File | null>(null)
const viewerFile = ref<File | null>(null)
const benching = ref(false)
const bench = ref<BenchResult | null>(null)
const error = ref<string | null>(null)

const onPick = (event: Event) => {
  const input = event.target as HTMLInputElement
  picked.value = input.files && input.files.length > 0 ? input.files[0] : null
  bench.value = null
  error.value = null
}

const runBench = async (file: File) => {
  benching.value = true
  error.value = null
  bench.value = null
  try {
    const buffer = await file.arrayBuffer()
    const converter = new AcDbAcadRustNativeConverter({
      useWorker: true,
      parserWorkerUrl: PARSER_WORKER_URL
    })
    const db = new AcDbDatabase()
    // A standalone parse has no host (the viewer) to bind the working database.
    // Entities that look the database up while being constructed (Hatch,
    // Dimension) need it, or the conversion throws
    // "The current working database must be set before using it!".
    acdbHostApplicationServices().workingDatabase = db

    // The PARSE/END progress payload carries the running `unknownEntityCount`
    // (entity variants the mapper did not recognize) so coverage is tracked per
    // file rather than silently dropped.
    let unknownEntities = 0
    const t0 = performance.now()
    await converter.read(buffer, db, {
      progress: async (_percentage, _stage, _status, data) => {
        if (data && typeof data === 'object') {
          const value = (data as { unknownEntityCount?: number })
            .unknownEntityCount
          if (typeof value === 'number') unknownEntities = value
        }
      }
    })
    const totalMs = performance.now() - t0

    const modelSpace = db.tables.blockTable.modelSpace
    const entities = modelSpace ? modelSpace.newIterator().count : 0

    bench.value = {
      fileName: file.name,
      totalMs,
      entities,
      unknownEntities
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    benching.value = false
  }
}

const benchPicked = () => {
  if (picked.value) runBench(picked.value)
}

const openInViewer = () => {
  if (picked.value) viewerFile.value = picked.value
}
</script>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  height: 100vh;
  font-family: system-ui, sans-serif;
}
.bar {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 16px;
  border-bottom: 1px solid #ddd;
  flex-wrap: wrap;
}
.bar h1 {
  font-size: 16px;
  margin: 0 16px 0 0;
  white-space: nowrap;
}
.bar button {
  padding: 4px 12px;
  cursor: pointer;
}
.error {
  color: #c0392b;
}
.hint {
  padding: 12px 16px;
  color: #444;
  max-width: 900px;
}
.bench {
  padding: 8px 16px;
}
.bench table {
  border-collapse: collapse;
}
.bench td {
  border: 1px solid #ddd;
  padding: 4px 10px;
  font-size: 13px;
}
.bench td:first-child {
  color: #555;
}
.bench .note {
  margin: 8px 0 0;
  font-size: 13px;
  color: #555;
  max-width: 900px;
}
.viewer {
  flex: 1;
  min-height: 0;
}
</style>
