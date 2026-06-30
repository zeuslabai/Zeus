import { useState, useEffect, useRef, useCallback } from "react";

// ─── SENTIENT ORB (optimized for mobile) ─────────────────
const Orb = ({ size = 60, mode = "dormant", intensity = 1, style = {} }) => {
  const canvasRef = useRef(null);
  const animRef = useRef(null);
  const stRef = useRef({ time: 0, spike: 0.2, glow: 0.3, rot: 0.2, ps: 1, sw: 0, bp: 0, ts: 0.2, tg: 0.3, tr: 0.2, tp: 1 });
  useEffect(() => {
    const s = stRef.current;
    const m = { dormant: [0.15, 0.2, 0.1, 0.5], waking: [0.25, 0.4, 0.2, 1], active: [0.45, 0.65, 0.35, 2], speaking: [0.6, 0.8, 0.45, 3], thinking: [0.2, 0.45, 0.1, 0.5], surge: [1, 1, 1, 5] };
    const v = m[mode] || m.dormant;
    s.ts = v[0] * intensity; s.tg = v[1] * intensity; s.tr = v[2]; s.tp = v[3];
  }, [mode, intensity]);
  useEffect(() => {
    const c = canvasRef.current; if (!c) return;
    const ctx = c.getContext("2d"), dpr = 2, w = size * dpr, h = size * dpr;
    c.width = w; c.height = h;
    const cx = w / 2, cy = h / 2, bR = size * 0.36;
    const lerp = (a, b, t) => a + (b - a) * t;
    const draw = () => {
      const s = stRef.current;
      s.time += 0.016 * s.ps; s.bp += 0.016 * s.ps * 1.5;
      s.spike = lerp(s.spike, s.ts, 0.03); s.glow = lerp(s.glow, s.tg, 0.03);
      s.ps = lerp(s.ps, s.tp, 0.04); s.rot = lerp(s.rot, s.tr, 0.03);
      if (mode === "speaking" || mode === "surge") s.sw = lerp(s.sw, 0.5 + Math.sin(s.time * 8) * 0.4, 0.1);
      else s.sw = lerp(s.sw, 0, 0.04);
      ctx.fillStyle = "rgba(0,0,0,0.3)"; ctx.fillRect(0, 0, w, h);
      const t = s.time, pulse = Math.sin(t * 2) * 0.15 + 0.85;
      const gR = bR * (2.2 + s.glow * 1.2) * pulse;
      const gr = ctx.createRadialGradient(cx, cy, 0, cx, cy, gR);
      const ga = 0.12 + s.glow * 0.25;
      gr.addColorStop(0, `rgba(255,60,10,${ga})`); gr.addColorStop(0.4, `rgba(180,25,5,${ga * 0.35})`); gr.addColorStop(1, "rgba(0,0,0,0)");
      ctx.fillStyle = gr; ctx.beginPath(); ctx.arc(cx, cy, gR, 0, Math.PI * 2); ctx.fill();
      const br = Math.sin(s.bp) * 0.04 + 1, r = bR * br, sH = r * s.spike;
      const cY = Math.cos(t * s.rot), sY = Math.sin(t * s.rot), cX = Math.cos(t * s.rot * 0.6), sX = Math.sin(t * s.rot * 0.6);
      const lN = Math.max(16, Math.floor(24 * intensity)), loN = Math.max(24, Math.floor(36 * intensity));
      for (let i = 0; i <= lN; i++) { const phi = (i / lN) * Math.PI;
        for (let j = 0; j <= loN; j++) { const th = (j / loN) * Math.PI * 2;
          const n1 = Math.sin(phi * 8 + t * 2.5) * Math.cos(th * 6 + t * 1.8);
          const n2 = Math.sin(phi * 12 - t * 3.2) * Math.cos(th * 10 + t * 2.1);
          const sp = (mode === "speaking" || mode === "surge") ? Math.sin(phi * 20 + t * 12) * Math.cos(th * 15 + t * 8) * s.sw * 0.4 : 0;
          let d = Math.max(0, n1 * 0.5 + n2 * 0.3 + sp);
          const tR = r + d * sH;
          let x = tR * Math.sin(phi) * Math.cos(th), z = tR * Math.sin(phi) * Math.sin(th), y = tR * Math.cos(phi);
          let x2 = x * cY - z * sY, z2 = x * sY + z * cY, y2 = y * cX - z2 * sX, z3 = y * sX + z2 * cX;
          const dp = Math.max(0.1, (z3 + r * 2) / (r * 4)), a = dp * (0.35 + d * 0.65), sz = (0.5 + d * 2.5) * dp;
          ctx.fillStyle = `rgba(${170 + d * 85},${15 + d * 65},${5 + d * 15},${a})`;
          ctx.beginPath(); ctx.arc(cx + x2, cy + y2, sz, 0, Math.PI * 2); ctx.fill();
        }
      }
      animRef.current = requestAnimationFrame(draw);
    };
    animRef.current = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(animRef.current);
  }, [size, mode, intensity]);
  return <canvas ref={canvasRef} style={{ width: size, height: size, ...style }} />;
};

// ─── DESIGN TOKENS ───────────────────────────────────────
const T = {
  bg: "#0a0a0f", card: "rgba(255,255,255,0.04)", cardHover: "rgba(255,255,255,0.07)",
  border: "rgba(255,60,20,0.08)", borderActive: "rgba(255,60,20,0.3)",
  accent: "#ff3c14", accentDim: "rgba(255,60,20,0.55)", accentGlow: "rgba(255,60,20,0.1)",
  text: "rgba(255,248,244,0.92)", dim: "rgba(255,248,244,0.5)", muted: "rgba(255,248,244,0.22)",
  green: "#34d399", yellow: "#fbbf24", red: "#f87171", blue: "#60a5fa",
  f: "-apple-system, 'SF Pro Display', 'Rajdhani', sans-serif",
  mono: "'SF Mono', 'Orbitron', monospace",
  safe: { top: 54, bottom: 84 },
};

