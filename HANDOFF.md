# HANDOFF — acad-rust-converter（交接给本仓库的后续 Agent）

> 本文件是跨仓库交接说明。本仓库从 `cad-viewer` monorepo 的
> `packages/acad-rust-converter` 拆分而来，**阶段 A（独立成型 + 验证）已完成**，
> **阶段 B（在 cad-viewer monorepo 内做 npm 闭环替换）尚未开始**。
> 接手 Agent 请按 §1 → §2 → §9 的顺序阅读，再动手。

---

## 1. 一句话现状

| 项 | 状态 |
| --- | --- |
| npm 包 | `@njbestway/acad-rust-converter@0.1.0`（独立 semver）**尚未发布** |
| 构建 | wasm（cargo + wasm-bindgen 0.2.126）+ tsc + 3×vite **全绿** |
| 黄金值 | `高山.dwg`（AC1032）解码 `entityCount = 244,756` / `modelSpaceEntityCount = 2,199`（acadrust **0.6.6**），**原生 dump 与浏览器双证** |
| 自带 example | `examples/cad-viewer-example` 端到端 **PASS**：worker + 6.75MB wasm 从 npm `dist/` 经 `viteStaticCopy` 拷贝加载，直接 Parse 得 **2,199** model-space 实体，无 404、无 Console error |
| 未完成 | ① `git add` 暂存 → 用户 commit → `npm publish` 0.1.0；② monorepo 闭环替换（阶段 B，见 §10） |

---

## 2. 来源与背景

