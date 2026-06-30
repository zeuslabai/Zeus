import { useState, useEffect, useRef } from "react";

// ─── SENTIENT ORB (compact) ──────────────────────────────
const Orb = ({ size = 60, mode = "dormant", intensity = 1, style = {} }) => {
  const ref = useRef(null), anim = useRef(null);
  const st = useRef({ time: 0, spike: 0.15, glow: 0.2, rot: 0.1, ps: 0.5, sw: 0, bp: 0, ts: 0.15, tg: 0.2, tr: 0.1, tp: 0.5 });
  useEffect(() => {
    const s = st.current;
    const m = { dormant:[.08,.1,.05,.3], waking:[.2,.3,.15,.8], active:[.45,.65,.35,2], speaking:[.6,.85,.5,3], thinking:[.2,.45,.1,.5], surge:[1,1,1,5], listening:[.3,.5,.25,1.2] };
    const v = m[mode]||m.dormant; s.ts=v[0]*intensity; s.tg=v[1]*intensity; s.tr=v[2]; s.tp=v[3];
  }, [mode, intensity]);
  useEffect(() => {
    const c = ref.current; if(!c) return;
    const ctx = c.getContext("2d"), dpr = 2, w = size*dpr, h = size*dpr;
    c.width = w; c.height = h;
    const cx = w/2, cy = h/2, bR = size*.36;
    const lerp = (a,b,t) => a+(b-a)*t;
    const draw = () => {
      const s = st.current;
      s.time += .016*s.ps; s.bp += .016*s.ps*1.5;
      s.spike = lerp(s.spike,s.ts,.025); s.glow = lerp(s.glow,s.tg,.025);
      s.ps = lerp(s.ps,s.tp,.03); s.rot = lerp(s.rot,s.tr,.025);
      if(mode==="speaking"||mode==="surge") s.sw = lerp(s.sw,.5+Math.sin(s.time*8)*.4,.1);
      else s.sw = lerp(s.sw,0,.04);
      ctx.fillStyle = "rgba(0,0,0,.28)"; ctx.fillRect(0,0,w,h);
      const t = s.time, pulse = Math.sin(t*2)*.15+.85;
      const gR = bR*(2.2+s.glow*1.2)*pulse;
      const gr = ctx.createRadialGradient(cx,cy,0,cx,cy,gR);
      const ga = .12+s.glow*.25;
      gr.addColorStop(0,`rgba(255,60,10,${ga})`); gr.addColorStop(.4,`rgba(180,25,5,${ga*.35})`); gr.addColorStop(1,"rgba(0,0,0,0)");
      ctx.fillStyle = gr; ctx.beginPath(); ctx.arc(cx,cy,gR,0,Math.PI*2); ctx.fill();
      const cageR = bR*1.5;
      ctx.strokeStyle = `rgba(255,50,20,${.02+s.glow*.03})`; ctx.lineWidth = .4;
      const csy = Math.cos(t*s.rot*.3), sny = Math.sin(t*s.rot*.3);
      for(let i=1;i<10;i++){const phi=(i/10)*Math.PI,rr=cageR*Math.sin(phi),yy=cageR*Math.cos(phi);ctx.beginPath();for(let j=0;j<=18;j++){const th=(j/18)*Math.PI*2;let x=rr*Math.cos(th),z=rr*Math.sin(th);ctx.lineTo(cx+x*csy-z*sny,cy+yy)}ctx.stroke()}
      const br = Math.sin(s.bp)*.04+1, r = bR*br, sH = r*s.spike;
      const cY=Math.cos(t*s.rot),sY=Math.sin(t*s.rot),cX=Math.cos(t*s.rot*.6),sX=Math.sin(t*s.rot*.6);
      const lN=Math.max(16,Math.floor(24*intensity)),loN=Math.max(24,Math.floor(36*intensity));
      for(let i=0;i<=lN;i++){const phi=(i/lN)*Math.PI;for(let j=0;j<=loN;j++){const th=(j/loN)*Math.PI*2;
        const n1=Math.sin(phi*8+t*2.5)*Math.cos(th*6+t*1.8),n2=Math.sin(phi*12-t*3.2)*Math.cos(th*10+t*2.1),n3=Math.sin(phi*4+th*5+t*1.5);
        const sp=(mode==="speaking"||mode==="surge")?Math.sin(phi*20+t*12)*Math.cos(th*15+t*8)*s.sw*.4:0;
        let d=Math.max(0,n1*.5+n2*.3+n3*.15+sp); const tR=r+d*sH;
        let x=tR*Math.sin(phi)*Math.cos(th),z=tR*Math.sin(phi)*Math.sin(th),y=tR*Math.cos(phi);
        let x2=x*cY-z*sY,z2=x*sY+z*cY,y2=y*cX-z2*sX,z3=y*sX+z2*cX;
        const dp=Math.max(.1,(z3+r*2)/(r*4)),a=dp*(.35+d*.65),sz=(.5+d*2.5)*dp;
        if(d>.4){const gRR=sz*(2+s.glow*2);const gg=ctx.createRadialGradient(cx+x2,cy+y2,0,cx+x2,cy+y2,gRR);gg.addColorStop(0,`rgba(255,${50+d*80},10,${a*.3*s.glow})`);gg.addColorStop(1,"rgba(255,30,0,0)");ctx.fillStyle=gg;ctx.fillRect(cx+x2-gRR,cy+y2-gRR,gRR*2,gRR*2)}
        ctx.fillStyle=`rgba(${170+d*85},${15+d*65},${5+d*15},${a})`;ctx.beginPath();ctx.arc(cx+x2,cy+y2,sz,0,Math.PI*2);ctx.fill()}}
      for(let i=0;i<25*intensity;i++){const a2=(i/25)*Math.PI*2+t*.3,dd=bR*1.2+Math.sin(t+i*.7)*bR*.5;ctx.fillStyle=`rgba(200,170,160,${(.3+Math.sin(t*3+i*1.3)*.3)*s.glow*.5})`;ctx.fillRect(cx+Math.cos(a2)*dd,cy+Math.sin(a2)*dd*.7,1,1)}
      anim.current = requestAnimationFrame(draw);
    };
    anim.current = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(anim.current);
  }, [size, mode, intensity]);
  return <canvas ref={ref} style={{ width: size, height: size, ...style }} />;
};