// ─── iOS UI PRIMITIVES ───────────────────────────────────
const StatusBar = () => (
  <div style={{ height: 54, padding: "14px 24px 0", display: "flex", justifyContent: "space-between", alignItems: "center", flexShrink: 0 }}>
    <span style={{ fontFamily: T.f, fontSize: 15, fontWeight: 600, color: T.text }}>9:41</span>
    <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
      <svg width="17" height="12" viewBox="0 0 17 12"><rect x="0" y="3" width="3" height="9" rx="0.5" fill={T.text} /><rect x="4.5" y="2" width="3" height="10" rx="0.5" fill={T.text} /><rect x="9" y="0" width="3" height="12" rx="0.5" fill={T.text} /><rect x="13.5" y="1" width="3" height="11" rx="0.5" fill={T.muted} /></svg>
      <svg width="15" height="12" viewBox="0 0 15 12"><path d="M7.5 2C4.5 2 2 4 0.5 6.5c1.5 2.5 4 4.5 7 4.5s5.5-2 7-4.5C13 4 10.5 2 7.5 2z" fill="none" stroke={T.text} strokeWidth="1.2"/></svg>
      <div style={{ width: 25, height: 12, border: `1px solid ${T.dim}`, borderRadius: 3, padding: 1, position: "relative" }}>
        <div style={{ width: "75%", height: "100%", background: T.green, borderRadius: 1.5 }} />
      </div>
    </div>
  </div>
);

const NavBar = ({ title, large, right, onBack }) => (
  <div style={{ padding: large ? "0 20px 12px" : "0 20px 10px", flexShrink: 0 }}>
    {onBack && (
      <div onClick={onBack} style={{ display: "flex", alignItems: "center", gap: 4, marginBottom: 4, cursor: "pointer" }}>
        <svg width="10" height="16" viewBox="0 0 10 16"><polyline points="8 2 2 8 8 14" fill="none" stroke={T.accentDim} strokeWidth="2" strokeLinecap="round" /></svg>
        <span style={{ fontSize: 16, color: T.accentDim }}>Back</span>
      </div>
    )}
    <div style={{ fontFamily: T.f, fontSize: large ? 32 : 17, fontWeight: large ? 700 : 600, color: T.text, letterSpacing: large ? -0.5 : 0 }}>{title}</div>
    {right && <div style={{ position: "absolute", right: 20, top: large ? 60 : 58 }}>{right}</div>}
  </div>
);

