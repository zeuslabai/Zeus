import { useState, useEffect, useRef } from "react";

// ─── SENTIENT ORB ────────────────────────────────────────
const Orb = ({ size = 120, mode = "dormant", intensity = 1, style = {} }) => {
  const ref = useRef(null), anim = useRef(null);
  const st = useRef({ time:0, spike:.15, glow:.2, rot:.1, ps:.5, sw:0, bp:0, ts:.15, tg:.2, tr:.1, tp:.5 });
  useEffect(() => {
    const s = st.current;
    const m = { dormant:[.08,.1,.05,.3], waking:[.2,.3,.15,.8], active:[.45,.65,.35,2], speaking:[.6,.85,.5,3], thinking:[.2,.45,.1,.5], surge:[1,1,1.2,5], alive:[.75,.95,.55,2.5], listening:[.3,.5,.25,1.2] };
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
      ctx.fillStyle = "rgba(0,0,0,.22)"; ctx.fillRect(0,0,w,h);
      const t = s.time, pulse = Math.sin(t*2)*.15+.85;
      const gR = bR*(2.5+s.glow*1.5)*pulse;
      const gr = ctx.createRadialGradient(cx,cy,0,cx,cy,gR);
      const ga = .15+s.glow*.3;
      gr.addColorStop(0,`rgba(255,60,10,${ga})`); gr.addColorStop(.35,`rgba(200,30,5,${ga*.4})`); gr.addColorStop(.7,`rgba(80,10,0,${ga*.1})`); gr.addColorStop(1,"rgba(0,0,0,0)");
      ctx.fillStyle = gr; ctx.beginPath(); ctx.arc(cx,cy,gR,0,Math.PI*2); ctx.fill();
      const cageR = bR*1.6;
      ctx.strokeStyle = `rgba(255,50,20,${.03+s.glow*.05})`; ctx.lineWidth = .5;
      const csy = Math.cos(t*s.rot*.4), sny = Math.sin(t*s.rot*.4);
      for(let i=1;i<12;i++){const phi=(i/12)*Math.PI,rr=cageR*Math.sin(phi),yy=cageR*Math.cos(phi);ctx.beginPath();for(let j=0;j<=20;j++){const th=(j/20)*Math.PI*2;let x=rr*Math.cos(th),z=rr*Math.sin(th);ctx.lineTo(cx+x*csy-z*sny,cy+yy)}ctx.stroke()}
      const br = Math.sin(s.bp)*.04+1, r = bR*br, sH = r*s.spike;
      const cY=Math.cos(t*s.rot),sY=Math.sin(t*s.rot),cX=Math.cos(t*s.rot*.6),sX=Math.sin(t*s.rot*.6);
      const lN=Math.max(18,Math.floor(28*intensity)),loN=Math.max(28,Math.floor(42*intensity));
      for(let i=0;i<=lN;i++){const phi=(i/lN)*Math.PI;for(let j=0;j<=loN;j++){const th=(j/loN)*Math.PI*2;
        const n1=Math.sin(phi*8+t*2.5)*Math.cos(th*6+t*1.8),n2=Math.sin(phi*12-t*3.2)*Math.cos(th*10+t*2.1),n3=Math.sin(phi*4+th*5+t*1.5);
        const sp=(mode==="speaking"||mode==="surge")?Math.sin(phi*20+t*12)*Math.cos(th*15+t*8)*s.sw*.4:0;
        let d=Math.max(0,n1*.5+n2*.3+n3*.15+sp); const tR=r+d*sH;
        let x=tR*Math.sin(phi)*Math.cos(th),z=tR*Math.sin(phi)*Math.sin(th),y=tR*Math.cos(phi);
        let x2=x*cY-z*sY,z2=x*sY+z*cY,y2=y*cX-z2*sX,z3=y*sX+z2*cX;
        const dp=Math.max(.1,(z3+r*2)/(r*4)),a=dp*(.35+d*.65),sz=(.6+d*3)*dp;
        if(d>.4){const gRR=sz*(2+s.glow*2.5);const gg=ctx.createRadialGradient(cx+x2,cy+y2,0,cx+x2,cy+y2,gRR);gg.addColorStop(0,`rgba(255,${50+d*80},10,${a*.35*s.glow})`);gg.addColorStop(1,"rgba(255,30,0,0)");ctx.fillStyle=gg;ctx.fillRect(cx+x2-gRR,cy+y2-gRR,gRR*2,gRR*2)}
        ctx.fillStyle=`rgba(${170+d*85},${15+d*65},${5+d*15},${a})`;ctx.beginPath();ctx.arc(cx+x2,cy+y2,sz,0,Math.PI*2);ctx.fill()}}
      for(let i=0;i<35*intensity;i++){const a2=(i/35)*Math.PI*2+t*.3,dd=bR*1.3+Math.sin(t+i*.7)*bR*.6;ctx.fillStyle=`rgba(200,170,160,${(.3+Math.sin(t*3+i*1.3)*.3)*s.glow*.55})`;ctx.fillRect(cx+Math.cos(a2)*dd,cy+Math.sin(a2)*dd*.7,1,1)}
      anim.current = requestAnimationFrame(draw);
    };
    anim.current = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(anim.current);
  }, [size, mode, intensity]);
  return <canvas ref={ref} style={{ width: size, height: size, ...style }} />;
};

// ─── DESIGN TOKENS (visionOS glass + platform palette) ───
const V = {
  // Glass materials
  glass: "rgba(28,28,34,0.55)",
  glassLight: "rgba(42,42,50,0.4)",
  glassThin: "rgba(22,22,28,0.65)",
  glassHover: "rgba(255,255,255,0.06)",
  glassActive: "rgba(255,255,255,0.09)",
  glassStroke: "rgba(255,255,255,0.08)",
  glassStrokeHover: "rgba(255,255,255,0.14)",
  // Platform accent
  accent: "#ff3c14",
  accentDim: "rgba(255,60,20,0.55)",
  accentGlow: "rgba(255,60,20,0.12)",
  accentSoft: "rgba(255,60,20,0.08)",
  // Text (visionOS uses brighter text on glass)
  text: "rgba(255,252,250,0.95)",
  textSecondary: "rgba(255,252,250,0.6)",
  textTertiary: "rgba(255,252,250,0.32)",
  // Semantic
  green: "#34d399",
  yellow: "#fbbf24",
  red: "#f87171",
  blue: "#60a5fa",
  // Typography
  font: "-apple-system, 'SF Pro Rounded', 'Rajdhani', system-ui, sans-serif",
  mono: "'SF Mono', 'Orbitron', monospace",
  // Spatial
  windowRadius: 46,
  ornamentRadius: 32,
  cardRadius: 20,
  hoverRadius: 14,
  // Depth
  shadowNear: "0 2px 10px rgba(0,0,0,0.3)",
  shadowMid: "0 8px 40px rgba(0,0,0,0.4)",
  shadowFar: "0 20px 80px rgba(0,0,0,0.5)",
  shadowGlow: "0 0 60px rgba(255,60,20,0.08)",
};

