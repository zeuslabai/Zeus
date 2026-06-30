# ZeusWeb Bundle Size Baseline

## Toolchain

| Tool | Version |
|------|---------|
| trunk | 0.21.1 (`trunk v0.21.1`) |
| wasm-opt | version 108 |
| Build date | Sat May 23 17:19:36 UTC 2026 |

## Build Command

```
cd apps/ZeusWeb && trunk build --release
```

Build result: ✅ success (`trunk 0.21.1` — 2m 16s compile, wasm-opt applied)

## Baseline Artifacts (`dist/`)

Raw output of `ls -lh dist/`:

```
total 5.2M
-rw-rw-r-- 1 mike mike 8.5K May 23 21:19 index.html
-rw-rw-r-- 1 mike mike  36K May 23 21:17 input-524452f2cf1a3305.css
-rw-rw-r-- 1 mike mike  76K May 23 21:19 zeus-web-7dae0a087dd763d.js
-rw-rw-r-- 1 mike mike 5.1M May 23 21:19 zeus-web-7dae0a087dd763d_bg.wasm
```

## Bundle Sizes

| Artifact | Raw | Gzipped |
|----------|-----|---------|
| `zeus-web-7dae0a087dd763d_bg.wasm` | 5.1M (5,349,376 bytes) | 1,778,107 bytes (~1.70 MB) |
| `zeus-web-7dae0a087dd763d.js` | 76K | 11,766 bytes (~11.5 KB) |
| `input-524452f2cf1a3305.css` | 36K | 6,796 bytes (~6.6 KB) |
| `index.html` | 8.5K | — |

### Raw `gzip -c <file> | wc -c` output

```
# WASM
dist/zeus-web-7dae0a087dd763d_bg.wasm → gzipped bytes: 1778107

# JS
dist/zeus-web-7dae0a087dd763d.js      → gzipped bytes: 11766

# CSS
dist/input-524452f2cf1a3305.css       → gzipped bytes: 6796
```

## Notes

- **Post-#65-C1 baseline** — diff_viewer component (245 LOC) added @ `6dd94b8c` is included in this build
- WASM optimized via `wasm-opt` (binaryen version 108) as part of trunk release pipeline
- CSS is Tailwind purged output (trunk PostCSS pipeline)
- JS is wasm-bindgen glue only (minimal — WASM carries all app logic)
- Total transfer weight (gzipped): ~1.72 MB (WASM dominates)