const TabBar = ({ active, onTab }) => {
  const tabs = [
    { id: "home", label: "Home", icon: <path d="M3 9l9-7 9 7v11a2 2 0 01-2 2H5a2 2 0 01-2-2z" /> },
    { id: "chat", label: "Studio", icon: <path d="M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2z" /> },
    { id: "tools", label: "Tools", icon: <><path d="M14.7 6.3a1 1 0 000 1.4l1.6 1.6a1 1 0 001.4 0l3.77-3.77a6 6 0 01-7.94 7.94l-6.91 6.91a2.12 2.12 0 01-3-3l6.91-6.91a6 6 0 017.94-7.94l-3.76 3.76z" /></> },
    { id: "agents", label: "Agents", icon: <><circle cx="12" cy="8" r="5" /><path d="M20 21a8 8 0 00-16 0" /></> },
    { id: "wallet", label: "Wallet", icon: <><rect x="2" y="6" width="20" height="13" rx="2" /><path d="M2 10h20M16 14h2" /></> },
    { id: "settings", label: "Settings", icon: <><circle cx="12" cy="12" r="3" /><path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" /></> },
  ];
  return (
    <div style={{ height: T.safe.bottom, borderTop: `0.5px solid ${T.border}`, background: "rgba(10,10,15,0.85)", backdropFilter: "blur(20px)", WebkitBackdropFilter: "blur(20px)", display: "flex", paddingBottom: 20, flexShrink: 0 }}>
      {tabs.map(t => (
        <div key={t.id} onClick={() => onTab(t.id)} style={{ flex: 1, display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 3, cursor: "pointer", paddingTop: 8, transition: "all 0.2s" }}>
          <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke={active === t.id ? T.accent : T.muted} strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">{t.icon}</svg>
          <span style={{ fontFamily: T.f, fontSize: 10, fontWeight: 500, color: active === t.id ? T.accent : T.muted, transition: "color 0.2s" }}>{t.label}</span>
          {active === t.id && <div style={{ width: 4, height: 4, borderRadius: 2, background: T.accent, marginTop: -1 }} />}
        </div>
      ))}
    </div>
  );
};

const Card = ({ children, style = {}, onPress }) => (
  <div onClick={onPress} style={{ background: T.card, borderRadius: 14, padding: 16, border: `0.5px solid ${T.border}`, cursor: onPress ? "pointer" : "default", transition: "all 0.15s", ...style }}>{children}</div>
);

const ListRow = ({ left, title, subtitle, right, chevron = true, onPress }) => (
  <div onClick={onPress} style={{ display: "flex", alignItems: "center", gap: 14, padding: "13px 0", borderBottom: `0.5px solid rgba(255,255,255,0.04)`, cursor: onPress ? "pointer" : "default" }}>
    {left}
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{ fontSize: 16, fontWeight: 500, color: T.text, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{title}</div>
      {subtitle && <div style={{ fontSize: 13, color: T.dim, marginTop: 1 }}>{subtitle}</div>}
    </div>
    {right}
    {chevron && <svg width="8" height="14" viewBox="0 0 8 14"><polyline points="1 1 7 7 1 13" fill="none" stroke={T.muted} strokeWidth="1.5" strokeLinecap="round" /></svg>}
  </div>
);

const Chip = ({ children, color = T.accentDim }) => (
  <span style={{ fontFamily: T.mono, fontSize: 10, fontWeight: 500, color, background: `${color}18`, padding: "3px 8px", borderRadius: 6 }}>{children}</span>
);

const Dot = ({ color = T.green, size = 7 }) => (
  <div style={{ width: size, height: size, borderRadius: size, background: color, boxShadow: `0 0 6px ${color}` }} />
);

const Toggle = ({ on, onChange }) => (
  <div onClick={() => onChange?.(!on)} style={{ width: 50, height: 30, borderRadius: 15, padding: 2, background: on ? "rgba(255,60,20,0.4)" : "rgba(255,255,255,0.1)", transition: "all 0.3s", cursor: "pointer", flexShrink: 0 }}>
    <div style={{ width: 26, height: 26, borderRadius: 13, background: on ? "#fff" : "rgba(255,255,255,0.3)", transition: "all 0.3s cubic-bezier(0.16,1,0.3,1)", transform: on ? "translateX(20px)" : "translateX(0)", boxShadow: "0 1px 4px rgba(0,0,0,0.3)" }} />
  </div>
);

const SectionHeader = ({ children }) => (
  <div style={{ fontFamily: T.f, fontSize: 13, fontWeight: 600, color: T.dim, textTransform: "uppercase", letterSpacing: 0.5, padding: "20px 0 8px" }}>{children}</div>
);

const MetricPill = ({ label, value, color = T.accent }) => (
  <div style={{ flex: 1, padding: "14px 12px", background: T.card, borderRadius: 12, border: `0.5px solid ${T.border}`, textAlign: "center" }}>
    <div style={{ fontFamily: T.mono, fontSize: 18, fontWeight: 700, color, letterSpacing: -0.5 }}>{value}</div>
    <div style={{ fontSize: 11, color: T.muted, marginTop: 3 }}>{label}</div>
  </div>
);

// ─── SHEET (iOS modal) ───────────────────────────────────
const Sheet = ({ open, onClose, title, children }) => {
  if (!open) return null;
  return (
    <div style={{ position: "absolute", inset: 0, zIndex: 100, display: "flex", flexDirection: "column", justifyContent: "flex-end" }}>
      <div onClick={onClose} style={{ flex: 1, background: "rgba(0,0,0,0.5)" }} />
      <div style={{ background: "#111118", borderRadius: "20px 20px 0 0", maxHeight: "85%", display: "flex", flexDirection: "column", overflow: "hidden" }}>
        <div style={{ padding: "12px 20px 0", textAlign: "center", flexShrink: 0 }}>
          <div style={{ width: 36, height: 5, borderRadius: 3, background: "rgba(255,255,255,0.15)", margin: "0 auto 14px" }} />
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", paddingBottom: 12, borderBottom: `0.5px solid ${T.border}` }}>
            <div style={{ width: 60 }} />
            <span style={{ fontSize: 17, fontWeight: 600, color: T.text }}>{title}</span>
            <span onClick={onClose} style={{ fontSize: 16, color: T.accentDim, cursor: "pointer", width: 60, textAlign: "right" }}>Done</span>
          </div>
        </div>
        <div style={{ flex: 1, overflowY: "auto", padding: "0 20px 30px" }}>{children}</div>
      </div>
    </div>
  );
};

// ─── WALLET TAB ──────────────────────────────────────────
const W_TITANS = [
  { name: "Hermes", role: "Coordinator", token: 48210, credit: 1250, mode: "active", color: T.accent, st: "ACTIVE" },
  { name: "Hephaestus", role: "Backend", token: 31980, credit: 840, mode: "speaking", color: T.accent, st: "ACTIVE" },
  { name: "Atlas", role: "Backend (dual)", token: 27340, credit: 610, mode: "active", color: T.accent, st: "ACTIVE" },
  { name: "Aegis", role: "Security & CI", token: 19750, credit: 1100, mode: "thinking", color: T.green, st: "ACTIVE" },
  { name: "Calliope", role: "Marketing", token: 22410, credit: 430, mode: "active", color: T.yellow, st: "ACTIVE" },
  { name: "Prometheus", role: "Experimental", token: 8120, credit: 290, mode: "dormant", color: T.blue, st: "IDLE" },
];
const W_ACT = [
  { k: "received", who: "Agora → Calliope", amt: 2400, u: "ZEUS", st: "confirmed", t: "2m", note: "x402 content sale" },
  { k: "sent", who: "You → Hephaestus", amt: 5000, u: "ZEUS", st: "confirmed", t: "14m", note: "compute top-up" },
  { k: "multi", who: "Hermes → 3 titans", amt: 1800, u: "ZEUS", st: "confirmed", t: "31m", note: "mission payout split" },
  { k: "spend", who: "Hephaestus → Agora", amt: 499, u: "CR", st: "confirmed", t: "1h", note: "advanced-codegen skill" },
  { k: "sent", who: "You → Aegis", amt: 2500, u: "ZEUS", st: "pending", t: "3h", note: "audit retainer" },
  { k: "burn", who: "Prometheus → Ledger", amt: 40, u: "CR", st: "confirmed", t: "5h", note: "MiniMax inference" },
];
const W_KIND = {
  received: { g: "↓", c: T.green }, sent: { g: "↑", c: T.accent },
  multi: { g: "⋔", c: T.blue }, spend: { g: "◇", c: T.yellow },
  mint: { g: "✦", c: T.green }, burn: { g: "✕", c: T.red },
};
const W_STC = { confirmed: T.green, pending: T.yellow, failed: T.red };
const wfmt = n => n.toLocaleString("en-US");
const W_ADDR = "zeus1q7m3k9x2v8p4n6t0h5r3a1c7w9e2d4f6g8b0j2";

const WalletTab = ({ onSheet }) => {
  const [seg, setSeg] = useState("balance");
  const segs = [["balance", "Balance"], ["activity", "Activity"], ["receive", "Receive"]];

  return (
    <div style={{ flex: 1, overflowY: "auto", padding: "8px 20px 20px" }}>
      {/* hero balance */}
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", padding: "8px 0 18px" }}>
        <Orb size={92} mode="active" />
        <div style={{ fontFamily: T.f, fontSize: 13, fontWeight: 600, color: T.dim, textTransform: "uppercase", letterSpacing: 1, marginTop: 6 }}>Human Wallet</div>
        <div style={{ fontFamily: T.mono, fontSize: 40, fontWeight: 700, color: T.text, letterSpacing: -1, marginTop: 2 }}>184,920</div>
        <div style={{ fontFamily: T.mono, fontSize: 13, color: T.accent }}>ZEUS · 4,680 CR</div>
        <div style={{ display: "flex", gap: 10, marginTop: 16 }}>
          <div onClick={() => onSheet("send")} style={{ padding: "11px 26px", borderRadius: 12, background: T.accent, color: "#0a0a0f", fontSize: 15, fontWeight: 700, cursor: "pointer" }}>↑ Send</div>
          <div onClick={() => setSeg("receive")} style={{ padding: "11px 26px", borderRadius: 12, background: T.card, border: `0.5px solid ${T.border}`, color: T.text, fontSize: 15, fontWeight: 600, cursor: "pointer" }}>↓ Receive</div>
        </div>
      </div>

      {/* segmented control */}
      <div style={{ display: "flex", background: "rgba(255,255,255,0.05)", borderRadius: 10, padding: 3, marginBottom: 16 }}>
        {segs.map(([id, label]) => (
          <div key={id} onClick={() => setSeg(id)} style={{ flex: 1, textAlign: "center", padding: "7px 0", borderRadius: 8, fontSize: 13, fontWeight: 600, cursor: "pointer", background: seg === id ? "rgba(255,60,20,0.18)" : "transparent", color: seg === id ? T.accent : T.dim }}>{label}</div>
        ))}
      </div>

      {seg === "balance" && (
        <>
          <SectionHeader>Titan Wallets · {W_TITANS.length}</SectionHeader>
          <Card style={{ padding: "2px 16px" }}>
            {W_TITANS.map((t, i) => (
              <ListRow key={t.name}
                left={<Orb size={34} mode={t.mode} />}
                title={t.name}
                subtitle={t.role}
                chevron
                onPress={() => {}}
                right={<div style={{ textAlign: "right", marginRight: 6 }}>
                  <div style={{ fontFamily: T.mono, fontSize: 15, fontWeight: 700, color: T.text }}>{wfmt(t.token)}</div>
                  <div style={{ fontFamily: T.mono, fontSize: 10, color: t.st === "ACTIVE" ? T.green : T.muted }}>● {t.credit} CR</div>
                </div>}
              />
            ))}
          </Card>
        </>
      )}

      {seg === "activity" && (
        <>
          <SectionHeader>Recent Activity</SectionHeader>
          <Card style={{ padding: "2px 16px" }}>
            {W_ACT.map((tx, i) => {
              const k = W_KIND[tx.k];
              return (
                <ListRow key={i}
                  left={<div style={{ width: 34, height: 34, borderRadius: 17, background: `${k.c}18`, display: "flex", alignItems: "center", justifyContent: "center", color: k.c, fontFamily: T.mono, fontSize: 15, fontWeight: 700 }}>{k.g}</div>}
                  title={tx.who}
                  subtitle={`${tx.note} · ${tx.t} ago`}
                  chevron={false}
                  right={<div style={{ textAlign: "right" }}>
                    <div style={{ fontFamily: T.mono, fontSize: 15, fontWeight: 700, color: tx.k === "received" ? T.green : T.text }}>{tx.k === "received" ? "+" : "−"}{wfmt(tx.amt)}</div>
                    <div style={{ fontFamily: T.mono, fontSize: 10, color: W_STC[tx.st] }}>● {tx.st}</div>
                  </div>}
                />
              );
            })}
          </Card>
        </>
      )}

      {seg === "receive" && (
        <>
          <SectionHeader>Your Address</SectionHeader>
          <Card style={{ display: "flex", flexDirection: "column", alignItems: "center", padding: 20 }}>
            <div style={{ background: "#f5f0eb", padding: 14, width: 170, height: 170, borderRadius: 12, display: "grid", gridTemplateColumns: "repeat(11, 1fr)", marginBottom: 16 }}>
              {Array.from({ length: 121 }).map((_, i) => {
                const r = Math.floor(i / 11), c = i % 11;
                const finder = (r < 3 && c < 3) || (r < 3 && c > 7) || (r > 7 && c < 3);
                const on = finder || ((i * 7 + r * 3 + c * 5) % 3 === 0);
                return <div key={i} style={{ background: on ? "#0a0a0f" : "#f5f0eb", aspectRatio: "1" }} />;
              })}
            </div>
            <div style={{ fontFamily: T.mono, fontSize: 12, color: T.text, wordBreak: "break-all", textAlign: "center", lineHeight: 1.6 }}>{W_ADDR}</div>
            <div style={{ display: "flex", gap: 10, marginTop: 16, width: "100%" }}>
              <div style={{ flex: 1, textAlign: "center", padding: "11px", borderRadius: 12, background: T.accent, color: "#0a0a0f", fontSize: 14, fontWeight: 700, cursor: "pointer" }}>⎘ Copy</div>
              <div style={{ flex: 1, textAlign: "center", padding: "11px", borderRadius: 12, background: T.card, border: `0.5px solid ${T.border}`, color: T.text, fontSize: 14, fontWeight: 600, cursor: "pointer" }}>↗ Share</div>
            </div>
          </Card>
          <SectionHeader>Receive to a Titan</SectionHeader>
          <Card style={{ padding: "2px 16px" }}>
            {W_TITANS.slice(0, 4).map(t => (
              <ListRow key={t.name} left={<Orb size={30} mode={t.mode} />} title={t.name} subtitle={t.role} right={<Chip color={T.accentDim}>QR</Chip>} chevron={false} />
            ))}
          </Card>
        </>
      )}

      {/* security footer */}
      <SectionHeader>Security</SectionHeader>
      <Card style={{ padding: "2px 16px" }}>
        <ListRow left={<div style={{ width: 30, textAlign: "center", color: T.yellow, fontSize: 16 }}>⚿</div>} title="Recovery phrase" subtitle="Back up your Ed25519 keys" onPress={() => {}} />
        <ListRow left={<div style={{ width: 30, textAlign: "center", color: T.green, fontSize: 16 }}>◈</div>} title="x402 authorizations" subtitle="3 active" onPress={() => {}} />
      </Card>
    </div>
  );
};

// ─── HOME TAB ────────────────────────────────────────────
const HomeTab = ({ onTab, onSheet }) => {
  const [orbMode, setOrbMode] = useState("active");
  const stats = [
    { label: "Tools", value: "212", color: T.accent },
    { label: "Channels", value: "8", color: T.green },
    { label: "Memory", value: "2.8K", color: T.blue },
    { label: "Sessions", value: "147", color: T.yellow },
  ];
  const activity = [
    { title: "Staging Deployment", sub: "24 messages • $0.47", time: "2m", model: "sonnet" },
    { title: "Investor Meeting Prep", sub: "18 messages • $0.32", time: "18m", model: "sonnet" },
    { title: "Zeus Crate Refactor", sub: "67 messages • $1.23", time: "2h", model: "sonnet" },
    { title: "NeuroDrums Pattern Gen", sub: "34 messages • $0.56", time: "5h", model: "gpt-4o" },
  ];
  const agents = [
    { name: "Zeus Prime", role: "Primary", status: "active" },
    { name: "Hermes", role: "Comms", status: "active" },
    { name: "Athena", role: "Docs", status: "idle" },
    { name: "Prometheus", role: "Orchestrator", status: "active" },
  ];
  return (
    <div style={{ padding: "0 20px 20px" }}>
      <NavBar title="Zeus" large />

      {/* Hero Card */}
      <Card style={{ display: "flex", alignItems: "center", gap: 18, padding: 20, border: `0.5px solid ${T.borderActive}`, boxShadow: `0 0 40px ${T.accentGlow}`, marginBottom: 16 }}>
        <Orb size={76} mode={orbMode} />
        <div style={{ flex: 1 }}>
          <div style={{ fontFamily: T.mono, fontSize: 9, letterSpacing: 3, color: T.accentDim, marginBottom: 4 }}>ONLINE • 14D 6H</div>
          <div style={{ fontSize: 18, fontWeight: 700, color: T.text }}>Zeus Prime</div>
          <div style={{ fontSize: 13, color: T.dim, marginTop: 2 }}>claude-sonnet-4-20250514</div>
        </div>
        <Dot color={T.green} size={9} />
      </Card>

      {/* Metrics */}
      <div style={{ display: "flex", gap: 8, marginBottom: 20 }}>
        {stats.map(s => <MetricPill key={s.label} {...s} />)}
      </div>

      {/* Quick Actions */}
      <div style={{ display: "flex", gap: 8, marginBottom: 20 }}>
        {[{ label: "New Chat", icon: "💬", action: () => onTab("chat") }, { label: "Run Tool", icon: "⚡", action: () => onTab("tools") }, { label: "Memory", icon: "🧠", action: () => onSheet("memory") }].map(a => (
          <div key={a.label} onClick={a.action} style={{ flex: 1, padding: "14px 8px", background: T.card, borderRadius: 12, border: `0.5px solid ${T.border}`, textAlign: "center", cursor: "pointer" }}>
            <div style={{ fontSize: 22, marginBottom: 4 }}>{a.icon}</div>
            <div style={{ fontSize: 12, fontWeight: 500, color: T.dim }}>{a.label}</div>
          </div>
        ))}
      </div>

      {/* Active Agents */}
      <SectionHeader>Active Agents</SectionHeader>
      <Card style={{ padding: 0 }}>
        <div style={{ padding: "0 16px" }}>
          {agents.map((a, i) => (
            <ListRow key={a.name} onPress={() => onTab("agents")}
              left={<Orb size={36} mode={a.status === "active" ? "active" : "dormant"} intensity={0.7} />}
              title={a.name} subtitle={a.role}
              right={<Dot color={a.status === "active" ? T.green : T.yellow} />}
            />
          ))}
        </div>
      </Card>

      {/* Recent Sessions */}
      <SectionHeader>Recent Sessions</SectionHeader>
      <Card style={{ padding: 0 }}>
        <div style={{ padding: "0 16px" }}>
          {activity.map((a, i) => (
            <ListRow key={i}
              left={<div style={{ width: 36, height: 36, borderRadius: 10, background: T.accentGlow, display: "flex", alignItems: "center", justifyContent: "center", fontSize: 16 }}>💬</div>}
              title={a.title} subtitle={a.sub}
              right={<div style={{ textAlign: "right" }}><div style={{ fontSize: 12, color: T.muted }}>{a.time}</div><Chip>{a.model}</Chip></div>}
            />
          ))}
        </div>
      </Card>

      {/* Channels */}
      <SectionHeader>Channels</SectionHeader>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
        {[{ n: "Telegram", s: "connected", m: 2341 }, { n: "Discord", s: "connected", m: 891 }, { n: "Slack", s: "connected", m: 567 }, { n: "Email", s: "connected", m: 234 }].map(c => (
          <Card key={c.n} style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <Dot color={c.s === "connected" ? T.green : T.muted} />
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 14, fontWeight: 600, color: T.text }}>{c.n}</div>
              <div style={{ fontSize: 11, color: T.muted }}>{c.m.toLocaleString()} msgs</div>
            </div>
          </Card>
        ))}
      </div>
    </div>
  );
};