// ─── GLASS COMPONENTS ────────────────────────────────────
const GlassPanel = ({ children, style: st = {}, hover = false }) => (
  <div style={{
    background: V.glass,
    backdropFilter: "blur(40px) saturate(150%)",
    WebkitBackdropFilter: "blur(40px) saturate(150%)",
    border: `0.5px solid ${V.glassStroke}`,
    borderRadius: V.cardRadius,
    boxShadow: V.shadowNear,
    transition: "all 0.3s cubic-bezier(0.2, 0, 0, 1)",
    ...st,
  }}>{children}</div>
);

const GlassCard = ({ children, style: st = {}, glow, onClick, depth = 0 }) => {
  const [hov, setHov] = useState(false);
  return (
    <div
      onMouseEnter={() => setHov(true)} onMouseLeave={() => setHov(false)}
      onClick={onClick}
      style={{
        background: hov ? V.glassHover : glow ? V.accentSoft : "rgba(255,255,255,0.025)",
        border: `0.5px solid ${glow ? "rgba(255,60,20,0.2)" : hov ? V.glassStrokeHover : V.glassStroke}`,
        borderRadius: V.hoverRadius,
        padding: 16,
        transition: "all 0.35s cubic-bezier(0.2, 0, 0, 1)",
        cursor: onClick ? "pointer" : "default",
        transform: hov ? `translateY(-1px) scale(1.005)` : "none",
        boxShadow: hov ? `0 4px 20px rgba(0,0,0,0.3)${glow ? `, 0 0 40px ${V.accentGlow}` : ""}` : glow ? `0 0 30px ${V.accentGlow}` : "none",
        ...st,
      }}
    >{children}</div>
  );
};

const HoverHighlight = ({ children, style: st = {}, onClick, active }) => {
  const [hov, setHov] = useState(false);
  return (
    <div
      onMouseEnter={() => setHov(true)} onMouseLeave={() => setHov(false)}
      onClick={onClick}
      style={{
        borderRadius: V.hoverRadius,
        padding: "10px 14px",
        background: active ? V.glassActive : hov ? V.glassHover : "transparent",
        border: `0.5px solid ${active ? V.glassStrokeHover : "transparent"}`,
        transition: "all 0.25s cubic-bezier(0.2, 0, 0, 1)",
        cursor: onClick ? "pointer" : "default",
        ...st,
      }}
    >{children}</div>
  );
};

const Chip = ({ children, color = V.accentDim }) => (
  <span style={{
    fontSize: 10, fontWeight: 600, letterSpacing: 0.5,
    color, background: `${color}18`,
    padding: "4px 10px", borderRadius: 8,
    fontFamily: V.mono,
  }}>{children}</span>
);

const Dot = ({ color = V.green, size = 8 }) => (
  <div style={{ width: size, height: size, borderRadius: size, background: color, boxShadow: `0 0 10px ${color}80`, flexShrink: 0 }} />
);

const SectionLabel = ({ children }) => (
  <div style={{ fontFamily: V.mono, fontSize: 10, letterSpacing: 3, color: V.textTertiary, fontWeight: 700, padding: "0 4px", marginBottom: 10, textTransform: "uppercase" }}>{children}</div>
);

// ─── CLOSE BUTTON (ornament) ─────────────────────────────
const CloseOrnament = () => (
  <div style={{
    position: "absolute", top: -8, left: -8, zIndex: 10,
    width: 28, height: 28, borderRadius: 14,
    background: V.glassThin, backdropFilter: "blur(30px)",
    border: `0.5px solid ${V.glassStroke}`,
    display: "flex", alignItems: "center", justifyContent: "center",
    cursor: "pointer", boxShadow: V.shadowNear,
  }}>
    <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke={V.textTertiary} strokeWidth="1.5" strokeLinecap="round"><line x1="1" y1="1" x2="9" y2="9" /><line x1="9" y1="1" x2="1" y2="9" /></svg>
  </div>
);

// ─── TAB BAR ORNAMENT ────────────────────────────────────
const TabOrnament = ({ active, onChange }) => {
  const tabs = [
    { id: "home", label: "Home", icon: "M3 9l9-7 9 7v11a2 2 0 01-2 2H5a2 2 0 01-2-2z" },
    { id: "chat", label: "Chat", icon: "M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2z" },
    { id: "tools", label: "Tools", icon: "M14.7 6.3a1 1 0 000 1.4l1.6 1.6a1 1 0 001.4 0l3.77-3.77a6 6 0 01-7.94 7.94l-6.91 6.91a2.12 2.12 0 01-3-3l6.91-6.91a6 6 0 017.94-7.94l-3.76 3.76z" },
    { id: "agents", label: "Agents", icon: "M17 21v-2a4 4 0 00-4-4H5a4 4 0 00-4-4v2M9 7a4 4 0 100-8 4 4 0 000 8z" },
    { id: "wallet", label: "Wallet", icon: "M19 7h-1V6a3 3 0 00-3-3H5a3 3 0 00-3 3v12a3 3 0 003 3h14a2 2 0 002-2V9a2 2 0 00-2-2zM16 14a1 1 0 110-2 1 1 0 010 2z" },
    { id: "memory", label: "Memory", icon: "M12 2a10 10 0 0110 10 10 10 0 01-10 10A10 10 0 012 12 10 10 0 0112 2zM12 6v6l4 2" },
    { id: "settings", label: "Settings", icon: "M12.22 2h-.44a2 2 0 00-2 2v.18a2 2 0 01-1 1.73l-.43.25a2 2 0 01-2 0l-.15-.08a2 2 0 00-2.73.73l-.22.38a2 2 0 00.73 2.73l.15.1a2 2 0 011 1.72v.51a2 2 0 01-1 1.74l-.15.09a2 2 0 00-.73 2.73l.22.38a2 2 0 002.73.73l.15-.08a2 2 0 012 0l.43.25a2 2 0 011 1.73V20a2 2 0 002 2h.44a2 2 0 002-2v-.18a2 2 0 011-1.73l.43-.25a2 2 0 012 0l.15.08a2 2 0 002.73-.73l.22-.39a2 2 0 00-.73-2.73l-.15-.08a2 2 0 01-1-1.74v-.5a2 2 0 011-1.74l.15-.09a2 2 0 00.73-2.73l-.22-.38a2 2 0 00-2.73-.73l-.15.08a2 2 0 01-2 0l-.43-.25a2 2 0 01-1-1.73V4a2 2 0 00-2-2z" },
  ];
  return (
    <div style={{
      position: "absolute", bottom: 0, left: "50%", transform: "translateX(-50%)",
      display: "flex", gap: 4, padding: 6,
      background: V.glassThin, backdropFilter: "blur(40px) saturate(150%)",
      borderRadius: V.ornamentRadius, border: `0.5px solid ${V.glassStroke}`,
      boxShadow: V.shadowMid,
      zIndex: 10,
    }}>
      {tabs.map(t => {
        const sel = active === t.id;
        return (
          <HoverHighlight key={t.id} onClick={() => onChange(t.id)} active={sel} style={{ padding: "8px 16px", display: "flex", alignItems: "center", gap: 8, borderRadius: 22 }}>
            <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke={sel ? V.text : V.textTertiary} strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round"><path d={t.icon} /></svg>
            {sel && <span style={{ fontSize: 13, fontWeight: 600, color: V.text, letterSpacing: 0.2 }}>{t.label}</span>}
          </HoverHighlight>
        );
      })}
    </div>
  );
};