- 本仓库从 `e:\rustpro\cad-viewer` 的 `packages/acad-rust-converter` 拆出（源包在 monorepo 内为 `@mlightcad/acad-rust-converter@1.7.5`，但 **registry 上从未发布**，1.7.5 只是 monorepo 耦合版本号）。
- 它是 cad-viewer 的**主 DWG 解析器**：基于 [acadrust](https://github.com/hakanaktt/acadrust)（纯 Rust，MPL-2.0）编译为 WebAssembly，替代 GPL-3.0 的 `@mlightcad/libredwg-converter`，供 `AcDbDatabaseConverterManager` 的 DWG 槽使用。
- **拆分纪律（重要）**：不能先删 monorepo 里的包。必须**先发布 npm**，再由 monorepo 改为 npm 依赖（阶段 B）形成闭环；在 0.1.0 真正上 registry 之前，monorepo 保持可用、不得改动其 DWG 链路。
- 许可证：`(MIT AND MPL-2.0)`。本包自身代码 MIT；编译进 `acad-rust_bg.wasm` 的 acadrust 为 MPL-2.0。`LICENSE` / `NOTICE` 已在仓库根与包目录各一份。

---

## 3. 已锁定决策（勿再讨论）

| 项 | 决策 |
| --- | --- |
| 包名 / scope | `@njbestway/acad-rust-converter` |
| 版本策略 | 独立 semver，从 **0.1.0** 起（不延续 monorepo 的 1.7.5） |
| acadrust 依赖 | `acadrust = { version = "0.6.6", features = ["serde"] }`（crates.io；**彻底弃用**原 `Cargo.toml` 里标注 DO NOT COMMIT 的本地路径 `path = "E:/home/dxf-gis/acadrust"`） |
| wasm-bindgen | crate 固定 `=0.2.126`，本机 CLI 也必须是 **0.2.126**（glue schema 校验要求精确匹配） |
| peerDependency | `@mlightcad/data-model ^1.15.6` |
| 提交纪律 | **AI 只执行 `git add` 暂存，commit / publish 一律由用户执行** |

> 0.6.6 是修复 ACIS REGION framing 回归后的版本；`高山.dwg` 的黄金值已证明 0.6.6 解码与 0.6.4/main-tip 一致（244,756），无回归。

---

## 4. 仓库结构

```
E:\rustpro\acad-rust-converter\
├─ package.json                 # private workspace root, packageManager pnpm@10.33.4
├─ pnpm-workspace.yaml          # packages/*, examples/*; allowBuilds: esbuild: true
├─ tsconfig.base.json           # 根 tsconfig（包与 example 的 extends 目标）
├─ .npmrc                       # @njbestway / @mlightcad → registry.npmjs.org
├─ .pnpmrc                      # strict-peer-dependencies 等
├─ .gitignore                   # node_modules/lib/dist/target/wasm pkg 等
├─ LICENSE / NOTICE            # (MIT AND MPL-2.0)
├─ packages/acad-rust-converter/       # 可发布库
│  ├─ package.json             # @njbestway/acad-rust-converter@0.1.0, publishConfig.access=public
│  ├─ src/                     # 8 个 TS：index / AcDbAcadRustConverter / AcDbAcadRustNativeConverter /
│  │                           #   AcadRustSectionClient / registerAcadRustConverter / model/acadrust-model /
│  │                           #   worker.ts / main-thread.ts
│  ├─ rust/                    # Cargo.toml(0.6.6) / src/lib.rs / src/tianzheng.rs / src/bin/dump_fixture.rs
│  │                           #   examples/*.rs(20 个诊断 bin) / tests/r12_dwg_read.rs
│  ├─ scripts/build-wasm.mjs   # cargo build --release --target wasm32-unknown-unknown + wasm-bindgen → wasm/pkg
│  ├─ vite.config.main.ts      # ESM + UMD 库产物（index.ts 入口）
│  ├─ vite.config.worker.ts    # acad-rust-parser-worker.js（并把 wasm 拷进 dist）
│  ├─ vite.config.maint.ts     # acad-rust-converter-main-thread.js
│  ├─ __tests__/               # worker.spec / parity.spec / register.spec（fixtures 未 tracked，见 §8）
│  ├─ lib/  dist/  wasm/  rust/target/   # 均为生成物（gitignored）
└─ examples/cad-viewer-example/         # 端到端验证 example（见 §7）
```

---

## 5. 构建与发布

### 5.1 前置工具链

- `rustup` + `cargo`（本机实测 1.99）
- `rustup target add wasm32-unknown-unknown`
- `cargo install wasm-bindgen-cli --version 0.2.126`（**必须 0.2.126**）
- Node ≥ 18，pnpm 10.x

### 5.2 命令

```bash
pnpm install
pnpm --filter @njbestway/acad-rust-converter build
```

`build` 依次执行：

1. `node scripts/build-wasm.mjs`（cargo → wasm32 → wasm-bindgen → `wasm/pkg`）——首次约 5~6 分钟
2. `tsc`（产出 `lib/*.d.ts`）
3. `vite build -c vite.config.main.ts`（ESM + UMD）
4. `vite build -c vite.config.worker.ts`（worker，并拷 wasm 到 `dist/`）
5. `vite build -c vite.config.maint.ts`（main-thread ESM）

其他脚本：`build:wasm` / `build:js` / `build:main` / `build:worker` / `build:main-thread` / `clean`。
`prepublishOnly` 会自动 `build`。

### 5.3 已验证产物（A3 实测，字节数可作回归基线）

| 文件 | 大小 |
| --- | --- |
| `dist/acad-rust-converter.js` | 70,058 B |
| `dist/acad-rust-converter.umd.cjs` | 52,885 B |
| `dist/acad-rust-parser-worker.js` | 6,351 B |
| `dist/acad-rust-converter-main-thread.js` | 3,583 B |
| `dist/acad-rust_bg.wasm` | **6,756,807 B（6.75 MB）** |

`dist/` 的 worker + wasm + main-thread **随 npm 包发布**，消费者无需 Rust 工具链；只有从源码构建本包才需要。

### 5.4 发布（用户执行）

```bash
git -C E:\rustpro\acad-rust-converter add -A     # AI 可代做
# 用户 review 后 commit
pnpm --filter @njbestway/acad-rust-converter build
npm publish                                        # 用户执行，@njbestway 权限
```

发布后**记录确切的 0.1.0（或实际版本）**，阶段 B 需要精确 pin。

---

## 6. 运行时架构与部署接线（最容易踩坑的地方）

### 6.1 两条转换架构

- **Architecture A**：`AcDbAcadRustConverter` —— `dwg_to_dxf`（acadrust WASM）→ `AcDbNativeDxfConverter`（MIT，主线程）。
- **Architecture B**：`AcDbAcadRustNativeConverter` —— `dwg_open` → `CadDocument`（存于 wasm 内存，返回 handle）→ 分页拉取 section/entity JSON → 直接映射到 `AcDbDatabase`。**example 注册的是 B**。
- 公共 API：`createAcadRustConverter(config)` / `registerAcadRustConverter(config)`；`registerAcadRustConverter({ architecture: 'native' })` 会返回带 A 回退的包装器。

### 6.2 资源部署铁律

- **worker 与 wasm 必须同目录**：`src/worker.ts` 用
  `new URL('acad-rust_bg.wasm', self.location.href)` fetch wasm。
- 宿主（example/CLI）用 `viteStaticCopy` 从
  `node_modules/@njbestway/acad-rust-converter/dist` 把
  `acad-rust-parser-worker.js`、`acad-rust_bg.wasm`、`acad-rust-converter-main-thread.js`
  拷到**同一个**部署目录（example 用 `assets/`）。
- config 用法：
  - `assetBaseUrl: './assets/'` → 自动派生 worker / wasm / main-thread 三个 URL；
  - `parserWorkerUrl` / `wasmUrl` / `mainThreadModuleUrl` 可逐项覆盖。
- main-thread 路径（`useWorker:false`）是浏览器专用：用 `document.baseURI` 解析 URL，并动态 `import()` main-thread 模块。

### 6.3 独立调用必须绑定 working database

脱离宿主（没有 `MlCadViewer` 等初始化）直接 `new AcDbDatabase()` + `converter.read()` 时，**Hatch / Dimension 等查库实体会抛**：

```
The current working database must be set before using it!
```

修复模式（example 已落地，见 `examples/cad-viewer-example/src/App.vue`）：

```ts
import { acdbHostApplicationServices } from '@mlightcad/data-model'

const db = new AcDbDatabase()
acdbHostApplicationServices().workingDatabase = db   // read 之前
await converter.read(buffer, db, { ... })
```

> 纯 LINE 图纸不会触发，含 Hatch/Dimension 的图纸必触发——冒烟时别只测简单图。

---

## 7. 自带 example（examples/cad-viewer-example）

- 结构：`index.html` / `package.json` / `tsconfig.json` / `vite.config.ts` / `src/{main.ts,App.vue,registerAcadRust.ts,env.d.ts}`。
- 依赖：`@mlightcad/cad-viewer` 及装配包 **1.7.6** + `@mlightcad/data-model ^1.15.14` + `@njbestway/acad-rust-converter: workspace:*` + Vue/Element Plus。
- `vite.config.ts`：`viteStaticCopy` 把 `./node_modules/@njbestway/acad-rust-converter/dist` 的 worker/wasm/main-thread 拷到 `assets/`；`base: './'`。
- `src/registerAcadRust.ts`：镜像真实消费者写法——`new AcDbAcadRustNativeConverter({ useWorker: true, parserWorkerUrl })` 注册到 `AcDbFileType.DWG`，并做 `instanceof` 断言日志。
- 运行：

```bash
pnpm --filter @njbestway/acad-rust-converter build   # 先产出 dist（workspace 软链依赖它）
pnpm --filter cad-viewer-example build
pnpm --filter cad-viewer-example preview --port 4173
```

### 7.1 A4 冒烟实测结论（2026-10，浏览器实测）

- worker：`GET /assets/acad-rust-parser-worker.js` → 200，6,351 B，`new Worker(...)` 无 error。
- wasm：`GET /assets/acad-rust_bg.wasm` → 200，`application/wasm`，**6,756,807 B**，魔数 `00 61 73 6d`。
- `高山.dwg` → **Model-space entities = 2,199**（与原生 dump 完全一致），unmapped/skipped = 1，耗时约 9.3 s。
- 无 Console 红色错误、无 RuntimeError / OOM / abort。
- 修复 working-database 后，**上传 → 直接 Parse 即成功**（不再需要先点 Open in viewer）。

### 7.2 example 已知限制（非阻塞）

1. `MlCadViewer` 根节点为 `position: fixed; inset: 0; z-index: 2`，点过 "Open in viewer" 后会整屏覆盖顶部工具栏与结果表格（cad-viewer 1.7.6 的布局行为，非本包问题）。
2. preview 生产构建会过滤 info 级日志，`[acad-rust] DWG parser = ...` 在 Console 不可见；这不是注册失败，可用 `instanceof` 或功能结果佐证。
3. cad-viewer 1.7.6 的 peer 约束会报 warning（`three@>=0.185.1` 实得 0.172.0；`@vueuse/core@^11` 实得 15.0.0），不阻塞构建/运行。

---

## 8. 黄金值与验证方法（回归必跑）

### 8.1 原生 dump（最可靠，不依赖浏览器）

```bash
cargo run --release --features dumper --bin dump_fixture \
  --manifest-path E:\rustpro\acad-rust-converter\packages\acad-rust-converter\rust\Cargo.toml \
  -- E:\rustpro\cad-viewer\samples\高山.dwg <out-dir> 1
```

输出 `<out-dir>/meta.json`（含 `entityCount`）、各 section JSON、`entity-page-0.json`、每个非空块的 `block-entity-page-*.json`、以及供 parity 对拍的 `baseline.dxf`。

**高山.dwg 基准值（acadrust 0.6.6 实测）**：

```json
{
  "blockRecordCount": 537,
  "classCount": 26,
  "dimStyleCount": 20,
  "dwgSourceVersion": "AC1032",
  "entityCount": 244756,
  "layerCount": 212,
  "lineTypeCount": 68,
  "modelSpaceEntityCount": 2199,
  "objectCount": 1591,
  "textStyleCount": 40,
  "version": "AC1032",
  "vportCount": 1
}
```

> `dump_fixture` 有 `cfg!(target_arch="wasm32")` 守卫，只能原生运行；`dumper` feature 会引入 `anyhow`。

### 8.2 浏览器冒烟（example）

- 打开 `http://localhost:4173/`，上传 `E:\rustpro\cad-viewer\samples\高山.dwg`，直接 **Parse (WASM worker)**。
- 通过标准：Model-space entities = **2,199**、无页面红错、wasm 200 / 6,756,807 B、无 Console error。
- **preview 服务必须用独立长存进程**（见 §11.4），Bash 后台终端会被回收导致 `ERR_CONNECTION_REFUSED`。

### 8.3 单元测试

`__tests__/` 有 3 个 spec：worker、parity（2790 行）、register。parity 测试读取 `__tests__/fixtures/*.json`——**这些夹具未入库**，需先用 §8.1 的 `dump_fixture` 生成后放入对应目录。原 monorepo 的 `rust/_dump` 语料也未迁移（gitignore 且巨大）。

---

## 9. 阶段 A 完成清单（证据）

| 阶段 | 内容 | 状态 |
| --- | --- | --- |
| A1 | 仓库骨架（root package/workspace/tsconfig/.npmrc/.pnpmrc/.gitignore/README/LICENSE/NOTICE） | ✅ |
| A2 | 46 个 tracked 源文件迁移；5 个改动文件（`package.json` / `tsconfig.json` / `vite.config.main.ts` / `rust/Cargo.toml` / `README.md`）；tsconfig 改 extends `../../tsconfig.base.json`；vite.config.main 内联 `createLibEntryFileName`（不再依赖 monorepo 的 `../vite-config`） | ✅ |
| A3 | `pnpm install` + 隔离构建全绿（acadrust 0.6.6 编译 wasm32 无 API 断裂，约 5m47s；tsc、3×vite 通过；5 产物 + lib d.ts） | ✅ |
| A4 | example 8 文件 + `pnpm install` + build（`Copied 3 items`）+ 浏览器冒烟（2,199 / 无 404 / 无错误 / 直接 Parse 修复后 PASS） | ✅ |
| A5 | `git add` 暂存（**未做，见 §10.1**） | ⬜ |

---

## 10. 后续事项（交接）

### 10.1 立即：发布 0.1.0（用户执行）

1. `git -C E:\rustpro\acad-rust-converter add -A`（可代做暂存）
2. 用户 review 并 commit
3. `pnpm --filter @njbestway/acad-rust-converter build`
4. 用户 `npm publish`（`publishConfig.access: public`）
5. **记录实际发布版本号**，供阶段 B pin

### 10.2 阶段 B：cad-viewer monorepo 闭环替换（**发布成功后**，在 `e:\rustpro\cad-viewer` 执行）

> 前置：0.1.0 已在 registry 可见（`npm view @njbestway/acad-rust-converter@0.1.0`）。在此之前**不要动 monorepo**。

**B1 接线中枢改 scope（一处驱动全部拷贝）**
- `tools/worker-assets.mjs:51`：`ACADRUST_CONVERTER_PACKAGE = '@njbestway/acad-rust-converter'`
  → 自动重指向 3 个 example 的 `vite.config.ts`（`acadRustDist`）、`tools/copy-workers.mjs`、CLI 的 `copy-runner-assets.mjs`。
- `packages/cad-simple-viewer/src/app/AcApWorkerAssets.ts:57`（及 :26 注释）同步同值（文档镜像，保持一致）。

**B2 4 个消费者 package.json：`workspace:*` → registry 精确版**
```
"@mlightcad/acad-rust-converter": "workspace:*"  →  "@njbestway/acad-rust-converter": "0.1.0"
```
- `packages/cad-viewer-example/package.json`（约 :52）
- `packages/cad-simple-viewer-example/package.json`（约 :35）
- `packages/cad-simple-viewer-cli/package.json`（约 :61）
- `packages/cad-diff-viewer-example/package.json`（约 :24）

**B3 import 说明符改 scope**
`from '@mlightcad/acad-rust-converter'` → `from '@njbestway/acad-rust-converter'`：
- `packages/cad-viewer-example/src/registerAcadRust.ts:15`
- `packages/cad-simple-viewer-example/src/registerAcadRust.ts`
- `packages/cad-diff-viewer-example/src/registerAcadRust.ts:5`
- `packages/cad-simple-viewer-cli/runner/main.ts:1`

> `@mlightcad/cad-simple-viewer` 导出的**文件名常量** `ACADRUST_PARSER_WORKER_FILE` 不需要改（文件名不变）。

**B4 `cad-viewer-example/vite.config.ts` 摘除 dev 源码别名**
- `devSourcePackages` 去掉 `'acad-rust-converter'`（约 :167）。
- 别名正则去掉 `|acad-rust-converter`（约 :187）。
- 静态拷贝段（约 :255+）随 B1 常量自动生效，不改。

**B5 删除 monorepo 本地包 + 重算 lock**
- `git rm -r packages/acad-rust-converter`
- `pnpm install`（workspace 少 1 包，新包从 registry 拉入，lock 带 integrity）

**B6 回归 + 冒烟**
- `jest` 跑既有基线（acad-ts 迁出时为 1247 passed，以当下实际为准），确保零回归。
- 逐个 `pnpm --filter <example/cli> build`（含 vue-tsc），确认无悬空 import、worker+wasm 拷贝落地。
- 浏览器在 `cad-viewer-example` 开 `高山.dwg`：DWG 走 `@njbestway` 版 worker+wasm，实体数达 **2,199** model-space（总 244,756），Console 无 error。

**B7 收尾**
- 暂存卫生核查（无 `dist/`、临时文件混入），清理所有 `.tmp-stepN`。
- 汇总 monorepo 侧改动，交用户 commit。
- 沉淀记忆：acad-rust 迁出事实 + 0.6.6 pin + "拷贝中枢单常量"经验。

### 10.3 维护事项

- **升 acadrust**：改 `Cargo.toml` 版本 → 重新构建 wasm → 重跑 §8.1 黄金值 + §8.2 冒烟，两者都过才算升级成功。
- **升 wasm-bindgen**：crate 与 CLI 必须同步到同一精确版本，否则 glue schema 校验失败。
- **包体积**：wasm 6.75 MB 是 npm 包主体，发布前留意 `npm notice` 的包大小。
- **`__tests__/fixtures` 缺失**：CI 若跑 parity，需要先按 §8.1 生成夹具（可考虑加一个 `scripts/dump-fixtures.mjs`）。
- **example README 可补**：当前 example 无 README，可直接复用本文件 §7。

---

## 11. 环境与陷阱（AI Agent 特别注意）

### 11.1 工具链版本
- cargo 1.99、wasm32-unknown-unknown 已装、wasm-bindgen-cli **0.2.126**
- Node/pnpm：pnpm 10.33.4（`packageManager` 已锁定）

### 11.2 pnpm 10 默认忽略 build script
`esbuild` 的 postinstall 需要在 `pnpm-workspace.yaml` 显式放行：
```yaml
allowBuilds:
  esbuild: true
```
（本次已修好；不要重复写多个 `esbuild: true`，YAML 重复键会导致解析问题。）

### 11.3 工作区外写入
- Write/SearchReplace 只能写当前工作区（`e:\rustpro\cad-viewer`）；写本仓库请用
  **`.tmp-stepN` 暂存 + `Copy-Item`**，或 PowerShell 直接写。
- 本仓库根路径：`E:\rustpro\acad-rust-converter`。

### 11.4 preview 服务必须 detached
- Bash 工具的 background 终端会在轮次之间被回收，导致浏览器 `ERR_CONNECTION_REFUSED`。
- 正确做法（PowerShell）：
```powershell
$vite = 'E:\rustpro\acad-rust-converter\examples\cad-viewer-example\node_modules\vite\bin\vite.js'
$node = (Get-Command node).Source
Start-Process -FilePath $node -ArgumentList $vite,'preview','--port','4173','--strictPort' `
  -WorkingDirectory 'E:\rustpro\acad-rust-converter\examples\cad-viewer-example' `
  -RedirectStandardOutput '<log>' -RedirectStandardError '<err-log>' -WindowStyle Hidden
```
- `pnpm` 在本机是 `pnpm.ps1`，`Start-Process` 不能直接拉起，故用 `node vite.js`。

### 11.5 PowerShell 语法
- 不支持 `&&` 和 CMD 的 `&`，用 `;`。
- 管道里的 `$_` 常被外层 shell 吞掉；复杂逻辑写 `.ps1` 用 `powershell -File` 执行。
- 大输出用 `*>` 重定向到文件再 `Get-Content`，避免管道 `Select-Object -Last` 截断丢错。

### 11.6 build 的前置顺序
example 依赖 workspace 软链包的 `dist/`：
```
pnpm --filter @njbestway/acad-rust-converter build   # 先生成 dist
pnpm --filter cad-viewer-example build               # 再 build example
```
否则 `viteStaticCopy` 找不到 worker/wasm（`Copied 3 items` 是成功信号）。

### 11.7 曾出现过的坑（已修）
- `AcDbConversionProgressCallback` 要求 `Promise<void>`：`progress` 回调必须写 `async`，否则 `vue-tsc` 报 `TS2322`。
- example 直接 Parse 缺 working database：见 §6.3，已用 `acdbHostApplicationServices().workingDatabase = db` 修复。
- `pnpm-workspace.yaml` 的 `allowBuilds` 曾重复写 4 次，已清理为单个 `esbuild: true`。

---

## 12. 关键文件索引

| 用途 | 路径（相对本仓库） |
| --- | --- |
| 包入口 / 公共 API | `packages/acad-rust-converter/src/index.ts` |
| Architecture A | `packages/acad-rust-converter/src/AcDbAcadRustConverter.ts` |
| Architecture B（本体） | `packages/acad-rust-converter/src/AcDbAcadRustNativeConverter.ts` |
| worker（wasm 相对 fetch） | `packages/acad-rust-converter/src/worker.ts` |
| 注册器 / 回退包装 | `packages/acad-rust-converter/src/registerAcadRustConverter.ts` |
| Rust wasm 桥 | `packages/acad-rust-converter/rust/src/lib.rs` |
| 原生 dump 工具 | `packages/acad-rust-converter/rust/src/bin/dump_fixture.rs` |
| wasm 构建脚本 | `packages/acad-rust-converter/scripts/build-wasm.mjs` |
| example 接线 | `examples/cad-viewer-example/{vite.config.ts,src/main.ts,src/registerAcadRust.ts,src/App.vue}` |

---

## 13. 许可证

`(MIT AND MPL-2.0)`。本包自身代码 MIT；`acad-rust_bg.wasm` 内嵌 MPL-2.0 的 acadrust。
分发时必须保留 `LICENSE` 与 `NOTICE`（npm `files` 已包含）。