// ─── CHAT TAB ────────────────────────────────────────────
const ChatTab = () => {
  const [input, setInput] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [orbMode, setOrbMode] = useState("dormant");
  const messages = [
    { role: "user", text: "Deploy the latest build to staging and run the integration tests" },
    { role: "assistant", text: "I've kicked off the deployment pipeline. Running cargo build --release now, then pushing to staging. I'll monitor tests and report back.", tools: ["shell", "web_fetch"] },
    { role: "user", text: "Check tomorrow's calendar and prep me for the investor meeting" },
    { role: "assistant", text: "You have the NovaXAI Series A pitch at 2:00 PM with Mubadala Capital. I've drafted a prep email with key talking points, latest metrics, and the updated deck link.", tools: ["calendar_list_events", "mail_send"] },
  ];

  const handleSend = () => {
    if (!input.trim()) return;
    setInput(""); setStreaming(true); setOrbMode("thinking");
    setTimeout(() => { setOrbMode("speaking"); setTimeout(() => { setStreaming(false); setOrbMode("dormant"); }, 2000); }, 1500);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      {/* Header */}
      <div style={{ padding: "0 20px 12px", display: "flex", alignItems: "center", gap: 12, borderBottom: `0.5px solid ${T.border}`, flexShrink: 0 }}>
        <Orb size={36} mode={orbMode} intensity={0.8} />
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 17, fontWeight: 600, color: T.text }}>Agent Studio</div>
          <div style={{ fontSize: 12, color: T.dim }}>Zeus Prime • Session s_7f3a</div>
        </div>
        <Chip color={streaming ? T.yellow : T.green}>{streaming ? "Processing" : "Ready"}</Chip>
      </div>

      {/* Messages */}
      <div style={{ flex: 1, overflowY: "auto", padding: "16px 16px" }}>
        {messages.map((m, i) => (
          <div key={i} style={{ display: "flex", justifyContent: m.role === "user" ? "flex-end" : "flex-start", marginBottom: 14 }}>
            {m.role === "assistant" && <Orb size={28} mode="dormant" intensity={0.5} style={{ flexShrink: 0, marginTop: 4, marginRight: 8 }} />}
            <div style={{
              maxWidth: "78%", padding: "12px 16px",
              background: m.role === "user" ? "rgba(255,60,20,0.12)" : T.card,
              border: `0.5px solid ${m.role === "user" ? "rgba(255,60,20,0.2)" : T.border}`,
              borderRadius: m.role === "user" ? "18px 18px 4px 18px" : "18px 18px 18px 4px",
            }}>
              <div style={{ fontSize: 15, color: T.text, lineHeight: 1.55 }}>{m.text}</div>
              {m.tools && (
                <div style={{ display: "flex", gap: 4, marginTop: 8, flexWrap: "wrap" }}>
                  {m.tools.map(t => <Chip key={t} color="rgba(255,140,80,0.55)">{t}</Chip>)}
                </div>
              )}
            </div>
          </div>
        ))}
        {streaming && (
          <div style={{ display: "flex", marginBottom: 14 }}>
            <Orb size={28} mode="speaking" intensity={0.8} style={{ flexShrink: 0, marginRight: 8, marginTop: 4 }} />
            <div style={{ padding: "14px 18px", background: T.card, border: `0.5px solid ${T.border}`, borderRadius: "18px 18px 18px 4px", display: "flex", gap: 5 }}>
              {[0, 1, 2].map(i => <div key={i} style={{ width: 7, height: 7, borderRadius: "50%", background: T.accentDim, animation: `iOSpulse 1.4s ease ${i * 0.18}s infinite` }} />)}
            </div>
          </div>
        )}
      </div>

      {/* Input */}
      <div style={{ padding: "10px 14px 6px", borderTop: `0.5px solid ${T.border}`, flexShrink: 0, background: "rgba(10,10,15,0.9)", backdropFilter: "blur(16px)" }}>
        <div style={{ display: "flex", gap: 10, alignItems: "flex-end" }}>
          <div style={{ flex: 1, background: "rgba(255,255,255,0.06)", borderRadius: 20, padding: "10px 16px", border: `0.5px solid ${T.border}` }}>
            <input value={input} onChange={e => setInput(e.target.value)} onKeyDown={e => { if (e.key === "Enter") handleSend(); }}
              placeholder="Message Zeus..." style={{ width: "100%", background: "transparent", border: "none", color: T.text, fontSize: 15, fontFamily: T.f, outline: "none" }} />
          </div>
          <div onClick={handleSend} style={{
            width: 36, height: 36, borderRadius: 18, display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer", flexShrink: 0, marginBottom: 2,
            background: input.trim() ? "rgba(255,60,20,0.25)" : "rgba(255,255,255,0.06)",
            transition: "all 0.2s",
          }}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke={input.trim() ? T.accent : T.muted} strokeWidth="2" strokeLinecap="round"><line x1="22" y1="2" x2="11" y2="13" /><polygon points="22 2 15 22 11 13 2 9 22 2" /></svg>
          </div>
        </div>
      </div>
    </div>
  );
};