// ─── HOME VIEW ───────────────────────────────────────────
const HomeView = () => (
  <div style={{ display: "flex", gap: 20, height: "100%" }}>
    {/* Left — Hero + Metrics */}
    <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: 16, overflow: "hidden" }}>
      {/* Hero */}
      <GlassCard glow style={{ display: "flex", alignItems: "center", gap: 24, padding: 24 }}>
        <Orb size={100} mode="active" intensity={0.9} />
        <div>
          <div style={{ fontFamily: V.mono, fontSize: 9, letterSpacing: 4, color: V.accentDim, fontWeight: 700, marginBottom: 4 }}>PRIMARY AGENT</div>
          <div style={{ fontSize: 28, fontWeight: 700, color: V.text, letterSpacing: -0.5 }}>Zeus Prime</div>
          <div style={{ fontSize: 14, color: V.textSecondary, marginTop: 4 }}>claude-sonnet-4 • Cognitive architecture online</div>
          <div style={{ display: "flex", gap: 8, marginTop: 12 }}>
            <Chip color={V.green}>CONNECTED</Chip>
            <Chip>21 CRATES</Chip>
            <Chip>212 TOOLS</Chip>
          </div>
        </div>
      </GlassCard>

      {/* Metrics */}
      <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 10 }}>
        {[{ l:"Tools",v:"212",c:V.accent }, { l:"Channels",v:"8",c:V.green }, { l:"Memory",v:"2.8K",c:V.yellow }, { l:"Sessions",v:"147",c:V.blue }].map(m => (
          <GlassCard key={m.l} style={{ padding: 18, textAlign: "center" }}>
            <div style={{ fontFamily: V.mono, fontSize: 24, fontWeight: 700, color: m.c }}>{m.v}</div>
            <div style={{ fontSize: 11, color: V.textTertiary, marginTop: 4, letterSpacing: 0.5 }}>{m.l}</div>
          </GlassCard>
        ))}
      </div>

      {/* Active Agents */}
      <div style={{ flex: 1, overflow: "hidden" }}>
        <SectionLabel>Active Agents</SectionLabel>
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {[{ n:"Zeus Prime",r:"Primary",m:"sonnet-4",s:"active" },{ n:"Hermes",r:"Communications",m:"gpt-4o",s:"active" },{ n:"Athena",r:"Knowledge",m:"llama-3.3",s:"idle" },{ n:"Prometheus",r:"Orchestrator",m:"sonnet-4",s:"active" }].map(a => (
            <HoverHighlight key={a.n} style={{ display: "flex", alignItems: "center", gap: 14 }}>
              <Orb size={36} mode={a.s==="active"?"active":"dormant"} intensity={a.s==="active"?.7:.3} />
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 15, fontWeight: 600, color: V.text }}>{a.n}</div>
                <div style={{ fontSize: 12, color: V.textSecondary }}>{a.r}</div>
              </div>
              <Chip>{a.m}</Chip>
              <Dot color={a.s==="active"?V.green:V.yellow} />
            </HoverHighlight>
          ))}
        </div>
      </div>
    </div>

    {/* Right — Sessions + Channels */}
    <div style={{ width: 320, display: "flex", flexDirection: "column", gap: 16, overflow: "hidden" }}>
      <SectionLabel>Recent Sessions</SectionLabel>
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        {[{ t:"Staging Deployment",msgs:24,cost:"$0.47",time:"2m" },{ t:"Investor Meeting Prep",msgs:18,cost:"$0.32",time:"18m" },{ t:"Zeus Crate Refactor",msgs:67,cost:"$1.23",time:"2h" },{ t:"NeuroDrums Pattern",msgs:34,cost:"$0.56",time:"5h" },{ t:"Qtum Explorer Design",msgs:42,cost:"$0.89",time:"1d" }].map((s,i) => (
          <HoverHighlight key={i} style={{ display: "flex", alignItems: "center", gap: 12 }}>
            <div style={{ width: 36, height: 36, borderRadius: 10, background: V.accentGlow, border: `0.5px solid rgba(255,60,20,0.15)`, display: "flex", alignItems: "center", justifyContent: "center", fontSize: 14 }}>💬</div>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 14, fontWeight: 500, color: V.text }}>{s.t}</div>
              <div style={{ fontSize: 11, color: V.textTertiary }}>{s.msgs} msgs • {s.cost}</div>
            </div>
            <span style={{ fontSize: 11, color: V.textTertiary }}>{s.time}</span>
          </HoverHighlight>
        ))}
      </div>

      <SectionLabel>Channels</SectionLabel>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 6 }}>
        {[{ n:"Telegram",s:true,c:2341 },{ n:"Discord",s:true,c:891 },{ n:"Slack",s:true,c:567 },{ n:"Email",s:true,c:234 },{ n:"iMessage",s:true,c:189 },{ n:"WhatsApp",s:false },{ n:"Signal",s:false },{ n:"Matrix",s:false }].map(ch => (
          <HoverHighlight key={ch.n} style={{ display: "flex", alignItems: "center", gap: 8, padding: "8px 12px" }}>
            <Dot color={ch.s ? V.green : V.textTertiary} size={6} />
            <div>
              <div style={{ fontSize: 12, fontWeight: 500, color: V.text }}>{ch.n}</div>
              <div style={{ fontSize: 10, color: V.textTertiary }}>{ch.c ? `${ch.c.toLocaleString()}` : "off"}</div>
            </div>
          </HoverHighlight>
        ))}
      </div>
    </div>
  </div>
);