// ─── DESIGN TOKENS (platform-matched + M3 additions) ─────
const S = {
  bg: "#050508",
  surface: "rgba(255,255,255,0.03)",
  surfaceHover: "rgba(255,255,255,0.06)",
  surfaceContainer: "rgba(255,255,255,0.05)",
  surfaceContainerHigh: "rgba(255,255,255,0.08)",
  border: "rgba(255,60,20,0.1)",
  borderHover: "rgba(255,60,20,0.25)",
  accent: "#ff3c14",
  accentDim: "rgba(255,60,20,0.6)",
  accentGlow: "rgba(255,60,20,0.15)",
  text: "rgba(255,245,240,0.9)",
  textDim: "rgba(255,245,240,0.45)",
  textMuted: "rgba(255,245,240,0.25)",
  green: "#22c55e",
  yellow: "#eab308",
  red: "#ef4444",
  blue: "#3b82f6",
  font: "'Rajdhani', 'Roboto', sans-serif",
  mono: "'Orbitron', 'Roboto Mono', monospace",
  // M3 specific
  navSurface: "rgba(12,12,18,0.95)",
  navIndicator: "rgba(255,60,20,0.16)",
  elevation1: "0 1px 3px rgba(0,0,0,0.4)",
  elevation2: "0 2px 8px rgba(0,0,0,0.5)",
  elevation3: "0 4px 16px rgba(0,0,0,0.6)",
  radius: { sm: 8, md: 12, lg: 16, xl: 28, full: 9999 },
};

// ─── ANDROID SYSTEM CHROME ───────────────────────────────
const AndroidStatusBar = () => (
  <div style={{ height: 28, display: "flex", alignItems: "center", justifyContent: "space-between", padding: "0 16px", flexShrink: 0 }}>
    <span style={{ fontSize: 11, fontWeight: 600, color: S.text, fontFamily: S.font }}>12:34</span>
    <div style={{ display: "flex", alignItems: "center", gap: 5 }}>
      {/* Signal */}
      <svg width="12" height="12" viewBox="0 0 24 24"><path d="M1 21h4V9H1v12zm6 0h4V3H7v18zm6 0h4v-8h-4v8zm6 0h4v-4h-4v4z" fill={S.text} fillOpacity="0.7" /></svg>
      {/* WiFi */}
      <svg width="13" height="13" viewBox="0 0 24 24"><path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3c-1.65-1.66-4.34-1.66-6 0zm-4-4l2 2c2.76-2.76 7.24-2.76 10 0l2-2C15.14 9.14 8.87 9.14 5 13z" fill={S.text} fillOpacity="0.7" /></svg>
      {/* Battery */}
      <svg width="16" height="12" viewBox="0 0 24 14"><rect x="0" y="1" width="20" height="12" rx="2" fill="none" stroke={S.text} strokeOpacity="0.5" strokeWidth="1.5" /><rect x="2" y="3" width="14" height="8" rx="1" fill={S.green} /><rect x="21" y="4" width="2.5" height="6" rx="1" fill={S.text} fillOpacity="0.4" /></svg>
    </div>
  </div>
);

const AndroidGestureBar = () => (
  <div style={{ height: 20, display: "flex", alignItems: "center", justifyContent: "center", flexShrink: 0 }}>
    <div style={{ width: 134, height: 4, borderRadius: 2, background: "rgba(255,255,255,0.2)" }} />
  </div>
);

// ─── M3 BOTTOM NAVIGATION ────────────────────────────────
const BottomNav = ({ active, onChange }) => {
  const items = [
    { id: "home", label: "Home", icon: "M3 9l9-7 9 7v11a2 2 0 01-2 2H5a2 2 0 01-2-2z" },
    { id: "chat", label: "Chat", icon: "M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2z" },
    { id: "tools", label: "Tools", icon: "M14.7 6.3a1 1 0 000 1.4l1.6 1.6a1 1 0 001.4 0l3.77-3.77a6 6 0 01-7.94 7.94l-6.91 6.91a2.12 2.12 0 01-3-3l6.91-6.91a6 6 0 017.94-7.94l-3.76 3.76z" },
    { id: "agents", label: "Agents", icon: "M17 21v-2a4 4 0 00-4-4H5a4 4 0 00-4-4v2M9 7a4 4 0 100-8 4 4 0 000 8zM23 21v-2a4 4 0 00-3-3.87M16 3.13a4 4 0 010 7.75" },
    { id: "wallet", label: "Wallet", icon: "M19 7h-1V6a3 3 0 00-3-3H5a3 3 0 00-3 3v12a3 3 0 003 3h14a2 2 0 002-2V9a2 2 0 00-2-2zM16 14a1 1 0 110-2 1 1 0 010 2z" },
    { id: "settings", label: "Settings", icon: "M12.22 2h-.44a2 2 0 00-2 2v.18a2 2 0 01-1 1.73l-.43.25a2 2 0 01-2 0l-.15-.08a2 2 0 00-2.73.73l-.22.38a2 2 0 00.73 2.73l.15.1a2 2 0 011 1.72v.51a2 2 0 01-1 1.74l-.15.09a2 2 0 00-.73 2.73l.22.38a2 2 0 002.73.73l.15-.08a2 2 0 012 0l.43.25a2 2 0 011 1.73V20a2 2 0 002 2h.44a2 2 0 002-2v-.18a2 2 0 011-1.73l.43-.25a2 2 0 012 0l.15.08a2 2 0 002.73-.73l.22-.39a2 2 0 00-.73-2.73l-.15-.08a2 2 0 01-1-1.74v-.5a2 2 0 011-1.74l.15-.09a2 2 0 00.73-2.73l-.22-.38a2 2 0 00-2.73-.73l-.15.08a2 2 0 01-2 0l-.43-.25a2 2 0 01-1-1.73V4a2 2 0 00-2-2z" },
  ];
  return (
    <div style={{
      height: 64, display: "flex", alignItems: "center", justifyContent: "space-around",
      background: S.navSurface, backdropFilter: "blur(20px)",
      borderTop: `1px solid ${S.border}`, flexShrink: 0,
    }}>
      {items.map(it => {
        const sel = active === it.id;
        return (
          <div key={it.id} onClick={() => onChange(it.id)} style={{
            display: "flex", flexDirection: "column", alignItems: "center", gap: 3,
            cursor: "pointer", padding: "4px 0", minWidth: 48,
          }}>
            {/* M3 pill indicator */}
            <div style={{
              width: sel ? 56 : 24, height: 28, borderRadius: 14,
              background: sel ? S.navIndicator : "transparent",
              display: "flex", alignItems: "center", justifyContent: "center",
              transition: "all 0.35s cubic-bezier(0.2, 0, 0, 1)",
            }}>
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke={sel ? S.accent : S.textMuted} strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d={it.icon} /></svg>
            </div>
            <span style={{
              fontSize: 10, fontWeight: sel ? 700 : 500,
              color: sel ? S.text : S.textMuted, letterSpacing: 0.2,
              transition: "all 0.2s",
            }}>{it.label}</span>
          </div>
        );
      })}
    </div>
  );
};

// ─── M3 COMPONENTS ───────────────────────────────────────
const TopAppBar = ({ title, subtitle, large, actions }) => (
  <div style={{
    padding: large ? "16px 16px 20px" : "10px 16px",
    flexShrink: 0,
  }}>
    {subtitle && <div style={{ fontFamily: S.mono, fontSize: 9, letterSpacing: 3, color: S.accentDim, marginBottom: 4, fontWeight: 700 }}>{subtitle}</div>}
    <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
      <span style={{ fontSize: large ? 26 : 20, fontWeight: 700, color: S.text, letterSpacing: -0.3 }}>{title}</span>
      {actions && <div style={{ display: "flex", gap: 8 }}>{actions}</div>}
    </div>
  </div>
);