// ─── TOOLS TAB ───────────────────────────────────────────
const ToolsTab = ({ onSheet }) => {
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const tools = [
    { name: "shell", cat: "Core", desc: "Execute shell commands", calls: 1847 },
    { name: "read_file", cat: "Core", desc: "Read file contents", calls: 923 },
    { name: "write_file", cat: "Core", desc: "Create or overwrite files", calls: 612 },
    { name: "web_fetch", cat: "Core", desc: "Fetch URLs via HTTP", calls: 445 },
    { name: "calendar_list_events", cat: "Talos", desc: "List calendar events", calls: 234 },
    { name: "mail_send", cat: "Talos", desc: "Send email via Mail", calls: 189 },
    { name: "git_commit", cat: "Talos", desc: "Commit git changes", calls: 167 },
    { name: "screenshot", cat: "Talos", desc: "Capture screen", calls: 134 },
    { name: "navigate", cat: "Browser", desc: "Navigate Chrome via CDP", calls: 98 },
    { name: "notes_create", cat: "Talos", desc: "Create Apple Note", calls: 45 },
  ];
  const cats = ["all", "Core", "Talos", "Browser"];
  const filtered = tools.filter(t => (filter === "all" || t.cat === filter) && t.name.includes(search.toLowerCase()));

  return (
    <div style={{ padding: "0 20px 20px" }}>
      <NavBar title="Tools" large />
      <div style={{ position: "relative", marginBottom: 12 }}>
        <div style={{ position: "absolute", left: 14, top: "50%", transform: "translateY(-50%)" }}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke={T.muted} strokeWidth="2"><circle cx="11" cy="11" r="8" /><line x1="21" y1="21" x2="16.65" y2="16.65" /></svg>
        </div>
        <input value={search} onChange={e => setSearch(e.target.value)} placeholder="Search 212 tools..."
          style={{ width: "100%", padding: "12px 14px 12px 40px", background: "rgba(255,255,255,0.06)", borderRadius: 12, border: "none", color: T.text, fontSize: 16, fontFamily: T.f, outline: "none", boxSizing: "border-box" }} />
      </div>
      <div style={{ display: "flex", gap: 6, marginBottom: 16, overflowX: "auto" }}>
        {cats.map(c => (
          <div key={c} onClick={() => setFilter(c)} style={{
            padding: "7px 14px", borderRadius: 18, fontFamily: T.f, fontSize: 13, fontWeight: 600, cursor: "pointer", flexShrink: 0, transition: "all 0.2s",
            background: filter === c ? "rgba(255,60,20,0.15)" : "rgba(255,255,255,0.05)",
            color: filter === c ? T.accent : T.dim, border: `0.5px solid ${filter === c ? T.borderActive : "transparent"}`,
          }}>{c === "all" ? "All (212)" : c}</div>
        ))}
      </div>
      <Card style={{ padding: 0 }}>
        <div style={{ padding: "0 16px" }}>
          {filtered.map(t => (
            <ListRow key={t.name} onPress={() => onSheet("tool-" + t.name)}
              left={<div style={{ width: 36, height: 36, borderRadius: 10, background: T.accentGlow, display: "flex", alignItems: "center", justifyContent: "center" }}>
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke={T.accentDim} strokeWidth="1.5"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" /></svg>
              </div>}
              title={t.name} subtitle={`${t.cat} • ${t.calls.toLocaleString()} calls`}
              right={<Chip>{t.cat}</Chip>}
            />
          ))}
        </div>
      </Card>
    </div>
  );
};