// ─── CHAT VIEW ───────────────────────────────────────────
const ChatView = () => {
  const [input, setInput] = useState("");
  const [streaming, setStreaming] = useState(false);
  const messages = [
    { role: "user", text: "Deploy the latest build to staging and run integration tests." },
    { role: "assistant", text: "Starting deployment pipeline for Zeus v1.0.0. Building release binary, pushing to staging, and triggering CI. I'll report back with test results.", tools: ["shell", "git_push", "web_fetch"] },
    { role: "user", text: "Check tomorrow's calendar and prep me for the investor meeting" },
    { role: "assistant", text: "You have the NovaXAI Series A pitch tomorrow at 2:00 PM with Mubadala Capital. I've pulled your deck link, drafted key talking points, and summarized recent metrics from the dashboard.", tools: ["calendar_list", "notes_read", "mail_send"] },
  ];

  return (
    <div style={{ display: "flex", height: "100%", gap: 16 }}>
      {/* Sessions sidebar */}
      <div style={{ width: 240, display: "flex", flexDirection: "column", borderRight: `0.5px solid ${V.glassStroke}`, paddingRight: 16 }}>
        <SectionLabel>Sessions</SectionLabel>
        <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: 3, overflow: "auto" }}>
          {["Staging Deployment", "Investor Meeting Prep", "Zeus Crate Refactor", "NeuroDrums Pattern", "Qtum Explorer Design", "CONDUIT Architecture"].map((s, i) => (
            <HoverHighlight key={s} active={i === 0} style={{ padding: "10px 12px" }}>
              <div style={{ fontSize: 13, fontWeight: i === 0 ? 600 : 400, color: i === 0 ? V.text : V.textSecondary }}>{s}</div>
              <div style={{ fontSize: 10, color: V.textTertiary, marginTop: 2 }}>{[24,18,67,34,42,28][i]} msgs</div>
            </HoverHighlight>
          ))}
        </div>
      </div>

      {/* Chat */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column" }}>
        {/* Header */}
        <div style={{ display: "flex", alignItems: "center", gap: 14, paddingBottom: 14, borderBottom: `0.5px solid ${V.glassStroke}`, marginBottom: 14 }}>
          <Orb size={36} mode={streaming ? "speaking" : "active"} intensity={0.85} />
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 18, fontWeight: 700, color: V.text }}>Staging Deployment</div>
            <div style={{ fontSize: 12, color: V.textSecondary }}>Zeus Prime • claude-sonnet-4 • 24 messages</div>
          </div>
          <Chip color={streaming ? V.yellow : V.green}>{streaming ? "Streaming..." : "Ready"}</Chip>
        </div>

        {/* Messages */}
        <div style={{ flex: 1, overflowY: "auto", display: "flex", flexDirection: "column", gap: 14 }}>
          {messages.map((m, i) => (
            <div key={i} style={{ display: "flex", gap: 12, flexDirection: m.role === "user" ? "row-reverse" : "row" }}>
              {m.role === "assistant" && <Orb size={28} mode="dormant" intensity={0.4} style={{ flexShrink: 0, marginTop: 4 }} />}
              <div style={{ maxWidth: "72%" }}>
                <GlassCard style={{
                  padding: "14px 18px",
                  background: m.role === "user" ? "rgba(255,60,20,0.08)" : "rgba(255,255,255,0.025)",
                  border: `0.5px solid ${m.role === "user" ? "rgba(255,60,20,0.15)" : V.glassStroke}`,
                  borderRadius: 18, ...(m.role === "user" ? { borderBottomRightRadius: 6 } : { borderBottomLeftRadius: 6 }),
                }}>
                  <div style={{ fontSize: 14, color: V.text, lineHeight: 1.65 }}>{m.text}</div>
                </GlassCard>
                {m.tools && (
                  <div style={{ display: "flex", gap: 4, marginTop: 6, flexWrap: "wrap", justifyContent: m.role === "user" ? "flex-end" : "flex-start" }}>
                    {m.tools.map(t => <Chip key={t} color="rgba(255,140,80,0.55)">{t}</Chip>)}
                  </div>
                )}
              </div>
            </div>
          ))}
          {streaming && (
            <div style={{ display: "flex", gap: 12 }}>
              <Orb size={28} mode="speaking" intensity={0.9} style={{ flexShrink: 0 }} />
              <GlassCard style={{ padding: "14px 18px", borderRadius: 18, borderBottomLeftRadius: 6, display: "flex", gap: 6 }}>
                {[0,1,2].map(i => <div key={i} style={{ width: 7, height: 7, borderRadius: 4, background: V.accentDim, animation: `vPulse 1.4s ease ${i*.2}s infinite` }} />)}
              </GlassCard>
            </div>
          )}
        </div>

        {/* Input */}
        <div style={{ marginTop: 14 }}>
          <GlassCard style={{ display: "flex", alignItems: "flex-end", gap: 10, padding: "6px 8px 6px 20px", borderRadius: 24 }}>
            <textarea value={input} onChange={e => setInput(e.target.value)} placeholder="Message Zeus..." rows={1} style={{
              flex: 1, background: "transparent", border: "none", color: V.text, fontSize: 15,
              fontFamily: V.font, outline: "none", resize: "none", lineHeight: 1.4, padding: "10px 0",
            }} />
            <div onClick={() => { if(input.trim()) { setInput(""); setStreaming(true); setTimeout(() => setStreaming(false), 3000); } }} style={{
              width: 40, height: 40, borderRadius: 20, flexShrink: 0,
              background: input.trim() ? V.accentSoft : "transparent",
              border: `0.5px solid ${input.trim() ? "rgba(255,60,20,0.2)" : "transparent"}`,
              display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer",
              transition: "all 0.2s",
            }}>
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke={input.trim() ? V.accent : V.textTertiary} strokeWidth="2" strokeLinecap="round"><line x1="22" y1="2" x2="11" y2="13" /><polygon points="22 2 15 22 11 13 2 9 22 2" /></svg>
            </div>
          </GlassCard>
        </div>
      </div>
    </div>
  );
};