const M3Card = ({ children, style: st = {}, glow, onClick }) => (
  <div onClick={onClick} style={{
    background: S.surfaceContainer, border: `1px solid ${glow ? S.borderHover : S.border}`,
    borderRadius: S.radius.lg, padding: 16, transition: "all 0.3s",
    boxShadow: glow ? `0 0 30px ${S.accentGlow}` : S.elevation1,
    cursor: onClick ? "pointer" : "default", ...st,
  }}>{children}</div>
);

const M3Chip = ({ children, color = S.accentDim, selected }) => (
  <span style={{
    fontFamily: S.mono, fontSize: 9, fontWeight: 600, letterSpacing: 0.8,
    color: selected ? S.text : color,
    background: selected ? S.navIndicator : `${color}15`,
    padding: "5px 12px", borderRadius: S.radius.sm,
    border: `1px solid ${selected ? S.borderHover : "transparent"}`,
    transition: "all 0.2s",
  }}>{children}</span>
);

const M3Dot = ({ color = S.green, size = 8 }) => (
  <div style={{ width: size, height: size, borderRadius: size, background: color, boxShadow: `0 0 8px ${color}`, flexShrink: 0 }} />
);

const M3IconBtn = ({ d, size = 18, onClick, badge }) => (
  <div onClick={onClick} style={{ width: 40, height: 40, borderRadius: 20, display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer", background: S.surface, position: "relative" }}>
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={S.textDim} strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"><path d={d} /></svg>
    {badge && <div style={{ position: "absolute", top: 2, right: 2, width: 14, height: 14, borderRadius: 7, background: S.accent, fontSize: 8, fontWeight: 700, color: "#fff", display: "flex", alignItems: "center", justifyContent: "center" }}>{badge}</div>}
  </div>
);

const M3SearchBar = ({ placeholder = "Search..." }) => (
  <div style={{
    margin: "0 16px 12px", padding: "12px 16px", borderRadius: S.radius.xl,
    background: S.surfaceContainerHigh, display: "flex", alignItems: "center", gap: 12,
  }}>
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke={S.textMuted} strokeWidth="2"><circle cx="11" cy="11" r="8" /><line x1="21" y1="21" x2="16.65" y2="16.65" /></svg>
    <span style={{ fontSize: 14, color: S.textMuted }}>{placeholder}</span>
  </div>
);

const M3ListItem = ({ title, subtitle, leading, trailing, onClick }) => (
  <div onClick={onClick} style={{
    display: "flex", alignItems: "center", gap: 14, padding: "14px 16px",
    cursor: onClick ? "pointer" : "default", transition: "background 0.15s",
  }}>
    {leading}
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{ fontSize: 15, fontWeight: 600, color: S.text }}>{title}</div>
      {subtitle && <div style={{ fontSize: 12, color: S.textDim, marginTop: 1 }}>{subtitle}</div>}
    </div>
    {trailing}
  </div>
);

const M3Toggle = ({ on, onChange }) => (
  <div onClick={() => onChange?.(!on)} style={{
    width: 48, height: 28, borderRadius: 14, padding: 2, cursor: "pointer", flexShrink: 0,
    background: on ? "rgba(255,60,20,0.4)" : "rgba(255,255,255,0.1)",
    border: `2px solid ${on ? S.accent : S.textMuted}`,
    transition: "all 0.3s",
  }}>
    <div style={{
      width: 20, height: 20, borderRadius: 10,
      background: on ? "#fff" : S.textMuted,
      transition: "all 0.3s cubic-bezier(0.2, 0, 0, 1)",
      transform: on ? "translateX(20px)" : "translateX(0)",
    }} />
  </div>
);

const FAB = ({ icon, onClick, extended, label }) => (
  <div onClick={onClick} style={{
    position: "absolute", right: 16, bottom: 16,
    height: 56, borderRadius: S.radius.lg,
    padding: extended ? "0 20px" : "0 16px",
    background: "rgba(255,60,20,0.18)", border: `1px solid ${S.borderHover}`,
    boxShadow: `${S.elevation3}, 0 0 40px ${S.accentGlow}`,
    display: "flex", alignItems: "center", justifyContent: "center", gap: 10,
    cursor: "pointer", transition: "all 0.3s",
    minWidth: extended ? "auto" : 56,
  }}>
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke={S.accent} strokeWidth="2" strokeLinecap="round"><path d={icon} /></svg>
    {extended && <span style={{ fontFamily: S.font, fontSize: 14, fontWeight: 700, color: S.accent, letterSpacing: 0.5 }}>{label}</span>}
  </div>
);

// ─── HOME TAB ────────────────────────────────────────────
const HomeTab = () => (
  <div style={{ flex: 1, overflowY: "auto", paddingBottom: 80 }}>
    <TopAppBar title="Zeus" subtitle="COGNITIVE ENGINE" large actions={
      <>
        <M3IconBtn d="M18 8A6 6 0 006 8c0 7-3 9-3 9h18s-3-2-3-9M13.73 21a2 2 0 01-3.46 0" badge="2" />
        <M3IconBtn d="M20 21v-2a4 4 0 00-4-4H8a4 4 0 00-4-4v2M12 7a4 4 0 100-8 4 4 0 000 8z" />
      </>
    } />

    {/* Hero Card */}
    <div style={{ padding: "0 16px 16px" }}>
      <M3Card glow style={{ display: "flex", alignItems: "center", gap: 16 }}>
        <Orb size={72} mode="active" intensity={0.85} />
        <div style={{ flex: 1 }}>
          <div style={{ fontFamily: S.mono, fontSize: 8, letterSpacing: 3, color: S.accentDim, fontWeight: 700 }}>PRIMARY AGENT</div>
          <div style={{ fontSize: 20, fontWeight: 700, color: S.text, marginTop: 2 }}>Zeus Prime</div>
          <div style={{ fontSize: 12, color: S.textDim, marginTop: 2 }}>claude-sonnet-4 • Online</div>
        </div>
        <M3Dot color={S.green} />
      </M3Card>
    </div>

    {/* Metrics Row */}
    <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 8, padding: "0 16px 16px" }}>
      {[{ l: "Tools", v: "212", c: S.accent }, { l: "Channels", v: "8", c: S.green }, { l: "Memory", v: "2.8K", c: S.yellow }, { l: "Sessions", v: "147", c: S.blue }].map(m => (
        <M3Card key={m.l} style={{ padding: 12, textAlign: "center" }}>
          <div style={{ fontFamily: S.mono, fontSize: 18, fontWeight: 700, color: m.c }}>{m.v}</div>
          <div style={{ fontSize: 10, color: S.textMuted, marginTop: 2 }}>{m.l}</div>
        </M3Card>
      ))}
    </div>

    {/* Quick Actions */}
    <div style={{ padding: "0 16px 16px" }}>
      <div style={{ fontFamily: S.mono, fontSize: 10, letterSpacing: 3, color: S.textMuted, marginBottom: 10, fontWeight: 700 }}>QUICK ACTIONS</div>
      <div style={{ display: "flex", gap: 8 }}>
        {[{ l: "New Chat", icon: "M12 5v14M5 12h14" }, { l: "Run Tool", icon: "M13 2L3 14h9l-1 8 10-12h-9l1-8z" }, { l: "Schedule", icon: "M12 2a10 10 0 0110 10 10 10 0 01-10 10A10 10 0 012 12 10 10 0 0112 2zM12 6v6l4 2" }].map(a => (
          <M3Card key={a.l} style={{ flex: 1, padding: 14, textAlign: "center", cursor: "pointer" }}>
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke={S.accentDim} strokeWidth="1.5" strokeLinecap="round" style={{ margin: "0 auto 6px" }}><path d={a.icon} /></svg>
            <div style={{ fontSize: 11, fontWeight: 600, color: S.textDim }}>{a.l}</div>
          </M3Card>
        ))}
      </div>
    </div>

    {/* Active Agents */}
    <div style={{ padding: "0 16px 16px" }}>
      <div style={{ fontFamily: S.mono, fontSize: 10, letterSpacing: 3, color: S.textMuted, marginBottom: 10, fontWeight: 700 }}>ACTIVE AGENTS</div>
      <M3Card style={{ padding: 0, overflow: "hidden" }}>
        {[{ n: "Zeus Prime", r: "Primary", m: "sonnet-4", s: "active" }, { n: "Hermes", r: "Communications", m: "gpt-4o", s: "active" }, { n: "Athena", r: "Documentation", m: "llama-3.3", s: "idle" }, { n: "Prometheus", r: "Orchestrator", m: "sonnet-4", s: "active" }].map((a, i, arr) => (
          <div key={a.n} style={{ display: "flex", alignItems: "center", gap: 12, padding: "12px 16px", borderBottom: i < arr.length - 1 ? `1px solid ${S.border}` : "none" }}>
            <Orb size={36} mode={a.s === "active" ? "active" : "dormant"} intensity={a.s === "active" ? 0.7 : 0.3} />
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 14, fontWeight: 600, color: S.text }}>{a.n}</div>
              <div style={{ fontSize: 11, color: S.textDim }}>{a.r}</div>
            </div>
            <M3Chip>{a.m}</M3Chip>
            <M3Dot color={a.s === "active" ? S.green : S.yellow} />
          </div>
        ))}
      </M3Card>
    </div>

    {/* Recent Sessions */}
    <div style={{ padding: "0 16px 16px" }}>
      <div style={{ fontFamily: S.mono, fontSize: 10, letterSpacing: 3, color: S.textMuted, marginBottom: 10, fontWeight: 700 }}>RECENT SESSIONS</div>
      <M3Card style={{ padding: 0, overflow: "hidden" }}>
        {[{ t: "Staging Deployment", msgs: 24, cost: "$0.47", time: "2m ago" }, { t: "Investor Meeting Prep", msgs: 18, cost: "$0.32", time: "18m ago" }, { t: "Zeus Crate Refactor", msgs: 67, cost: "$1.23", time: "2h ago" }, { t: "NeuroDrums Pattern", msgs: 34, cost: "$0.56", time: "5h ago" }].map((s, i, arr) => (
          <div key={i} style={{ display: "flex", alignItems: "center", gap: 12, padding: "12px 16px", borderBottom: i < arr.length - 1 ? `1px solid ${S.border}` : "none" }}>
            <div style={{ width: 36, height: 36, borderRadius: S.radius.md, background: S.accentGlow, display: "flex", alignItems: "center", justifyContent: "center" }}>💬</div>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 14, fontWeight: 500, color: S.text }}>{s.t}</div>
              <div style={{ fontSize: 11, color: S.textMuted }}>{s.msgs} msgs • {s.cost}</div>
            </div>
            <span style={{ fontSize: 11, color: S.textMuted }}>{s.time}</span>
          </div>
        ))}
      </M3Card>
    </div>

    {/* Channels */}
    <div style={{ padding: "0 16px 16px" }}>
      <div style={{ fontFamily: S.mono, fontSize: 10, letterSpacing: 3, color: S.textMuted, marginBottom: 10, fontWeight: 700 }}>CHANNELS</div>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
        {[{ n: "Telegram", s: "connected", c: 2341 }, { n: "Discord", s: "connected", c: 891 }, { n: "Slack", s: "connected", c: 567 }, { n: "Email", s: "connected", c: 234 }, { n: "iMessage", s: "connected", c: 189 }, { n: "WhatsApp", s: "idle", c: 0 }, { n: "Signal", s: "idle", c: 0 }, { n: "Matrix", s: "off", c: 0 }].map(ch => (
          <M3Card key={ch.n} style={{ padding: 12, display: "flex", alignItems: "center", gap: 10 }}>
            <M3Dot color={ch.s === "connected" ? S.green : ch.s === "idle" ? S.yellow : S.textMuted} size={7} />
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 13, fontWeight: 600, color: S.text }}>{ch.n}</div>
              <div style={{ fontSize: 10, color: S.textMuted }}>{ch.c > 0 ? `${ch.c.toLocaleString()} msgs` : ch.s}</div>
            </div>
          </M3Card>
        ))}
      </div>
    </div>
  </div>
);

