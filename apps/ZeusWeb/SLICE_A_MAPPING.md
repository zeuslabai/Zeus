# Slice A — onboarding_wizard.rs `.z-*` → Tailwind utility mapping

Source-grounded against `input.css` (verified). Base: `d57bf83`. 20 distinct classes, 117 uses.
Reuse existing `@theme` tokens (catch #2): `z-text`/`z-surface`/`z-border-active`/`z-accent` already in `tailwind.css`.
Preserve `font-rajdhani: 'Rajdhani', monospace` for input-field (catch #3 — source says monospace).

## Spacing / layout (mechanical)
| `.z-*` (uses) | source | Tailwind |
|---|---|---|
| `z-mb20` (16) | margin-bottom:20px | `mb-5` |
| `z-mb16` (4) | margin-bottom:16px | `mb-4` |
| `z-flex1` (10) | flex:1 | `flex-1` |
| `z-row-gap14` (5) | flex; items-center; gap:14px | `flex items-center gap-[14px]` |
| `z-row-between-base` (5) | flex; justify-between; items-baseline; mb:4px | `flex justify-between items-baseline mb-1` |
| `z-grid-2col` (3) | grid; cols 1fr 1fr; gap:10px | `grid grid-cols-2 gap-2.5` |
| `z-col-gap6` (3) | flex-col; gap:6px | `flex flex-col gap-1.5` |

## Typography
| `.z-*` (uses) | source | Tailwind |
|---|---|---|
| `z-step-body` (11) | 14px; rgba(fg,.7); mt:10px; lh:1.7 | `text-sm text-z-text/70 mt-2.5 leading-[1.7]` |
| `z-orb-accent-label` (10) | Orbitron; 10px; ls:3px; rgba(accent,.8); mb:12px; 700 | `font-orbitron text-[10px] tracking-[3px] text-z-accent/80 mb-3 font-bold` |
| `z-orb-title-22` (9) | Orbitron; 22px; 600; rgba(fg,.9) | `font-orbitron text-[22px] font-semibold text-z-text` |
| `z-raj-title-22` (3) | Rajdhani; 22px; 600; rgba(fg,.9) | `font-rajdhani text-[22px] font-semibold text-z-text` |
| `z-orb-field-label` (3) | Orbitron; 10px; ls:3px; rgba(fg,.7); block; mb:8px; uppercase | `font-orbitron text-[10px] tracking-[3px] text-z-text/70 block mb-2 uppercase` |
| `z-fs13-bold` (5) | 13px; 700; rgba(fg,.8) | `text-[13px] font-bold text-z-text/80` |
| `z-fs11-dim35` (5) | 11px; rgba(fg,.35) | `text-[11px] text-z-text/35` |

## Components
| `.z-*` (uses) | source | Tailwind |
|---|---|---|
| `z-input-field` (7) | w-full; border-box; bg rgba(fff,.04); 1px border rgba(accent,.15); radius 8px; pad 9px 14px; 13px; rgba(fg,.9); Rajdhani **monospace**; outline none | `w-full box-border bg-white/[0.04] border border-z-accent/15 rounded-lg px-3.5 py-[9px] text-[13px] text-z-text font-rajdhani outline-none` |

## Single-prop tokens (map to existing @theme)
| `.z-*` (uses) | source | Tailwind |
|---|---|---|
| `z-text` (1) | color var(--z-text) | `text-z-text` |
| `z-surface` (1) | bg var(--z-surface) | `bg-z-surface` |
| `z-border` (1) | border-color var(--z-border) | `border-z-border` |
| `z-border-active` (1) | border-color var(--z-border-active) | `border-z-border-active` |

## Animation — NEEDS @theme keyframe port (catch #3, the one non-trivial item)
`z-fade-in` (14): `opacity:0; animation: fadeIn 1.2s cubic-bezier(0.16,1,0.3,1) forwards`
`@keyframes fadeIn { from {opacity:0; translateY(16px)} to {opacity:1; translateY(0)} }`

**Port to tailwind.css `@theme`:**
```css
@theme {
  --animate-fade-in: fadeIn 1.2s cubic-bezier(0.16,1,0.3,1) forwards;
}
@keyframes fadeIn {
  from { opacity: 0; transform: translateY(16px); }
  to   { opacity: 1; transform: translateY(0); }
}
```
Then `z-fade-in` → `opacity-0 animate-fade-in`. (Keyframe name stays `fadeIn`, utility is `animate-fade-in`.)

## Cut order
1. Add the `@theme` keyframe block to `tailwind.css` FIRST (unblocks z-fade-in's 14 uses).
2. Replace the 19 static classes (mechanical sed/edit).
3. Replace `z-fade-in` → `opacity-0 animate-fade-in`.
4. `npx @tailwindcss/cli -i tailwind.css -o /tmp/check.css` → grep-verify all values resolve.
5. Full `trunk build` (build the BUNDLE, not just CLI output).
6. 5-axis self-verify → push slice branch → coord 4-axis ratify.