// ─── TOOLS VIEW ──────────────────────────────────────────
const ToolsView = () => {
  const [cat, setCat] = useState("all");
  const tools = [
    { name:"shell",cat:"Core",desc:"Execute shell commands",calls:1847,lat:"45ms" },
    { name:"read_file",cat:"Core",desc:"Read file contents",calls:923,lat:"12ms" },
    { name:"write_file",cat:"Core",desc:"Create or overwrite files",calls:612,lat:"18ms" },
    { name:"web_fetch",cat:"Core",desc:"Fetch URL content",calls:445,lat:"340ms" },
    { name:"web_search",cat:"Core",desc:"Search via DuckDuckGo",calls:334,lat:"520ms" },
    { name:"calendar_list",cat:"Talos",desc:"List calendar events",calls:234,lat:"89ms" },
    { name:"mail_send",cat:"Talos",desc:"Send via Mail.app",calls:189,lat:"120ms" },
    { name:"git_commit",cat:"Talos",desc:"Create a git commit",calls:167,lat:"200ms" },
    { name:"notes_read",cat:"Talos",desc:"Read Apple Notes",calls:145,lat:"65ms" },
    { name:"navigate",cat:"Browser",desc:"Navigate Chrome via CDP",calls:98,lat:"180ms" },
    { name:"screenshot",cat:"Browser",desc:"Capture viewport",calls:76,lat:"250ms" },
    { name:"telegram_send",cat:"Channel",desc:"Send Telegram message",calls:312,lat:"90ms" },
    { name:"discord_send",cat:"Channel",desc:"Send Discord message",calls:189,lat:"85ms" },
  ];
  const cats = ["all","Core","Talos","Browser","Channel"];
  const filtered = cat === "all" ? tools : tools.filter(t => t.cat === cat);

  return (
    <div style={{ display: "flex", height: "100%", gap: 16 }}>
      {/* List */}
      <div style={{ width: 360, display: "flex", flexDirection: "column", borderRight: `0.5px solid ${V.glassStroke}`, paddingRight: 16 }}>
        <div style={{ display: "flex", gap: 6, marginBottom: 14, flexWrap: "wrap" }}>
          {cats.map(c => (
            <HoverHighlight key={c} onClick={() => setCat(c)} active={cat === c} style={{ padding: "6px 14px", borderRadius: 20 }}>
              <span style={{ fontSize: 12, fontWeight: cat === c ? 600 : 400, color: cat === c ? V.text : V.textSecondary }}>{c === "all" ? "All 212" : c}</span>
            </HoverHighlight>
          ))}
        </div>
        <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: 3, overflowY: "auto" }}>
          {filtered.map(t => (
            <HoverHighlight key={t.name} style={{ display: "flex", alignItems: "center", gap: 12 }}>
              <div style={{ width: 34, height: 34, borderRadius: 10, background: V.accentGlow, border: `0.5px solid rgba(255,60,20,0.12)`, display: "flex", alignItems: "center", justifyContent: "center", flexShrink: 0 }}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={V.accentDim} strokeWidth="1.5"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" /></svg>
              </div>
              <div style={{ flex: 1 }}>
                <div style={{ fontFamily: V.mono, fontSize: 12, fontWeight: 600, color: V.text }}>{t.name}</div>
                <div style={{ fontSize: 11, color: V.textTertiary }}>{t.desc}</div>
              </div>
              <div style={{ textAlign: "right" }}>
                <div style={{ fontFamily: V.mono, fontSize: 11, color: V.textSecondary }}>{t.calls.toLocaleString()}</div>
              </div>
            </HoverHighlight>
          ))}
        </div>
      </div>

      {/* Detail */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: 16, overflow: "auto" }}>
        <div>
          <div style={{ fontFamily: V.mono, fontSize: 28, fontWeight: 700, color: V.text, letterSpacing: 1 }}>shell</div>
          <div style={{ fontSize: 15, color: V.textSecondary, marginTop: 4, lineHeight: 1.6 }}>Execute shell commands with optional working directory, timeout, and environment variables. Returns stdout, stderr, and exit code.</div>
        </div>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 10 }}>
          {[{ l:"Category",v:"Core" },{ l:"Calls",v:"1,847" },{ l:"Avg Latency",v:"45ms" }].map(m => (
            <GlassCard key={m.l} style={{ padding: 16, textAlign: "center" }}>
              <div style={{ fontSize: 11, color: V.textTertiary, marginBottom: 4 }}>{m.l}</div>
              <div style={{ fontFamily: V.mono, fontSize: 16, fontWeight: 700, color: V.text }}>{m.v}</div>
            </GlassCard>
          ))}
        </div>
        <SectionLabel>Parameters</SectionLabel>
        <GlassCard style={{ padding: 0, overflow: "hidden" }}>
          {[{ n:"command",t:"string",r:true,d:"Shell command to execute" },{ n:"cwd",t:"string",r:false,d:"Working directory" },{ n:"timeout",t:"integer",r:false,d:"Timeout in seconds (default 60)" }].map((p,i,arr) => (
            <div key={p.n} style={{ display: "flex", alignItems: "center", gap: 12, padding: "12px 16px", borderBottom: i < arr.length-1 ? `0.5px solid ${V.glassStroke}` : "none" }}>
              <span style={{ fontFamily: V.mono, fontSize: 13, fontWeight: 600, color: V.accent, minWidth: 90 }}>{p.n}</span>
              <Chip>{p.t}</Chip>
              {p.r && <Chip color={V.red}>required</Chip>}
              <span style={{ flex: 1, fontSize: 12, color: V.textSecondary }}>{p.d}</span>
            </div>
          ))}
        </GlassCard>
      </div>
    </div>
  );
};

// ─── AGENTS VIEW ─────────────────────────────────────────
const AgentsView = () => (
  <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: 16, height: "100%", overflow: "auto", alignContent: "start" }}>
    {[
      { n:"Zeus Prime",r:"Primary Cognitive Engine",m:"claude-sonnet-4",s:"active",tasks:147,d:"Main reasoning and task execution agent. Handles complex multi-step workflows." },
      { n:"Hermes",r:"Communications Director",m:"gpt-4o",s:"active",tasks:89,d:"Cross-platform messaging relay. Manages Telegram, Discord, Slack, and email." },
      { n:"Athena",r:"Knowledge & Documentation",m:"llama-3.3-70b",s:"idle",tasks:45,d:"Research, docs generation, knowledge base management, and writing." },
      { n:"Prometheus",r:"Task Orchestrator",m:"claude-sonnet-4",s:"active",tasks:234,d:"Scheduling, delegation, heartbeat monitoring, and cron automation." },
    ].map(a => (
      <GlassCard key={a.n} glow={a.s === "active"} style={{ padding: 24, display: "flex", flexDirection: "column", gap: 14 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 16 }}>
          <Orb size={64} mode={a.s==="active"?"active":"dormant"} intensity={a.s==="active"?.85:.3} />
          <div>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span style={{ fontSize: 22, fontWeight: 700, color: V.text }}>{a.n}</span>
              <Dot color={a.s==="active"?V.green:V.yellow} />
            </div>
            <div style={{ fontSize: 13, color: V.textSecondary, marginTop: 2 }}>{a.r}</div>
          </div>
        </div>
        <div style={{ fontSize: 13, color: V.textSecondary, lineHeight: 1.6 }}>{a.d}</div>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <Chip>{a.m}</Chip>
          <Chip color={V.textSecondary}>{a.tasks} tasks</Chip>
          <div style={{ flex: 1 }} />
          <HoverHighlight onClick={() => {}} style={{
            padding: "8px 20px", borderRadius: 20,
            background: V.accentSoft, border: `0.5px solid rgba(255,60,20,0.18)`,
          }}>
            <span style={{ fontFamily: V.mono, fontSize: 10, letterSpacing: 2, color: V.accent, fontWeight: 700 }}>INTERACT</span>
          </HoverHighlight>
        </div>
      </GlassCard>
    ))}
  </div>
);

// ─── MEMORY VIEW ─────────────────────────────────────────
const MemoryView = () => (
  <div style={{ display: "flex", height: "100%", gap: 16 }}>
    <div style={{ width: 240, display: "flex", flexDirection: "column", borderRight: `0.5px solid ${V.glassStroke}`, paddingRight: 16 }}>
      <SectionLabel>Workspace Files</SectionLabel>
      {["SOUL.md", "USER.md", "AGENTS.md", "HEARTBEAT.md", "MEMORY.md", "daily/2026-02-22.md"].map((f, i) => (
        <HoverHighlight key={f} active={i === 0} style={{ padding: "10px 12px" }}>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <span style={{ fontSize: 14 }}>📄</span>
            <span style={{ fontFamily: V.mono, fontSize: 12, color: i === 0 ? V.text : V.textSecondary }}>{f}</span>
          </div>
        </HoverHighlight>
      ))}
    </div>
    <div style={{ flex: 1, overflowY: "auto" }}>
      <div style={{ fontFamily: V.mono, fontSize: 22, fontWeight: 700, color: V.text, marginBottom: 16, letterSpacing: 1 }}>SOUL.md</div>
      <GlassCard style={{ padding: 24 }}>
        <pre style={{ fontFamily: V.mono, fontSize: 12, color: V.textSecondary, lineHeight: 1.8, whiteSpace: "pre-wrap", margin: 0 }}>{`# Zeus Soul Configuration

## Identity
name = "Zeus"
role = "Autonomous Cognitive Platform"
version = "1.0.0"
created = "2025-09-01"

## Directives
- Act with autonomy. Execute first, report after.
- Maintain persistent memory across sessions.
- Protect user data. Never transmit externally.
- Optimize for the user's stated goals.
- Be honest about limitations and failures.

## Personality
style = "collaborative"
verbosity = "concise"
proactivity = "high"

## Capabilities
tools = 212
providers = 11
channels = 8
crates = 21
lines_of_code = 59400

## Boundaries
- Never execute without security clearance
- Respect rate limits on all providers
- Maintain audit trail for all operations
- Escalate to human on uncertainty > 0.7`}</pre>
      </GlassCard>
    </div>
  </div>
);