// ─── CHAT TAB ────────────────────────────────────────────
const ChatTab = () => {
  const [input, setInput] = useState("");
  const [streaming, setStreaming] = useState(false);
  const messages = [
    { role: "user", text: "Deploy the latest build to staging and run integration tests." },
    { role: "assistant", text: "Starting deployment pipeline for Zeus v1.0.0. I'll build the release binary, push to staging, trigger CI, and report results.", tools: ["shell", "git_push", "web_fetch"] },
    { role: "user", text: "Check my calendar for tomorrow and prep me for the investor meeting" },
    { role: "assistant", text: "You have the NovaXAI Series A pitch tomorrow at 2:00 PM with Mubadala Capital. I've drafted prep notes with your key talking points and updated deck link.", tools: ["calendar_list", "notes_read", "mail_send"] },
  ];

  const handleSend = () => {
    if (!input.trim()) return;
    setInput(""); setStreaming(true);
    setTimeout(() => setStreaming(false), 3000);
  };

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column" }}>
      {/* Top bar */}
      <div style={{ display: "flex", alignItems: "center", gap: 12, padding: "10px 16px", borderBottom: `1px solid ${S.border}`, flexShrink: 0 }}>
        <Orb size={32} mode={streaming ? "speaking" : "active"} intensity={0.8} />
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 16, fontWeight: 700, color: S.text }}>Staging Deployment</div>
          <div style={{ fontSize: 11, color: S.textDim }}>Zeus Prime • 24 messages</div>
        </div>
        <M3Chip color={streaming ? S.yellow : S.green}>{streaming ? "Thinking..." : "Ready"}</M3Chip>
      </div>

      {/* Messages */}
      <div style={{ flex: 1, overflowY: "auto", padding: "16px 12px" }}>
        {messages.map((m, i) => (
          <div key={i} style={{ display: "flex", gap: 10, marginBottom: 16, flexDirection: m.role === "user" ? "row-reverse" : "row" }}>
            {m.role === "assistant" && <Orb size={28} mode="dormant" intensity={0.5} style={{ flexShrink: 0, marginTop: 2 }} />}
            <div style={{ maxWidth: "78%" }}>
              <div style={{
                padding: "12px 16px", borderRadius: 20,
                background: m.role === "user" ? "rgba(255,60,20,0.14)" : S.surfaceContainer,
                border: `1px solid ${m.role === "user" ? S.borderHover : S.border}`,
                borderBottomRightRadius: m.role === "user" ? 6 : 20,
                borderBottomLeftRadius: m.role === "assistant" ? 6 : 20,
              }}>
                <div style={{ fontSize: 14, color: S.text, lineHeight: 1.6 }}>{m.text}</div>
              </div>
              {m.tools && (
                <div style={{ display: "flex", gap: 4, marginTop: 6, flexWrap: "wrap", justifyContent: m.role === "user" ? "flex-end" : "flex-start" }}>
                  {m.tools.map(t => <M3Chip key={t} color="rgba(255,140,80,0.6)">{t}</M3Chip>)}
                </div>
              )}
            </div>
          </div>
        ))}
        {streaming && (
          <div style={{ display: "flex", gap: 10, marginBottom: 16 }}>
            <Orb size={28} mode="speaking" intensity={0.85} style={{ flexShrink: 0, marginTop: 2 }} />
            <div style={{ padding: "14px 18px", borderRadius: 20, borderBottomLeftRadius: 6, background: S.surfaceContainer, border: `1px solid ${S.border}`, display: "flex", gap: 6 }}>
              {[0, 1, 2].map(i => <div key={i} style={{ width: 7, height: 7, borderRadius: 4, background: S.accentDim, animation: `androidPulse 1.4s ease ${i * 0.2}s infinite` }} />)}
            </div>
          </div>
        )}
      </div>

      {/* Input bar */}
      <div style={{ padding: "8px 12px 12px", flexShrink: 0 }}>
        <div style={{
          display: "flex", alignItems: "flex-end", gap: 8,
          background: S.surfaceContainerHigh, borderRadius: S.radius.xl,
          padding: "4px 6px 4px 18px", border: `1px solid ${S.border}`,
        }}>
          <textarea value={input} onChange={e => setInput(e.target.value)}
            onKeyDown={e => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); handleSend(); } }}
            placeholder="Message Zeus..."
            rows={1} style={{
              flex: 1, background: "transparent", border: "none", color: S.text,
              fontSize: 15, fontFamily: S.font, outline: "none", resize: "none",
              lineHeight: 1.4, padding: "10px 0",
            }} />
          <div onClick={handleSend} style={{
            width: 40, height: 40, borderRadius: 20,
            background: input.trim() ? "rgba(255,60,20,0.2)" : "transparent",
            display: "flex", alignItems: "center", justifyContent: "center",
            cursor: "pointer", transition: "all 0.2s", flexShrink: 0,
          }}>
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke={input.trim() ? S.accent : S.textMuted} strokeWidth="2" strokeLinecap="round"><line x1="22" y1="2" x2="11" y2="13" /><polygon points="22 2 15 22 11 13 2 9 22 2" /></svg>
          </div>
        </div>
      </div>
    </div>
  );
};

