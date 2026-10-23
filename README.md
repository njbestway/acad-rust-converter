# acad-rust-converter

DWG parser plugin for [@mlightcad/data-model](https://www.npmjs.com/package/@mlightcad/data-model), built on [acadrust](https://github.com/hakanaktt/acadrust) (pure Rust, compiled to WebAssembly, MPL-2.0). It transcodes DWG to a live `AcDbDatabase`, so any CAD viewer using the `AcDbDatabaseConverterManager` slot API can adopt it as its copyleft-free DWG reader — a replacement for the GPL-3.0 `@mlightcad/libredwg-converter`.

- **Library**: [`packages/acad-rust-converter`](packages/acad-rust-converter) → `@njbestway/acad-rust-converter`
- **Example app**: [`examples/cad-viewer-example`](examples/cad-viewer-example) — registers the converter as the default DWG parser for the `@mlightcad/cad-viewer` UI and deploys the worker + wasm via `viteStaticCopy`.

This repository was split out of the `cad-viewer` monorepo, where the same package served as the primary open-source DWG path for the viewer examples and CLI.

## Why WASM

`acadrust` decodes DWG entirely in Rust. Compiled to `wasm32-unknown-unknown` and driven from a TypeScript Web Worker, it keeps a multi-second parse off the UI thread and lets the WASM heap be reclaimed. Because the built `.wasm` and worker ship inside the npm `dist/`, **consumers do not need the Rust toolchain** — only building this package from source does.

A main-thread transcoder is also emitted for embedders that cannot spawn a worker (`useWorker: false`).

## Quick start

```bash
pnpm install
pnpm --filter @njbestway/acad-rust-converter build   # needs Rust toolchain (see package README)
pnpm --filter cad-viewer-example dev                 # open a DWG through the registered parser
```

Building from source requires `rustup` + `cargo`, the `wasm32-unknown-unknown` target, and `wasm-bindgen-cli` pinned to **0.2.126** (must match the crate dependency, or the glue schema check fails). Full details are in [`packages/acad-rust-converter/README.md`](packages/acad-rust-converter/README.md).

## Repository layout

```
packages/acad-rust-converter/   # the publishable library (src/ TS + rust/ WASM bridge + scripts/ + 3 vite configs)
examples/cad-viewer-example/    # consumer app: proves the npm artifact + worker/wasm deploy path end-to-end
```

## License

`(MIT AND MPL-2.0)`. This package's own code is MIT; the compiled `acad-rust_bg.wasm` embeds MPL-2.0 `acadrust`. See [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE).