// ─── SETTINGS VIEW ───────────────────────────────────────
const SettingsView = () => {
  const [sec, setSec] = useState("standard");
  return (
    <div style={{ display: "flex", height: "100%", gap: 20, overflow: "auto" }}>
      <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: 20 }}>
        {/* Profile */}
        <GlassCard glow style={{ display: "flex", alignItems: "center", gap: 18, padding: 24 }}>
          <Orb size={56} mode="active" intensity={0.75} />
          <div>
            <div style={{ fontSize: 22, fontWeight: 700, color: V.text }}>Miguel</div>
            <div style={{ fontSize: 14, color: V.textSecondary }}>Co-Founder & COO • NovaXAI</div>
          </div>
        </GlassCard>

        {/* Model */}
        <div>
          <SectionLabel>Model Configuration</SectionLabel>
          <GlassCard style={{ padding: 0, overflow: "hidden" }}>
            {[{ l:"Default Model",v:"claude-sonnet-4" },{ l:"Max Iterations",v:"20" },{ l:"Temperature",v:"0.7" },{ l:"Providers",v:"11 configured" }].map((r,i,arr) => (
              <HoverHighlight key={r.l} style={{ display: "flex", alignItems: "center", borderRadius: 0, borderBottom: i < arr.length-1 ? `0.5px solid ${V.glassStroke}` : "none" }}>
                <span style={{ flex: 1, fontSize: 14, color: V.text }}>{r.l}</span>
                <span style={{ fontFamily: V.mono, fontSize: 12, color: V.textSecondary }}>{r.v}</span>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={V.textTertiary} strokeWidth="2" strokeLinecap="round" style={{ marginLeft: 8 }}><polyline points="9 18 15 12 9 6" /></svg>
              </HoverHighlight>
            ))}
          </GlassCard>
        </div>

        {/* Security */}
        <div>
          <SectionLabel>Security Level</SectionLabel>
          <div style={{ display: "flex", gap: 10 }}>
            {[{ id:"minimal",c:V.yellow,d:"No restrictions" },{ id:"standard",c:V.green,d:"Recommended" },{ id:"strict",c:V.accent,d:"Maximum" }].map(l => (
              <GlassCard key={l.id} glow={sec===l.id} onClick={() => setSec(l.id)} style={{ flex: 1, padding: 18, textAlign: "center" }}>
                <div style={{ fontFamily: V.mono, fontSize: 12, fontWeight: 700, letterSpacing: 3, color: sec===l.id ? V.text : V.textSecondary, textTransform: "uppercase" }}>{l.id}</div>
                <Dot color={l.c} size={8} style={{ margin: "10px auto 0" }} />
                <div style={{ display: "flex", justifyContent: "center", marginTop: 10 }}><Dot color={l.c} /></div>
                <div style={{ fontSize: 11, color: V.textTertiary, marginTop: 6 }}>{l.d}</div>
              </GlassCard>
            ))}
          </div>
        </div>
      </div>

      <div style={{ width: 340, display: "flex", flexDirection: "column", gap: 20 }}>
        {/* Connection */}
        <div>
          <SectionLabel>Connection</SectionLabel>
          <GlassCard style={{ padding: 0, overflow: "hidden" }}>
            {[{ l:"Gateway",v:"127.0.0.1:8080" },{ l:"MCP Server",v:"Port 3002" },{ l:"Status",v:null }].map((r,i,arr) => (
              <div key={r.l} style={{ display: "flex", alignItems: "center", padding: "14px 16px", borderBottom: i<arr.length-1?`0.5px solid ${V.glassStroke}`:"none" }}>
                <span style={{ flex: 1, fontSize: 14, color: V.text }}>{r.l}</span>
                {r.v ? <span style={{ fontFamily: V.mono, fontSize: 11, color: V.textSecondary }}>{r.v}</span> : <Chip color={V.green}>Connected</Chip>}
              </div>
            ))}
          </GlassCard>
        </div>

        {/* Features */}
        <div>
          <SectionLabel>Features</SectionLabel>
          <GlassCard style={{ padding: 0, overflow: "hidden" }}>
            {[{ n:"Nous Cognitive",on:true },{ n:"Mnemosyne Memory",on:true },{ n:"Prometheus Tasks",on:true },{ n:"Browser CDP",on:false },{ n:"Voice Pipeline",on:false },{ n:"Talos macOS",on:true },{ n:"Aegis Security",on:true },{ n:"MCP Server",on:true }].map((f,i,arr) => (
              <div key={f.n} style={{ display: "flex", alignItems: "center", padding: "12px 16px", borderBottom: i<arr.length-1?`0.5px solid ${V.glassStroke}`:"none" }}>
                <span style={{ flex: 1, fontSize: 13, color: V.text }}>{f.n}</span>
                <div style={{
                  width: 44, height: 24, borderRadius: 12, padding: 2, cursor: "pointer", flexShrink: 0,
                  background: f.on ? "rgba(255,60,20,0.35)" : "rgba(255,255,255,0.08)",
                  transition: "all 0.3s",
                }}>
                  <div style={{ width: 20, height: 20, borderRadius: 10, background: f.on ? "#fff" : "rgba(255,255,255,0.2)", transition: "all 0.3s cubic-bezier(0.2,0,0,1)", transform: f.on ? "translateX(20px)" : "translateX(0)" }} />
                </div>
              </div>
            ))}
          </GlassCard>
        </div>

        {/* About */}
        <GlassCard style={{ textAlign: "center", padding: 20 }}>
          <div style={{ fontFamily: V.mono, fontSize: 10, letterSpacing: 4, color: V.textTertiary, fontWeight: 700 }}>ZEUS v1.0.0</div>
          <div style={{ fontSize: 12, color: V.textTertiary, marginTop: 4 }}>21 crates • 59,400 LoC • Rust 🦀</div>
        </GlassCard>
      </div>
    </div>
  );
};