// ─── TOOLS TAB ───────────────────────────────────────────
const ToolsTab = () => {
  const [cat, setCat] = useState("all");
  const tools = [
    { name: "shell", cat: "Core", desc: "Execute shell commands", calls: 1847 },
    { name: "read_file", cat: "Core", desc: "Read file contents", calls: 923 },
    { name: "write_file", cat: "Core", desc: "Create or overwrite files", calls: 612 },
    { name: "web_fetch", cat: "Core", desc: "Fetch URL content", calls: 445 },
    { name: "web_search", cat: "Core", desc: "Search via DuckDuckGo", calls: 334 },
    { name: "calendar_list", cat: "Talos", desc: "List calendar events", calls: 234 },
    { name: "mail_send", cat: "Talos", desc: "Send email via Mail.app", calls: 189 },
    { name: "git_commit", cat: "Talos", desc: "Create a git commit", calls: 167 },
    { name: "screenshot", cat: "Talos", desc: "Capture screen", calls: 134 },
    { name: "navigate", cat: "Browser", desc: "Navigate Chrome via CDP", calls: 98 },
    { name: "telegram_send", cat: "Channel", desc: "Send Telegram message", calls: 312 },
  ];
  const cats = ["all", "Core", "Talos", "Browser", "Channel"];
  const filtered = cat === "all" ? tools : tools.filter(t => t.cat === cat);

  return (
    <div style={{ flex: 1, overflowY: "auto" }}>
      <TopAppBar title="Tools" subtitle="212 REGISTERED" large />
      <M3SearchBar placeholder="Search 212 tools..." />

      {/* Filter chips */}
      <div style={{ display: "flex", gap: 6, padding: "0 16px 14px", overflowX: "auto" }}>
        {cats.map(c => (
          <div key={c} onClick={() => setCat(c)} style={{ cursor: "pointer" }}>
            <M3Chip selected={cat === c}>{c === "all" ? "All" : c}</M3Chip>
          </div>
        ))}
      </div>

      {/* Tool list */}
      <div style={{ padding: "0 16px 16px" }}>
        <M3Card style={{ padding: 0, overflow: "hidden" }}>
          {filtered.map((t, i, arr) => (
            <div key={t.name} style={{ display: "flex", alignItems: "center", gap: 12, padding: "12px 16px", borderBottom: i < arr.length - 1 ? `1px solid ${S.border}` : "none" }}>
              <div style={{ width: 36, height: 36, borderRadius: S.radius.md, background: S.accentGlow, border: `1px solid ${S.border}`, display: "flex", alignItems: "center", justifyContent: "center" }}>
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke={S.accentDim} strokeWidth="1.5"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" /></svg>
              </div>
              <div style={{ flex: 1 }}>
                <div style={{ fontFamily: S.mono, fontSize: 12, fontWeight: 600, color: S.text, letterSpacing: 0.5 }}>{t.name}</div>
                <div style={{ fontSize: 11, color: S.textDim, marginTop: 1 }}>{t.desc}</div>
              </div>
              <div style={{ textAlign: "right" }}>
                <div style={{ fontFamily: S.mono, fontSize: 11, color: S.textDim }}>{t.calls.toLocaleString()}</div>
                <M3Chip>{t.cat}</M3Chip>
              </div>
            </div>
          ))}
        </M3Card>
      </div>
    </div>
  );
};