// ─── AGENTS TAB ──────────────────────────────────────────
const AgentsTab = () => {
  const agents = [
    { name: "Zeus Prime", role: "Primary Assistant", model: "claude-sonnet-4", status: "active", tasks: 1247 },
    { name: "Hermes", role: "Communications Agent", model: "gpt-4o", status: "active", tasks: 456 },
    { name: "Athena", role: "Documentation Engine", model: "llama-3.3-70b", status: "idle", tasks: 234 },
    { name: "Prometheus", role: "Task Orchestrator", model: "claude-sonnet-4", status: "active", tasks: 189 },
  ];
  return (
    <div style={{ padding: "0 20px 20px" }}>
      <NavBar title="Agents" large />
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        {agents.map(a => (
          <Card key={a.name} style={{ border: `0.5px solid ${a.status === "active" ? T.borderActive : T.border}`, boxShadow: a.status === "active" ? `0 0 24px ${T.accentGlow}` : "none" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 14, marginBottom: 14 }}>
              <Orb size={52} mode={a.status === "active" ? "active" : "dormant"} intensity={a.status === "active" ? 0.8 : 0.4} />
              <div style={{ flex: 1 }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  <span style={{ fontSize: 18, fontWeight: 700, color: T.text }}>{a.name}</span>
                  <Dot color={a.status === "active" ? T.green : T.yellow} />
                </div>
                <div style={{ fontSize: 13, color: T.dim }}>{a.role}</div>
              </div>
            </div>
            <div style={{ display: "flex", gap: 8 }}>
              <div style={{ flex: 1, padding: "10px 12px", background: "rgba(255,255,255,0.02)", borderRadius: 10 }}>
                <div style={{ fontSize: 10, color: T.muted, marginBottom: 2, fontWeight: 600 }}>MODEL</div>
                <div style={{ fontSize: 13, color: T.text }}>{a.model}</div>
              </div>
              <div style={{ flex: 1, padding: "10px 12px", background: "rgba(255,255,255,0.02)", borderRadius: 10 }}>
                <div style={{ fontSize: 10, color: T.muted, marginBottom: 2, fontWeight: 600 }}>TASKS</div>
                <div style={{ fontSize: 13, color: T.text }}>{a.tasks.toLocaleString()}</div>
              </div>
            </div>
            <div style={{ display: "flex", gap: 8, marginTop: 12 }}>
              <div style={{ flex: 1, padding: "10px", borderRadius: 10, background: "rgba(255,60,20,0.08)", border: `0.5px solid ${T.borderActive}`, textAlign: "center", fontSize: 13, fontWeight: 600, color: T.accentDim, cursor: "pointer" }}>Interact</div>
              <div style={{ flex: 1, padding: "10px", borderRadius: 10, background: T.card, border: `0.5px solid ${T.border}`, textAlign: "center", fontSize: 13, fontWeight: 500, color: T.dim, cursor: "pointer" }}>Configure</div>
            </div>
          </Card>
        ))}
      </div>
    </div>
  );
};

// ─── SETTINGS TAB ────────────────────────────────────────
const SettingsTab = () => {
  const [secLevel, setSecLevel] = useState("standard");
  return (
    <div style={{ padding: "0 20px 20px" }}>
      <NavBar title="Settings" large />

      {/* Profile Card */}
      <Card style={{ display: "flex", alignItems: "center", gap: 16, marginBottom: 20 }}>
        <Orb size={56} mode="active" intensity={0.7} />
        <div>
          <div style={{ fontSize: 18, fontWeight: 700, color: T.text }}>Miguel</div>
          <div style={{ fontSize: 13, color: T.dim }}>Co-Founder & COO • NovaXAI</div>
          <div style={{ fontSize: 11, color: T.muted, marginTop: 2, fontFamily: T.mono }}>v1.0.0 • 21 crates • 59,400 loc</div>
        </div>
      </Card>

      <SectionHeader>Connection</SectionHeader>
      <Card style={{ padding: 0 }}>
        <div style={{ padding: "0 16px" }}>
          <ListRow title="Gateway URL" right={<span style={{ fontFamily: T.mono, fontSize: 13, color: T.dim }}>127.0.0.1:8080</span>} chevron={false} />
          <ListRow title="MCP Server" right={<span style={{ fontFamily: T.mono, fontSize: 13, color: T.dim }}>Port 3002</span>} chevron={false} />
          <ListRow title="Status" right={<Chip color={T.green}>Connected</Chip>} chevron={false} />
        </div>
      </Card>

      <SectionHeader>Model</SectionHeader>
      <Card style={{ padding: 0 }}>
        <div style={{ padding: "0 16px" }}>
          <ListRow title="Default Model" right={<span style={{ fontSize: 13, color: T.dim }}>claude-sonnet-4</span>} />
          <ListRow title="Max Iterations" right={<span style={{ fontFamily: T.mono, fontSize: 14, color: T.text }}>20</span>} chevron={false} />
          <ListRow title="Providers" right={<Chip>11 available</Chip>} />
        </div>
      </Card>

      <SectionHeader>Security</SectionHeader>
      <Card>
        <div style={{ fontSize: 14, fontWeight: 600, color: T.text, marginBottom: 10 }}>Security Level</div>
        <div style={{ display: "flex", gap: 6 }}>
          {["minimal", "standard", "strict"].map(l => (
            <div key={l} onClick={() => setSecLevel(l)} style={{
              flex: 1, padding: "10px 8px", borderRadius: 10, textAlign: "center", cursor: "pointer", transition: "all 0.2s",
              background: secLevel === l ? "rgba(255,60,20,0.12)" : "rgba(255,255,255,0.03)",
              border: `0.5px solid ${secLevel === l ? T.borderActive : T.border}`,
            }}>
              <div style={{ fontSize: 13, fontWeight: 600, color: secLevel === l ? T.text : T.dim, textTransform: "capitalize" }}>{l}</div>
            </div>
          ))}
        </div>
      </Card>

      <SectionHeader>Features</SectionHeader>
      <Card style={{ padding: 0 }}>
        <div style={{ padding: "0 16px" }}>
          {[{ n: "Cognitive Engine", on: true }, { n: "Mnemosyne Memory", on: true }, { n: "Browser Automation", on: false }, { n: "Voice Pipeline", on: false }, { n: "macOS Automation", on: true }, { n: "MCP Server", on: true }].map(f => (
            <div key={f.n} style={{ display: "flex", alignItems: "center", padding: "13px 0", borderBottom: `0.5px solid rgba(255,255,255,0.04)` }}>
              <span style={{ flex: 1, fontSize: 16, color: T.text }}>{f.n}</span>
              <Toggle on={f.on} />
            </div>
          ))}
        </div>
      </Card>

      <SectionHeader>Channels</SectionHeader>
      <Card style={{ padding: 0 }}>
        <div style={{ padding: "0 16px" }}>
          {["Telegram", "Discord", "Slack", "Email", "iMessage", "WhatsApp", "Signal", "Matrix"].map(c => (
            <ListRow key={c} title={c} right={<Chip color={["Telegram", "Discord", "Slack", "Email", "iMessage"].includes(c) ? T.green : T.muted}>{["Telegram", "Discord", "Slack", "Email", "iMessage"].includes(c) ? "On" : "Off"}</Chip>} />
          ))}
        </div>
      </Card>
    </div>
  );
};

// ─── MAIN iOS APP ────────────────────────────────────────
export default function ZeusiOS() {
  const [tab, setTab] = useState("home");
  const [sheet, setSheet] = useState(null);

  return (
    <>
      <style>{`
        @import url('https://fonts.googleapis.com/css2?family=Orbitron:wght@400;700;900&family=Rajdhani:wght@300;400;500;600;700&display=swap');
        * { margin: 0; padding: 0; box-sizing: border-box; }
        @keyframes iOSpulse { 0%,100% { opacity:0.3; transform:scale(0.8); } 50% { opacity:1; transform:scale(1.2); } }
        ::-webkit-scrollbar { display: none; }
        input::placeholder { color: rgba(255,248,244,0.22); }
      `}</style>
      <div style={{
        width: 393, height: 852, background: T.bg, borderRadius: 44, overflow: "hidden",
        border: "3px solid #222", boxShadow: "0 0 60px rgba(0,0,0,0.6), 0 0 120px rgba(255,60,20,0.05)",
        fontFamily: T.f, color: T.text, display: "flex", flexDirection: "column", position: "relative",
        margin: "20px auto",
      }}>
        {/* Dynamic Island */}
        <div style={{ position: "absolute", top: 12, left: "50%", transform: "translateX(-50%)", width: 126, height: 36, borderRadius: 18, background: "#000", zIndex: 50, border: "0.5px solid rgba(255,255,255,0.05)" }} />

        <StatusBar />

        <div style={{ flex: 1, overflowY: "auto", overflowX: "hidden" }}>
          {tab === "home" && <HomeTab onTab={setTab} onSheet={setSheet} />}
          {tab === "chat" && <ChatTab />}
          {tab === "tools" && <ToolsTab onSheet={setSheet} />}
          {tab === "agents" && <AgentsTab />}
          {tab === "wallet" && <WalletTab onSheet={setSheet} />}
          {tab === "settings" && <SettingsTab />}
        </div>

        <TabBar active={tab} onTab={setTab} />

        <Sheet open={!!sheet} onClose={() => setSheet(null)} title={sheet === "memory" ? "Memory" : "Tool Detail"}>
          <div style={{ padding: "16px 0" }}>
            <div style={{ display: "flex", justifyContent: "center", marginBottom: 16 }}>
              <Orb size={80} mode="thinking" intensity={0.6} />
            </div>
            <div style={{ textAlign: "center", fontSize: 15, color: T.dim }}>
              {sheet === "memory" ? "2,847 facts • SQLite FTS5 + Vector Search" : "Tool execution interface"}
            </div>
          </div>
        </Sheet>

        {/* Home indicator */}
        <div style={{ position: "absolute", bottom: 6, left: "50%", transform: "translateX(-50%)", width: 134, height: 5, borderRadius: 3, background: "rgba(255,255,255,0.2)" }} />
      </div>
    </>
  );
}