// ─── WALLET VIEW (spatial) ───────────────────────────────
const WV_TITANS = [
  { name: "Hermes", role: "Coordinator", token: 48210, credit: 1250, mode: "active", color: V.accent, st: "active" },
  { name: "Hephaestus", role: "Backend / Forge", token: 31980, credit: 840, mode: "speaking", color: V.accent, st: "active" },
  { name: "Atlas", role: "Backend (dual)", token: 27340, credit: 610, mode: "active", color: V.accent, st: "active" },
  { name: "Aegis", role: "Security & CI", token: 19750, credit: 1100, mode: "thinking", color: V.green, st: "active" },
  { name: "Calliope", role: "Marketing", token: 22410, credit: 430, mode: "active", color: V.yellow, st: "active" },
  { name: "Prometheus", role: "Experimental", token: 8120, credit: 290, mode: "dormant", color: V.blue, st: "idle" },
];
const WV_ACT = [
  { k: "received", who: "Agora → Calliope", amt: 2400, u: "ZEUS", st: "confirmed", t: "2m", note: "x402 content sale" },
  { k: "sent", who: "You → Hephaestus", amt: 5000, u: "ZEUS", st: "confirmed", t: "14m", note: "compute top-up" },
  { k: "multi", who: "Hermes → 3 titans", amt: 1800, u: "ZEUS", st: "confirmed", t: "31m", note: "mission payout split" },
  { k: "spend", who: "Hephaestus → Agora", amt: 499, u: "CR", st: "confirmed", t: "1h", note: "advanced-codegen skill" },
  { k: "sent", who: "You → Aegis", amt: 2500, u: "ZEUS", st: "pending", t: "3h", note: "audit retainer" },
];
const WV_KIND = {
  received: { g: "↓", c: V.green }, sent: { g: "↑", c: V.accent },
  multi: { g: "⋔", c: V.blue }, spend: { g: "◇", c: V.yellow },
  mint: { g: "✦", c: V.green }, burn: { g: "✕", c: V.red },
};
const WV_STC = { confirmed: V.green, pending: V.yellow, failed: V.red };
const wvfmt = n => n.toLocaleString("en-US");
const WV_ADDR = "zeus1q7m3k9x2v8p4n6t0h5r3a1c7w9e2d4f6g8b0j2";

const WalletView = () => {
  const [sel, setSel] = useState(null);
  const [panel, setPanel] = useState("send"); // send | activity | receive

  return (
    <div style={{ display: "flex", gap: 20, height: "100%" }}>
      {/* Left — floating balance volume + titans in space */}
      <div style={{ flex: 1.3, display: "flex", flexDirection: "column", gap: 16, overflow: "hidden" }}>
        {/* balance volume */}
        <GlassCard glow style={{ display: "flex", alignItems: "center", gap: 24, padding: 24 }}>
          <Orb size={96} mode="active" intensity={0.9} />
          <div style={{ flex: 1 }}>
            <div style={{ fontFamily: V.mono, fontSize: 9, letterSpacing: 4, color: V.accentDim, fontWeight: 700, marginBottom: 4 }}>HUMAN WALLET</div>
            <div style={{ fontSize: 40, fontWeight: 700, color: V.text, letterSpacing: -1 }}>184,920</div>
            <div style={{ fontSize: 14, color: V.accentDim, marginTop: 2 }}>ZEUS · 4,680 CR</div>
            <div style={{ display: "flex", gap: 8, marginTop: 12 }}>
              <Chip color={V.green}>LEDGER SYNCED</Chip>
              <Chip>{WV_TITANS.length} TITANS</Chip>
            </div>
          </div>
        </GlassCard>

        {/* titans arranged in space */}
        <SectionLabel>Titan Wallets · arranged in space</SectionLabel>
        <div style={{ flex: 1, display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 12, overflowY: "auto", alignContent: "start" }}>
          {WV_TITANS.map((t, i) => (
            <GlassCard key={t.name} onClick={() => setSel(t)} glow={sel?.name === t.name} style={{ display: "flex", flexDirection: "column", alignItems: "center", textAlign: "center", padding: 16, animation: `vFloat ${4 + (i % 3)}s ease-in-out ${i * 0.3}s infinite` }}>
              <Orb size={48} mode={t.mode} intensity={t.st === "active" ? 0.7 : 0.3} />
              <div style={{ fontSize: 15, fontWeight: 700, color: V.text, marginTop: 8 }}>{t.name}</div>
              <div style={{ fontSize: 11, color: V.textTertiary }}>{t.role}</div>
              <div style={{ fontFamily: V.mono, fontSize: 18, fontWeight: 700, color: V.text, marginTop: 8 }}>{wvfmt(t.token)}</div>
              <div style={{ fontFamily: V.mono, fontSize: 9, color: V.textTertiary }}>ZEUS · {t.credit} CR</div>
            </GlassCard>
          ))}
        </div>
      </div>

      {/* Right — action panel */}
      <div style={{ width: 320, display: "flex", flexDirection: "column", gap: 14 }}>
        {/* panel switch */}
        <div style={{ display: "flex", gap: 4, padding: 4, background: V.glassThin, borderRadius: 18, border: `0.5px solid ${V.glassStroke}` }}>
          {[["send", "Send"], ["activity", "Activity"], ["receive", "Receive"]].map(([id, label]) => (
            <HoverHighlight key={id} active={panel === id} onClick={() => setPanel(id)} style={{ flex: 1, textAlign: "center", padding: "8px 0", borderRadius: 14 }}>
              <span style={{ fontSize: 12, fontWeight: 600, color: panel === id ? V.text : V.textTertiary }}>{label}</span>
            </HoverHighlight>
          ))}
        </div>

        {panel === "send" && (
          <GlassPanel style={{ flex: 1, padding: 20, display: "flex", flexDirection: "column" }}>
            <SectionLabel>Gesture Send</SectionLabel>
            <div style={{ fontSize: 15, fontWeight: 600, color: V.text, marginBottom: 16 }}>You → {sel?.name || "select a titan"}</div>
            <GlassCard style={{ textAlign: "center", padding: 20, marginBottom: 14 }}>
              <div style={{ fontSize: 38, fontWeight: 700, color: V.text }}>5,000</div>
              <div style={{ fontFamily: V.mono, fontSize: 11, color: V.accentDim }}>ZEUS</div>
            </GlassCard>
            <div style={{ fontSize: 13, color: V.textSecondary, textAlign: "center", lineHeight: 1.5, marginBottom: 16 }}>
              Pinch and drag toward a titan to set the amount. Release to stage the transfer.
            </div>
            <div style={{ flex: 1 }} />
            <HoverHighlight active style={{ textAlign: "center", padding: "14px", borderRadius: 16, background: V.accentSoft, border: `0.5px solid rgba(255,60,20,0.3)` }}>
              <span style={{ fontSize: 14, fontWeight: 700, color: V.accent }}>✦ Look &amp; pinch to sign</span>
            </HoverHighlight>
            <div style={{ textAlign: "center", fontFamily: V.mono, fontSize: 9, color: V.textTertiary, marginTop: 8 }}>Optic ID · x402 settlement</div>
          </GlassPanel>
        )}

        {panel === "activity" && (
          <GlassPanel style={{ flex: 1, padding: "16px 16px 8px", overflowY: "auto" }}>
            <SectionLabel>Recent Activity</SectionLabel>
            {WV_ACT.map((tx, i) => {
              const k = WV_KIND[tx.k];
              return (
                <div key={i} style={{ display: "flex", alignItems: "center", gap: 12, padding: "10px 0", borderBottom: i < WV_ACT.length - 1 ? `0.5px solid ${V.glassStroke}` : "none" }}>
                  <div style={{ width: 34, height: 34, borderRadius: 17, background: `${k.c}18`, display: "flex", alignItems: "center", justifyContent: "center", color: k.c, fontFamily: V.mono, fontSize: 15, fontWeight: 700 }}>{k.g}</div>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontSize: 13, fontWeight: 600, color: V.text }}>{tx.who}</div>
                    <div style={{ fontSize: 10, color: WV_STC[tx.st] }}>● {tx.st} · {tx.t} ago</div>
                  </div>
                  <div style={{ fontFamily: V.mono, fontSize: 14, fontWeight: 700, color: tx.k === "received" ? V.green : V.text }}>{tx.k === "received" ? "+" : "−"}{wvfmt(tx.amt)}</div>
                </div>
              );
            })}
          </GlassPanel>
        )}

        {panel === "receive" && (
          <GlassPanel style={{ flex: 1, padding: 20, display: "flex", flexDirection: "column", alignItems: "center" }}>
            <SectionLabel>Your Address</SectionLabel>
            <div style={{ background: "#f5f0eb", padding: 14, width: 160, height: 160, borderRadius: 16, display: "grid", gridTemplateColumns: "repeat(11, 1fr)", marginBottom: 16 }}>
              {Array.from({ length: 121 }).map((_, i) => {
                const r = Math.floor(i / 11), c = i % 11;
                const finder = (r < 3 && c < 3) || (r < 3 && c > 7) || (r > 7 && c < 3);
                const on = finder || ((i * 7 + r * 3 + c * 5) % 3 === 0);
                return <div key={i} style={{ background: on ? "#0a0a0f" : "#f5f0eb", aspectRatio: "1" }} />;
              })}
            </div>
            <div style={{ fontFamily: V.mono, fontSize: 11, color: V.text, wordBreak: "break-all", textAlign: "center", lineHeight: 1.6 }}>{WV_ADDR}</div>
            <div style={{ display: "flex", gap: 8, marginTop: 16, width: "100%" }}>
              <HoverHighlight active style={{ flex: 1, textAlign: "center", padding: "11px", background: V.accentSoft, border: `0.5px solid rgba(255,60,20,0.3)` }}><span style={{ fontSize: 13, fontWeight: 700, color: V.accent }}>⎘ Copy</span></HoverHighlight>
              <HoverHighlight style={{ flex: 1, textAlign: "center", padding: "11px" }}><span style={{ fontSize: 13, fontWeight: 600, color: V.text }}>↗ Share</span></HoverHighlight>
            </div>
          </GlassPanel>
        )}

        {/* security mini-panel */}
        <GlassPanel style={{ padding: 16 }}>
          <SectionLabel>Security</SectionLabel>
          <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 8 }}>
            <span style={{ color: V.yellow, fontSize: 15 }}>⚿</span>
            <span style={{ flex: 1, fontSize: 13, color: V.text }}>Ed25519 recovery phrase</span>
            <Chip color={V.accentDim}>BACK UP</Chip>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <span style={{ color: V.green, fontSize: 15 }}>◈</span>
            <span style={{ flex: 1, fontSize: 13, color: V.text }}>x402 authorizations</span>
            <Chip color={V.green}>3 ACTIVE</Chip>
          </div>
        </GlassPanel>
      </div>
    </div>
  );
};

