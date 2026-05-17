# Zeus FreeBSD Port — Plan
## `scripts/packaging/freebsd/`

### Goal
Package the pre-built Zeus binary for FreeBSD in two forms:
1. **Standalone .txz package** — installable with `pkg add zeus-<version>.txz`
2. **Ports tree entry** — drop-in for `/usr/ports/sysutils/zeus`

---

### Files

| File | Purpose |
|------|---------|
| `Makefile` | Ports tree entry (bsd.port.mk) — `make install` target |
| `pkg-descr` | COMMENT (1-line) + description for pkg |
| `pkg-plist` | Packing list — files installed + permissions |
| `build-port.sh` | Script to produce a `.txz` package |

### Architecture Support
- `amd64` (x86_64) — primary
- `aarch64` (arm64) — secondary
- Falls back to `${ARCH}` for others

### Package Contents
- `/usr/local/bin/zeus` — binary (stripped)
- `/usr/local/man/man1/zeus.1` — generated man page
- `/usr/local/share/bash-completion/completions/zeus` — bash completions
- `/usr/local/share/zsh/site-functions/_zeus` — zsh completions
- `/usr/local/share/fish/vendor_completions.d/zeus.fish` — fish completions
- `/usr/local/share/doc/zeus/LICENSE` — MIT OR Apache-2.0
- sample config if present

### Build Flow
```
build-port.sh (binary) → stage tree → pkg create → dist/zeus-<version>.txz
```
No compilation — pure binary packaging.

### Next Steps (if needed)
- [ ] Add `sysutils/zeus/Makefile` stub for real ports tree submission
- [ ] Add `distinfo` generation (SHA256 of distributed binary)
- [ ] Publish binary releases at `https://github.com/zeuslabai/Zeus/releases`
- [ ] Set up CI: on tag, run `build-port.sh` → attach .txz to GitHub Release