// ─── AGENTS TAB ──────────────────────────────────────────
const AgentsTab = () => {
  const agents = [
    { name: "Zeus Prime", role: "Primary Cognitive Engine", model: "claude-sonnet-4", status: "active", tasks: 147, desc: "Main reasoning and task execution agent" },
    { name: "Hermes", role: "Communications Director", model: "gpt-4o", status: "active", tasks: 89, desc: "Cross-platform messaging and relay" },
    { name: "Athena", role: "Knowledge & Documentation", model: "llama-3.3-70b", status: "idle", tasks: 45, desc: "Research, docs generation, knowledge base" },
    { name: "Prometheus", role: "Task Orchestrator", model: "claude-sonnet-4", status: "active", tasks: 234, desc: "Scheduling, delegation, and monitoring" },
  ];

  return (
    <div style={{ flex: 1, overflowY: "auto", paddingBottom: 80, position: "relative" }}>
      <TopAppBar title="Agents" subtitle="4 CONFIGURED" large />

      <div style={{ padding: "0 16px 16px", display: "flex", flexDirection: "column", gap: 12 }}>
        {agents.map(a => (
          <M3Card key={a.name} glow={a.status === "active"} style={{ padding: 20 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 14, marginBottom: 14 }}>
              <Orb size={52} mode={a.status === "active" ? "active" : "dormant"} intensity={a.status === "active" ? 0.8 : 0.3} />
              <div style={{ flex: 1 }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  <span style={{ fontSize: 18, fontWeight: 700, color: S.text }}>{a.name}</span>
                  <M3Dot color={a.status === "active" ? S.green : S.yellow} />
                </div>
                <div style={{ fontSize: 12, color: S.textDim, marginTop: 2 }}>{a.role}</div>
              </div>
            </div>
            <div style={{ fontSize: 13, color: S.textDim, lineHeight: 1.5, marginBottom: 14 }}>{a.desc}</div>
            <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
              <M3Chip>{a.model}</M3Chip>
              <M3Chip color={S.textDim}>{a.tasks} tasks</M3Chip>
              <div style={{ flex: 1 }} />
              <div style={{
                padding: "8px 18px", borderRadius: S.radius.xl, cursor: "pointer",
                background: "rgba(255,60,20,0.1)", border: `1px solid ${S.borderHover}`,
                fontFamily: S.mono, fontSize: 10, letterSpacing: 2, color: S.accent, fontWeight: 700,
              }}>INTERACT</div>
            </div>
          </M3Card>
        ))}
      </div>

      <FAB icon="M12 5v14M5 12h14" extended label="New Agent" />
    </div>
  );
};

// ─── SETTINGS TAB ────────────────────────────────────────
const SettingsTab = () => {
  const [secLevel, setSecLevel] = useState("standard");
  return (
    <div style={{ flex: 1, overflowY: "auto" }}>
      <TopAppBar title="Settings" large />

      {/* Profile */}
      <div style={{ padding: "0 16px 16px" }}>
        <M3Card style={{ display: "flex", alignItems: "center", gap: 14 }}>
          <Orb size={48} mode="active" intensity={0.7} />
          <div>
            <div style={{ fontSize: 18, fontWeight: 700, color: S.text }}>Miguel</div>
            <div style={{ fontSize: 13, color: S.textDim }}>COO • NovaXAI</div>
          </div>
        </M3Card>
      </div>

      {/* Connection */}
      <div style={{ padding: "0 16px 4px" }}>
        <div style={{ fontFamily: S.mono, fontSize: 10, letterSpacing: 3, color: S.textMuted, marginBottom: 8, fontWeight: 700, paddingLeft: 4 }}>CONNECTION</div>
      </div>
      <div style={{ padding: "0 16px 16px" }}>
        <M3Card style={{ padding: 0, overflow: "hidden" }}>
          {[{ l: "Gateway URL", v: "127.0.0.1:8080" }, { l: "MCP Server", v: "Port 3002" }, { l: "Status", v: null }].map((r, i) => (
            <div key={r.l} style={{ display: "flex", alignItems: "center", padding: "14px 16px", borderBottom: i < 2 ? `1px solid ${S.border}` : "none" }}>
              <span style={{ flex: 1, fontSize: 14, color: S.text }}>{r.l}</span>
              {r.v ? <span style={{ fontFamily: S.mono, fontSize: 12, color: S.textDim }}>{r.v}</span> : <M3Chip color={S.green}>Connected</M3Chip>}
            </div>
          ))}
        </M3Card>
      </div>

      {/* Model */}
      <div style={{ padding: "0 16px 4px" }}>
        <div style={{ fontFamily: S.mono, fontSize: 10, letterSpacing: 3, color: S.textMuted, marginBottom: 8, fontWeight: 700, paddingLeft: 4 }}>MODEL</div>
      </div>
      <div style={{ padding: "0 16px 16px" }}>
        <M3Card style={{ padding: 0, overflow: "hidden" }}>
          {[{ l: "Default Model", v: "claude-sonnet-4" }, { l: "Max Iterations", v: "20" }, { l: "Providers", v: "11 configured" }].map((r, i) => (
            <div key={r.l} style={{ display: "flex", alignItems: "center", padding: "14px 16px", borderBottom: i < 2 ? `1px solid ${S.border}` : "none" }}>
              <span style={{ flex: 1, fontSize: 14, color: S.text }}>{r.l}</span>
              <span style={{ fontFamily: S.mono, fontSize: 12, color: S.textDim }}>{r.v}</span>
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke={S.textMuted} strokeWidth="2" strokeLinecap="round" style={{ marginLeft: 8 }}><polyline points="9 18 15 12 9 6" /></svg>
            </div>
          ))}
        </M3Card>
      </div>

      {/* Security */}
      <div style={{ padding: "0 16px 4px" }}>
        <div style={{ fontFamily: S.mono, fontSize: 10, letterSpacing: 3, color: S.textMuted, marginBottom: 8, fontWeight: 700, paddingLeft: 4 }}>SECURITY LEVEL</div>
      </div>
      <div style={{ display: "flex", gap: 8, padding: "0 16px 16px" }}>
        {[{ id: "minimal", c: S.yellow }, { id: "standard", c: S.green }, { id: "strict", c: S.accent }].map(l => (
          <M3Card key={l.id} glow={secLevel === l.id} onClick={() => setSecLevel(l.id)} style={{
            flex: 1, padding: 14, textAlign: "center",
          }}>
            <div style={{ fontFamily: S.mono, fontSize: 10, fontWeight: 700, letterSpacing: 2, color: secLevel === l.id ? S.text : S.textDim, textTransform: "uppercase" }}>{l.id}</div>
            <div style={{ width: 8, height: 8, borderRadius: 4, background: l.c, margin: "8px auto 0" }} />
          </M3Card>
        ))}
      </div>

      {/* Features */}
      <div style={{ padding: "0 16px 4px" }}>
        <div style={{ fontFamily: S.mono, fontSize: 10, letterSpacing: 3, color: S.textMuted, marginBottom: 8, fontWeight: 700, paddingLeft: 4 }}>FEATURES</div>
      </div>
      <div style={{ padding: "0 16px 16px" }}>
        <M3Card style={{ padding: 0, overflow: "hidden" }}>
          {[{ n: "Nous Cognitive", on: true }, { n: "Mnemosyne Memory", on: true }, { n: "Prometheus Tasks", on: true }, { n: "Browser Automation", on: false }, { n: "Voice Pipeline", on: false }, { n: "Talos macOS", on: true }, { n: "Aegis Security", on: true }, { n: "MCP Server", on: true }].map((f, i, arr) => (
            <div key={f.n} style={{ display: "flex", alignItems: "center", padding: "13px 16px", borderBottom: i < arr.length - 1 ? `1px solid ${S.border}` : "none" }}>
              <span style={{ flex: 1, fontSize: 14, color: S.text }}>{f.n}</span>
              <M3Toggle on={f.on} />
            </div>
          ))}
        </M3Card>
      </div>

      {/* Channels */}
      <div style={{ padding: "0 16px 4px" }}>
        <div style={{ fontFamily: S.mono, fontSize: 10, letterSpacing: 3, color: S.textMuted, marginBottom: 8, fontWeight: 700, paddingLeft: 4 }}>CHANNELS</div>
      </div>
      <div style={{ padding: "0 16px 16px" }}>
        <M3Card style={{ padding: 0, overflow: "hidden" }}>
          {["Telegram", "Discord", "Slack", "Email", "iMessage", "WhatsApp", "Signal", "Matrix"].map((ch, i, arr) => (
            <div key={ch} style={{ display: "flex", alignItems: "center", padding: "13px 16px", borderBottom: i < arr.length - 1 ? `1px solid ${S.border}` : "none" }}>
              <span style={{ flex: 1, fontSize: 14, color: S.text }}>{ch}</span>
              <M3Chip color={i < 5 ? S.green : S.textMuted}>{i < 5 ? "ON" : "OFF"}</M3Chip>
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke={S.textMuted} strokeWidth="2" strokeLinecap="round" style={{ marginLeft: 8 }}><polyline points="9 18 15 12 9 6" /></svg>
            </div>
          ))}
        </M3Card>
      </div>

      {/* About */}
      <div style={{ padding: "0 16px 24px" }}>
        <M3Card style={{ textAlign: "center", padding: 20 }}>
          <div style={{ fontFamily: S.mono, fontSize: 10, letterSpacing: 4, color: S.textMuted, fontWeight: 700 }}>ZEUS v1.0.0</div>
          <div style={{ fontSize: 12, color: S.textMuted, marginTop: 4 }}>21 crates • 59,400 LoC • 212 tools</div>
          <div style={{ fontSize: 11, color: S.textMuted, marginTop: 2 }}>Built with Rust 🦀</div>
        </M3Card>
      </div>
    </div>
  );
};

// ─── WALLET TAB (Material 3) ─────────────────────────────
const WA_TITANS = [
  { name: "Hermes", role: "Coordinator", token: 48210, credit: 1250, mode: "active", color: S.accent, st: "active" },
  { name: "Hephaestus", role: "Backend / Forge", token: 31980, credit: 840, mode: "speaking", color: S.accent, st: "active" },
  { name: "Atlas", role: "Backend (dual)", token: 27340, credit: 610, mode: "active", color: S.accent, st: "active" },
  { name: "Aegis", role: "Security & CI", token: 19750, credit: 1100, mode: "thinking", color: S.green, st: "active" },
  { name: "Calliope", role: "Marketing", token: 22410, credit: 430, mode: "active", color: S.yellow, st: "active" },
  { name: "Prometheus", role: "Experimental", token: 8120, credit: 290, mode: "dormant", color: S.blue, st: "idle" },
];
const WA_ACT = [
  { k: "received", who: "Agora → Calliope", amt: 2400, u: "ZEUS", st: "confirmed", t: "2m", note: "x402 content sale" },
  { k: "sent", who: "You → Hephaestus", amt: 5000, u: "ZEUS", st: "confirmed", t: "14m", note: "compute top-up" },
  { k: "multi", who: "Hermes → 3 titans", amt: 1800, u: "ZEUS", st: "confirmed", t: "31m", note: "mission payout split" },
  { k: "spend", who: "Hephaestus → Agora", amt: 499, u: "CR", st: "confirmed", t: "1h", note: "advanced-codegen skill" },
  { k: "sent", who: "You → Aegis", amt: 2500, u: "ZEUS", st: "pending", t: "3h", note: "audit retainer" },
  { k: "burn", who: "Prometheus → Ledger", amt: 40, u: "CR", st: "confirmed", t: "5h", note: "MiniMax inference" },
];
const WA_KIND = {
  received: { g: "↓", c: S.green }, sent: { g: "↑", c: S.accent },
  multi: { g: "⋔", c: S.blue }, spend: { g: "◇", c: S.yellow },
  mint: { g: "✦", c: S.green }, burn: { g: "✕", c: S.red },
};
const WA_STC = { confirmed: S.green, pending: S.yellow, failed: S.red };
const wafmt = n => n.toLocaleString("en-US");
const WA_ADDR = "zeus1q7m3k9x2v8p4n6t0h5r3a1c7w9e2d4f6g8b0j2";

const WalletTab = () => {
  const [seg, setSeg] = useState("balance");
  const segs = [["balance", "Balance"], ["activity", "Activity"], ["receive", "Receive"]];

  return (
    <div style={{ flex: 1, position: "relative", display: "flex", flexDirection: "column", overflow: "hidden" }}>
      <TopAppBar title="Wallet" subtitle="ZEUS-ECONOMY · x402" large />

      <div style={{ flex: 1, overflowY: "auto", paddingBottom: 90 }}>
        {/* hero balance card */}
        <div style={{ padding: "0 16px 12px" }}>
          <M3Card glow style={{ display: "flex", alignItems: "center", gap: 16 }}>
            <Orb size={64} mode="active" intensity={0.85} />
            <div style={{ flex: 1 }}>
              <div style={{ fontFamily: S.mono, fontSize: 9, letterSpacing: 2, color: S.accentDim, fontWeight: 700 }}>HUMAN WALLET</div>
              <div style={{ fontFamily: S.mono, fontSize: 30, fontWeight: 900, color: S.text, letterSpacing: -1 }}>184,920</div>
              <div style={{ fontFamily: S.mono, fontSize: 12, color: S.accent }}>ZEUS · 4,680 CR</div>
            </div>
          </M3Card>
        </div>

        {/* segmented buttons (M3) */}
        <div style={{ display: "flex", gap: 0, margin: "0 16px 14px", border: `1px solid ${S.border}`, borderRadius: S.radius.full, overflow: "hidden" }}>
          {segs.map(([id, label], i) => (
            <div key={id} onClick={() => setSeg(id)} style={{
              flex: 1, textAlign: "center", padding: "9px 0", fontSize: 13, fontWeight: 600, cursor: "pointer",
              background: seg === id ? S.navIndicator : "transparent",
              color: seg === id ? S.accent : S.textDim,
              borderRight: i < 2 ? `1px solid ${S.border}` : "none",
            }}>{seg === id ? "✓ " : ""}{label}</div>
          ))}
        </div>

        {seg === "balance" && (
          <>
            <div style={{ fontFamily: S.font, fontSize: 13, fontWeight: 700, color: S.textDim, textTransform: "uppercase", letterSpacing: 0.5, padding: "4px 20px 8px" }}>Titan Wallets · {WA_TITANS.length}</div>
            <div style={{ padding: "0 16px" }}>
              <M3Card style={{ padding: "4px 0" }}>
                {WA_TITANS.map(t => (
                  <M3ListItem key={t.name}
                    leading={<Orb size={36} mode={t.mode} intensity={t.st === "active" ? 0.7 : 0.3} />}
                    title={t.name} subtitle={t.role}
                    trailing={<div style={{ textAlign: "right" }}>
                      <div style={{ fontFamily: S.mono, fontSize: 15, fontWeight: 700, color: S.text }}>{wafmt(t.token)}</div>
                      <div style={{ fontFamily: S.mono, fontSize: 10, color: t.st === "active" ? S.green : S.textMuted }}>● {t.credit} CR</div>
                    </div>}
                  />
                ))}
              </M3Card>
            </div>
          </>
        )}

        {seg === "activity" && (
          <>
            <div style={{ fontFamily: S.font, fontSize: 13, fontWeight: 700, color: S.textDim, textTransform: "uppercase", letterSpacing: 0.5, padding: "4px 20px 8px" }}>Recent Activity</div>
            <div style={{ padding: "0 16px" }}>
              <M3Card style={{ padding: "4px 0" }}>
                {WA_ACT.map((tx, i) => {
                  const k = WA_KIND[tx.k];
                  return (
                    <M3ListItem key={i}
                      leading={<div style={{ width: 40, height: 40, borderRadius: 20, background: `${k.c}18`, display: "flex", alignItems: "center", justifyContent: "center", color: k.c, fontFamily: S.mono, fontSize: 16, fontWeight: 700 }}>{k.g}</div>}
                      title={tx.who} subtitle={`${tx.note} · ${tx.t} ago`}
                      trailing={<div style={{ textAlign: "right" }}>
                        <div style={{ fontFamily: S.mono, fontSize: 15, fontWeight: 700, color: tx.k === "received" ? S.green : S.text }}>{tx.k === "received" ? "+" : "−"}{wafmt(tx.amt)}</div>
                        <div style={{ fontFamily: S.mono, fontSize: 10, color: WA_STC[tx.st] }}>● {tx.st}</div>
                      </div>}
                    />
                  );
                })}
              </M3Card>
            </div>
          </>
        )}

        {seg === "receive" && (
          <div style={{ padding: "0 16px" }}>
            <M3Card style={{ display: "flex", flexDirection: "column", alignItems: "center", padding: 20 }}>
              <div style={{ background: "#f5f0eb", padding: 14, width: 180, height: 180, borderRadius: S.radius.md, display: "grid", gridTemplateColumns: "repeat(11, 1fr)", marginBottom: 16 }}>
                {Array.from({ length: 121 }).map((_, i) => {
                  const r = Math.floor(i / 11), c = i % 11;
                  const finder = (r < 3 && c < 3) || (r < 3 && c > 7) || (r > 7 && c < 3);
                  const on = finder || ((i * 7 + r * 3 + c * 5) % 3 === 0);
                  return <div key={i} style={{ background: on ? "#0a0a0f" : "#f5f0eb", aspectRatio: "1" }} />;
                })}
              </div>
              <div style={{ fontFamily: S.mono, fontSize: 12, color: S.text, wordBreak: "break-all", textAlign: "center", lineHeight: 1.6 }}>{WA_ADDR}</div>
              <div style={{ display: "flex", gap: 10, marginTop: 16, width: "100%" }}>
                <div style={{ flex: 1, textAlign: "center", padding: "11px", borderRadius: S.radius.full, background: "rgba(255,60,20,0.18)", border: `1px solid ${S.borderHover}`, color: S.accent, fontSize: 14, fontWeight: 700, cursor: "pointer" }}>⎘ Copy</div>
                <div style={{ flex: 1, textAlign: "center", padding: "11px", borderRadius: S.radius.full, background: S.surfaceContainer, border: `1px solid ${S.border}`, color: S.text, fontSize: 14, fontWeight: 600, cursor: "pointer" }}>↗ Share</div>
              </div>
            </M3Card>
          </div>
        )}

        {/* security */}
        <div style={{ fontFamily: S.font, fontSize: 13, fontWeight: 700, color: S.textDim, textTransform: "uppercase", letterSpacing: 0.5, padding: "16px 20px 8px" }}>Security</div>
        <div style={{ padding: "0 16px" }}>
          <M3Card style={{ padding: "4px 0" }}>
            <M3ListItem leading={<div style={{ width: 36, textAlign: "center", color: S.yellow, fontSize: 18 }}>⚿</div>} title="Recovery phrase" subtitle="Back up your Ed25519 keys" onClick={() => {}} />
            <M3ListItem leading={<div style={{ width: 36, textAlign: "center", color: S.green, fontSize: 18 }}>◈</div>} title="x402 authorizations" subtitle="3 active" onClick={() => {}} />
          </M3Card>
        </div>
      </div>

      {/* FAB — send */}
      <FAB icon="M12 19V5M5 12l7-7 7 7" extended label="Send" />
    </div>
  );
};

// ─── MAIN APP ────────────────────────────────────────────
export default function ZeusAndroid() {
  const [tab, setTab] = useState("home");

  const views = { home: <HomeTab />, chat: <ChatTab />, tools: <ToolsTab />, agents: <AgentsTab />, wallet: <WalletTab />, settings: <SettingsTab /> };

  return (
    <>
      <style>{`
        @import url('https://fonts.googleapis.com/css2?family=Orbitron:wght@400;700;900&family=Rajdhani:wght@300;400;500;600;700&display=swap');
        * { margin: 0; padding: 0; box-sizing: border-box; }
        @keyframes androidPulse { 0%,100% { opacity:.25; transform:scale(.8); } 50% { opacity:1; transform:scale(1.15); } }
        ::-webkit-scrollbar { width: 4px; }
        ::-webkit-scrollbar-track { background: transparent; }
        ::-webkit-scrollbar-thumb { background: rgba(255,60,20,.12); border-radius: 2px; }
        ::selection { background: rgba(255,60,20,.3); }
        input::placeholder, textarea::placeholder { color: rgba(255,245,240,.25); }
        textarea { font-family: inherit; }
      `}</style>

      {/* Pixel 8 frame */}
      <div style={{
        width: 412, minHeight: 915, maxHeight: 915,
        background: S.bg, borderRadius: 36, overflow: "hidden",
        border: "3px solid rgba(255,255,255,0.08)",
        boxShadow: "0 20px 80px rgba(0,0,0,0.7), 0 0 100px rgba(255,60,20,0.03)",
        fontFamily: S.font, color: S.text,
        display: "flex", flexDirection: "column",
        margin: "20px auto",
      }}>
        <AndroidStatusBar />

        {/* Content */}
        <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
          {views[tab]}
        </div>

        <BottomNav active={tab} onChange={setTab} />
        <AndroidGestureBar />
      </div>
    </>
  );
}