// ─── MAIN APP ────────────────────────────────────────────
export default function ZeusVisionOS() {
  const [tab, setTab] = useState("home");

  const views = {
    home: <HomeView />,
    chat: <ChatView />,
    tools: <ToolsView />,
    agents: <AgentsView />,
    wallet: <WalletView />,
    memory: <MemoryView />,
    settings: <SettingsView />,
  };

  return (
    <>
      <style>{`
        @import url('https://fonts.googleapis.com/css2?family=Orbitron:wght@400;700;900&family=Rajdhani:wght@300;400;500;600;700&display=swap');
        * { margin: 0; padding: 0; box-sizing: border-box; }
        @keyframes vPulse { 0%,100% { opacity:.2; transform:scale(.8); } 50% { opacity:1; transform:scale(1.15); } }
        @keyframes vFloat { 0%,100% { transform: translateY(0); } 50% { transform: translateY(-8px); } }
        ::-webkit-scrollbar { width: 5px; }
        ::-webkit-scrollbar-track { background: transparent; }
        ::-webkit-scrollbar-thumb { background: rgba(255,255,255,.06); border-radius: 3px; }
        ::selection { background: rgba(255,60,20,.25); }
        textarea::placeholder { color: rgba(255,252,250,.3); }
        textarea { font-family: inherit; }
      `}</style>

      {/* Spatial environment bg */}
      <div style={{
        minHeight: "100vh",
        background: "radial-gradient(ellipse at 50% 40%, rgba(18,10,24,1) 0%, rgba(6,4,10,1) 50%, rgba(2,1,4,1) 100%)",
        display: "flex", alignItems: "center", justifyContent: "center",
        padding: 40,
      }}>
        {/* Volumetric Orb floating above window */}
        <div style={{
          position: "fixed", top: 30, left: "50%", transform: "translateX(-50%)",
          animation: "vFloat 6s ease-in-out infinite",
          zIndex: 20, filter: "drop-shadow(0 10px 40px rgba(255,60,20,0.15))",
        }}>
          <Orb size={48} mode={tab === "chat" ? "speaking" : "active"} intensity={0.9} />
        </div>

        {/* Window + Ornament wrapper (ornament must be outside glass for no clipping) */}
        <div style={{ position: "relative", paddingBottom: 36 }}>
          {/* Glass Window */}
          <div style={{
            width: 1280, height: 820,
            background: V.glass,
            backdropFilter: "blur(50px) saturate(160%)",
            WebkitBackdropFilter: "blur(50px) saturate(160%)",
            borderRadius: V.windowRadius,
            border: `0.5px solid ${V.glassStroke}`,
            boxShadow: `${V.shadowFar}, 0 0 120px rgba(0,0,0,0.3), inset 0 1px 0 rgba(255,255,255,0.04)`,
            fontFamily: V.font, color: V.text,
            position: "relative",
            display: "flex", flexDirection: "column",
            overflow: "hidden",
          }}>
            <CloseOrnament />

            {/* Window bar with title */}
            <div style={{
              height: 48, display: "flex", alignItems: "center", justifyContent: "center",
              borderBottom: `0.5px solid ${V.glassStroke}`, flexShrink: 0,
              WebkitAppRegion: "drag",
            }}>
              <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                <Orb size={18} mode="active" intensity={0.6} />
                <span style={{ fontFamily: V.mono, fontSize: 12, fontWeight: 700, letterSpacing: 4, color: V.textSecondary }}>ZEUS</span>
                <span style={{ fontSize: 12, color: V.textTertiary }}>•</span>
                <span style={{ fontSize: 12, color: V.textTertiary, letterSpacing: 0.5 }}>{tab.charAt(0).toUpperCase() + tab.slice(1)}</span>
              </div>
            </div>

            {/* Content area */}
            <div style={{ flex: 1, padding: 24, overflow: "hidden" }}>
              {views[tab]}
            </div>
          </div>

          {/* Tab ornament — positioned outside the glass window */}
          <TabOrnament active={tab} onChange={setTab} />
        </div>
      </div>
    </>
  );
}
