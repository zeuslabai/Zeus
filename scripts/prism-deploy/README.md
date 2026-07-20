# PRISM deploy-on-merge

Kills the **"merged ≠ live / stale bundle"** bug class: when `main` moves, the
box rebuilds the gateway + frontend, swaps them into the live serving paths,
and runs a smoke test that **fails loud** if the deploy did not actually go
live. Verified against the live Linux deploy box `192.168.1.82`.

## Files

| File | Role |
|------|------|
| `redeploy.sh` | The whole deploy as one idempotent, auditable script: sync `origin/main` → build gateway (`cargo build --release -p prism-gateway`) → build frontend (`trunk build --release`) → atomic gateway swap → dist swap → nginx reload → **smoke test** → stamp `.last-deploy`. Any failed gate exits non-zero. |
| `redeploy-poll.sh` | B1 trigger. Compares remote `origin/main` SHA vs the last-deployed SHA in `.last-deploy`; runs `redeploy.sh` only on change. Zero GitHub config — lives entirely on the box. |
| `prism-redeploy.service` | oneshot unit that runs `redeploy-poll.sh` as `mike`. |
| `prism-redeploy.timer` | fires the poll every 60s. |

## The smoke test (why this is trustworthy)

After the swap, `redeploy.sh` asserts:

1. `curl :9080/health` == `ok` (gateway is up)
2. `curl :80/` == `200` (nginx serving)
3. **WASM freshness assert** — extract the `prism-app-<hash>_bg.wasm` name from
   the *freshly built* `app/dist/index.html`, then confirm nginx on `:80`
   serves *that exact* bundle and returns `200`. This deterministically catches
   the stale-bundle bug where `main` is clean but the box still serves a
   minutes-old WASM.

Proven on the live box: corrupting the served `index.html` to reference an old
bundle makes the assert fire (`[ FAIL ] STALE BUNDLE`) and the script exits
non-zero. Build happens *before* any swap, so a failed build never takes the
running service down.

## Install on the box

```bash
# scripts live in the oracles repo on the deploy box:
cp redeploy.sh redeploy-poll.sh /home/mike/oracles/scripts/
chmod +x /home/mike/oracles/scripts/redeploy*.sh

sudo cp prism-redeploy.service prism-redeploy.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now prism-redeploy.timer

# manual deploy any time:
/home/mike/oracles/scripts/redeploy.sh
```

## Assumptions (verified on 192.168.1.82)

- Repo `/home/mike/oracles`, remote `git@github.com:mikehash/oracles.git`
- Gateway package `prism-gateway` in `crates/`, runs as systemd
  `prism-gateway.service` from `/srv/prism/bin/prism-gateway` (root-owned →
  must `systemctl stop` before copy or "Text file busy")
- Frontend built by `trunk` from `app/` → served from `/srv/prism/dist`
- nginx `sites-available/prism`: `:80` root `/srv/prism/dist`, proxies
  `/v1`,`/health` → `:9080`
- Passwordless sudo for `mike`; toolchain in `~/.cargo/bin`

> `rsync` is intentionally **not** required — dist mirroring uses
> `find -delete` + `cp -a` because the box has no rsync.

## B2 (later, optional)

Swap the 60s poll for a GitHub Actions `on: push: branches: [main]` job on a
self-hosted runner that runs `redeploy.sh`. B1 gets ~90% of the value today
with no external dependency; B2 buys push-instant instead of ≤60s poll.
