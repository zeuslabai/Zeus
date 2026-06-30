import { useState, useEffect, useRef } from "react";

/* ═══ PALETTE — matches existing TUI ═══ */
const C = {
  bg: "#0a0a0f", bg2: "#12100e", bg3: "#1a1610",
  fg: "#d4cfc8", dim: "#5a5650", muted: "#3a3632", dark: "#2a2420",
  accent: "#ff3c14", accentDim: "#a0301a", accentBright: "#ff6842",
  accentFaint: "#401008", accentSoft: "rgba(255,60,20,0.05)",
  green: "#22c55e", greenDim: "#1a4a2e",
  yellow: "#eab308", yellowDim: "#6b5a10",
  blue: "#3b82f6", blueDim: "#1a2a4a",
  cyan: "#06b6d4", cyanDim: "#164e63",
  red: "#ef4444", redDim: "#4a1a1a",
  amber: "#ffa050", amberDim: "#5a3010",
  purple: "#a855f7", purpleDim: "#3a1a4a",
  white: "#f0ece6",
};

/* ═══ TABS ═══ */
const PRIMARY_TABS = [
  { id: "chat", name: "chat", glyph: "▸" },
  { id: "office", name: "office", glyph: "◇" },
  { id: "pantheon", name: "pantheon", glyph: "◈" },
  { id: "tools", name: "tools", glyph: "⚙" },
  { id: "memory", name: "memory", glyph: "▤" },
  { id: "channels", name: "channels", glyph: "⇌" },
  { id: "wallet", name: "wallet", glyph: "⊟" },
  { id: "approvals", name: "approvals", glyph: "✓" },
  { id: "settings", name: "settings", glyph: "⊕" },
  { id: "advanced", name: "more…", glyph: "▸▸" },
];

const ADVANCED_TABS = [
  { id: "agents", name: "Agents", desc: "Local + fleet roster, personas, bindings", glyph: "AGT", color: C.accent },
  { id: "skills", name: "Skills", desc: "Marketplace, install/enable, SKILL.md", glyph: "SKL", color: C.amber },
  { id: "mcp", name: "MCP Servers", desc: "Connected servers, tools, health", glyph: "MCP", color: C.cyan },
  { id: "projects", name: "Projects", desc: "Create, assign agents, status", glyph: "PRJ", color: C.green },
  { id: "canvas", name: "Canvas", desc: "Visual plan / workflow builder", glyph: "CNV", color: C.purple },
  { id: "voice", name: "Voice", desc: "Calls, STT/TTS config, recordings", glyph: "VCE", color: C.blue },
  { id: "nodecomms", name: "NodeComms", desc: "Inter-agent fleet messaging", glyph: "NCM", color: C.cyan },
  { id: "vectorstores", name: "VectorStores", desc: "Mnemosyne collections, semantic search", glyph: "VEC", color: C.amber },
  { id: "economy", name: "Economy", desc: "Agora wallet, marketplace, x402", glyph: "ECN", color: C.green },
  { id: "extensions", name: "Extensions", desc: "Deno/MCP extensions, runtime", glyph: "EXT", color: C.purple },
  { id: "knowledge-graph", name: "Knowledge Graph", desc: "Memory graph, communities", glyph: "GRA", color: C.blue },
  { id: "spawner", name: "Spawner", desc: "Active subagents, kill, logs", glyph: "SPN", color: C.accent },
  { id: "deploy", name: "Deploy / Daemon", desc: "Health, restart, launchd logs", glyph: "DPL", color: C.red },
];

/* ═══ TOP STATUS BAR ═══ */
const TopBar = ({ ctxPercent, hostname, model, gatewayVersion, connState }) => {
  const ctxColor = ctxPercent < 60 ? C.green : ctxPercent < 80 ? C.amber : C.red;
  const connColor = connState === "connected" ? C.green : connState === "reconnecting" ? C.amber : C.red;
  const bars = 10;
  const filled = Math.floor((ctxPercent / 100) * bars);

  return (
    <div style={{
      height: 22, background: C.bg2, borderBottom: `1px solid ${C.muted}`,
      display: "flex", alignItems: "center", padding: "0 10px", gap: 6,
      flexShrink: 0, fontSize: 9,
    }}>
      <span style={{ color: C.accent, fontWeight: 700, letterSpacing: 3 }}>ZEUS</span>
      <span style={{ color: C.muted }}>│</span>
      <span style={{ color: C.dim }}>{hostname}</span>
      <span style={{ color: C.muted }}>│</span>
      <span style={{ color: connColor, fontSize: 8 }}>●</span>
      <span style={{ color: connState === "connected" ? C.green : connColor }}>{connState}</span>
      <span style={{ color: C.muted }}>│</span>
      <span style={{ color: C.fg }}>{model}</span>
      <span style={{ color: C.muted }}>│</span>
      <span style={{ color: C.dim }}>v{gatewayVersion}</span>
      <span style={{ color: C.muted }}>│</span>
      <span style={{ color: C.dim }}>ctx</span>
      <span style={{ fontFamily: "inherit", letterSpacing: 0, color: ctxColor }}>
        [{Array.from({ length: bars }).map((_, i) => i < filled ? "▓" : "░").join("")}]
      </span>
      <span style={{ color: ctxColor }}>{ctxPercent}%</span>
      {ctxPercent > 80 && <span style={{ color: C.amber, marginLeft: 4 }}>⚠ near limit · /compact</span>}
      <span style={{ flex: 1 }} />
      <span style={{ color: C.dim }}>Ctrl+K palette</span>
      <span style={{ color: C.muted }}>│</span>
      <span style={{ color: C.dim }}>Ctrl+C quit</span>
    </div>
  );
};

/* ═══ TAB BAR ═══ */
const TabBar = ({ active, setActive, unreadByTab, pendingApprovals }) => (
  <div style={{
    height: 26, background: C.bg2, borderBottom: `1px solid ${C.muted}`,
    display: "flex", alignItems: "center", padding: "0 10px", gap: 0,
    flexShrink: 0, fontSize: 11,
  }}>
    {PRIMARY_TABS.map((t, i) => {
      const isActive = active === t.id || (active.startsWith("adv:") && t.id === "advanced");
      const unread = unreadByTab[t.id] || 0;
      const showApprovalBadge = t.id === "approvals" && pendingApprovals > 0;
      return (
        <div key={t.id} onClick={() => setActive(t.id)} style={{
          padding: "0 12px", height: 26,
          display: "flex", alignItems: "center", gap: 6,
          cursor: "pointer",
          borderBottom: `2px solid ${isActive ? C.accent : "transparent"}`,
          color: isActive ? C.fg : C.dim,
          background: isActive ? C.bg3 : "transparent",
        }}>
          <span style={{ color: isActive ? C.accent : C.muted, fontSize: 9 }}>{t.glyph}</span>
          <span style={{ fontWeight: isActive ? 700 : 400 }}>{t.name}</span>
          {unread > 0 && !isActive && (
            <span style={{
              padding: "1px 5px", fontSize: 8, fontWeight: 700,
              background: C.accent, color: C.bg, borderRadius: 8, minWidth: 14, textAlign: "center",
            }}>{unread}</span>
          )}
          {showApprovalBadge && (
            <span style={{
              padding: "1px 5px", fontSize: 8, fontWeight: 700,
              background: C.amber, color: C.bg, borderRadius: 8, minWidth: 14, textAlign: "center",
            }}>{pendingApprovals}</span>
          )}
        </div>
      );
    })}
    <span style={{ flex: 1 }} />
    <span style={{ color: C.muted, fontSize: 9 }}>Tab to switch  ·  ⇧Tab back  ·  : palette</span>
  </div>
);

/* ═══ BOTTOM HINT BAR ═══ */
const HintBar = ({ hints, status, queueCount }) => (
  <div style={{
    height: 22, background: C.bg2, borderTop: `1px solid ${C.muted}`,
    display: "flex", alignItems: "center", padding: "0 10px", gap: 12,
    flexShrink: 0, fontSize: 9,
  }}>
    {hints.map((h, i) => (
      <span key={i}>
        <span style={{ color: C.accentDim, fontWeight: 700 }}>{h.k}</span>{" "}
        <span style={{ color: C.dim }}>{h.v}</span>
      </span>
    ))}
    <span style={{ flex: 1 }} />
    {queueCount > 0 && (
      <>
        <span style={{ color: C.amber, fontSize: 8 }}>●</span>
        <span style={{ color: C.amber }}>{queueCount} queued</span>
        <span style={{ color: C.muted }}>│</span>
      </>
    )}
    <span style={{ color: C.dim }}>{status}</span>
  </div>
);

/* ═══════════════════════════════════════════════════ */
/* TAB 1 — CHAT                                         */
/* ═══════════════════════════════════════════════════ */
const ChatTab = ({ messages, queue, input, setInput, isStreaming, cookingIter, cookingTools, totalTools, expandedMsg, setExpandedMsg, onSubmit, onCancelLast, slashOpen, setSlashOpen }) => {
  const scrollRef = useRef(null);
  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [messages.length, isStreaming]);

  const slashCommands = [
    { cmd: "/help", desc: "Show all commands" },
    { cmd: "/clear", desc: "Clear chat history" },
    { cmd: "/compact", desc: "Compact conversation context" },
    { cmd: "/spawn", desc: "Spawn a subagent" },
    { cmd: "/stop", desc: "Stop current cooking loop" },
    { cmd: "/reset", desc: "Reset session state" },
    { cmd: "/model", desc: "Switch model mid-conversation" },
  ];

  const filteredSlash = input.startsWith("/")
    ? slashCommands.filter(s => s.cmd.startsWith(input.toLowerCase()))
    : [];

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
      {/* Messages scroll area */}
      <div ref={scrollRef} style={{ flex: 1, overflowY: "auto", padding: "8px 14px", display: "flex", flexDirection: "column", gap: 8 }}>
        {messages.map((m, i) => (
          <ChatMessage key={i} msg={m} expanded={expandedMsg === i} onExpand={() => setExpandedMsg(expandedMsg === i ? null : i)} />
        ))}
        {isStreaming && (
          <div style={{ padding: "6px 0 4px", display: "flex", alignItems: "center", gap: 12, fontSize: 10 }}>
            <ZeusFace state={cookingTools > 0 && cookingIter > 0 ? "working" : "thinking"} label="cooking" />
            <span style={{ color: C.muted }}>│</span>
            <span style={{ color: C.accent, fontWeight: 700 }}>iter {cookingIter}/8</span>
            <span style={{ color: C.dim }}>·</span>
            <span style={{ color: C.fg }}>{cookingTools} tools</span>
            <span style={{ color: C.dim }}>·</span>
            <span style={{ color: C.amber, fontStyle: "italic" }}>thinking<AnimatedDots /></span>
          </div>
        )}
      </div>

      {/* Queue indicator */}
      {queue.length > 0 && (
        <div style={{
          padding: "4px 14px", background: C.amberDim, borderTop: `1px solid ${C.amber}`,
          fontSize: 10, color: C.amber, display: "flex", alignItems: "center", gap: 8,
        }}>
          <span>📥</span>
          <span style={{ fontWeight: 700 }}>Queued: {queue.length} message{queue.length !== 1 ? "s" : ""}</span>
          <span style={{ color: C.dim, fontSize: 9 }}>— will fire as turns complete</span>
          <span style={{ flex: 1 }} />
          <span style={{ color: C.dim, fontSize: 9 }}><span style={{ color: C.amber, fontWeight: 700 }}>Esc</span> cancel last  ·  <span style={{ color: C.amber, fontWeight: 700 }}>Ctrl+Esc</span> clear all</span>
        </div>
      )}

      {/* Slash command palette overlay */}
      {filteredSlash.length > 0 && input.startsWith("/") && (
        <div style={{
          padding: "6px 14px", background: C.bg3, borderTop: `1px solid ${C.accentDim}`,
          maxHeight: 140, overflowY: "auto",
        }}>
          <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 4 }}>SLASH COMMANDS</div>
          {filteredSlash.map((s, i) => (
            <div key={i} style={{ display: "flex", padding: "2px 0", fontSize: 11 }}>
              <span style={{ color: C.accent, fontWeight: 700, width: 110 }}>{s.cmd}</span>
              <span style={{ color: C.dim }}>{s.desc}</span>
            </div>
          ))}
        </div>
      )}

      {/* Input bar */}
      <div style={{
        padding: "8px 14px 10px", background: C.bg2, borderTop: `1px solid ${C.muted}`,
        display: "flex", alignItems: "center", gap: 10,
      }}>
        <ZeusFace
          state={
            isStreaming ? "working" :
            queue.length > 0 ? "queued" :
            input.length > 0 ? "listening" :
            "ready"
          }
          small
          speed={isStreaming ? 200 : input.length > 0 ? 350 : 600}
        />
        <span style={{ color: C.muted, fontSize: 11 }}>│</span>
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              onSubmit();
            }
          }}
          placeholder={isStreaming ? "type to queue (input never blocks)…" : "message…"}
          style={{
            flex: 1, background: "transparent", border: "none",
            color: C.fg, fontFamily: "inherit", fontSize: 12,
            padding: "4px 0", outline: "none",
          }}
        />
        <span style={{ color: C.muted, fontSize: 9 }}>{input.length}/4096</span>
        <span style={{ color: C.dim, fontSize: 9 }}>↵</span>
      </div>
    </div>
  );
};

const AnimatedDots = () => {
  const [n, setN] = useState(0);
  useEffect(() => {
    const t = setInterval(() => setN(x => (x + 1) % 4), 400);
    return () => clearInterval(t);
  }, []);
  return <span>{".".repeat(n)}</span>;
};

/* ═══ ZEUS FACE ═══
   Animated ASCII face that expresses agent state.
   States: ready, thinking, working, success, error, queued, listening, sleeping
*/
const FACE_FRAMES = {
  // idle, ready for input — calm blink cycle
  ready: [
    "(◉‿◉)", "(◉‿◉)", "(◉‿◉)", "(◉‿◉)",
    "(-‿-)", "(◉‿◉)", "(◉‿◉)", "(◉‿◉)",
  ],
  // thinking — looks up, around, pondering
  thinking: [
    "(◉.◉)", "(◔.◉)", "(◔.◔)", "(◉.◔)",
    "(◉.◉)", "(◔ ◔)", "(- -)", "(◉.◉)",
  ],
  // working / cooking — busy, eyes scanning
  working: [
    "(◣_◢)", "(◢_◣)", "(◣_◢)", "(◢_◣)",
    "(▰_▰)", "(◣_◢)", "(◢_◣)", "(▰_▰)",
  ],
  // tool call running — focused, intense
  tool: [
    "[◉_◉]", "[◉.◉]", "[◉_◉]", "[◉.◉]",
    "[●_●]", "[◉_◉]", "[◉.◉]", "[◉_◉]",
  ],
  // success / completed — happy, satisfied
  success: [
    "(◉‿◉)✓", "(^‿^)✓", "(◉‿◉)✓", "(^‿^)✓",
  ],
  // error — concerned
  error: [
    "(✕_✕)", "(✕.✕)", "(✕_✕)", "(>_<)",
    "(✕_✕)", "(>_<)", "(✕_✕)", "(✕.✕)",
  ],
  // awaiting approval — alert, watching
  alert: [
    "(◉ω◉)!", "(◉ω◉)!", "(◉_◉)!", "(◉ω◉)!",
  ],
  // queue building — eyes moving like reading
  queued: [
    "(◔‿◉)", "(◉‿◔)", "(◔‿◉)", "(◉‿◔)",
  ],
  // listening / typing — attentive
  listening: [
    "(◉_◉)", "(◉_◉)", "(-_◉)", "(◉_◉)",
  ],
  // sleeping / idle long — z's
  sleeping: [
    "(-_-) z", "(-_-) zZ", "(-_-) zZz", "(-_-) zZ",
  ],
};

const FACE_COLORS = {
  ready: C.accent,
  thinking: C.amber,
  working: C.amber,
  tool: C.cyan,
  success: C.green,
  error: C.red,
  alert: C.yellow,
  queued: C.amber,
  listening: C.cyan,
  sleeping: C.dim,
};

const ZeusFace = ({ state = "ready", small = false, label = null, speed = 250 }) => {
  const [frame, setFrame] = useState(0);
  const frames = FACE_FRAMES[state] || FACE_FRAMES.ready;
  const color = FACE_COLORS[state] || C.accent;

  useEffect(() => {
    setFrame(0);
    const t = setInterval(() => setFrame(f => (f + 1) % frames.length), speed);
    return () => clearInterval(t);
  }, [state, frames.length, speed]);

  return (
    <span style={{
      display: "inline-flex", alignItems: "center", gap: 6,
      fontFamily: "inherit", fontSize: small ? 10 : 13,
      color, fontWeight: 700, letterSpacing: 0,
      whiteSpace: "pre",
      textShadow: `0 0 8px ${color}55`,
    }}>
      <span style={{ minWidth: small ? 50 : 64, display: "inline-block" }}>{frames[frame]}</span>
      {label && (
        <span style={{ fontSize: small ? 9 : 10, color: color, fontWeight: 600, fontStyle: "italic", opacity: 0.85 }}>
          {label}
        </span>
      )}
    </span>
  );
};

const ChatMessage = ({ msg, expanded, onExpand }) => {
  if (msg.role === "user") {
    return (
      <div style={{ display: "flex", gap: 8 }}>
        <span style={{ color: C.cyan, fontWeight: 700, fontSize: 10, width: 60, flexShrink: 0 }}>▸ user</span>
        <div style={{ flex: 1, color: C.fg, fontSize: 12 }}>
          {msg.text}
          {msg.channel_source && (
            <span style={{ marginLeft: 8, padding: "1px 6px", fontSize: 8, fontWeight: 700, letterSpacing: 1, background: C.bg2, color: C.dim, border: `1px solid ${C.muted}` }}>
              ↰ {msg.channel_source}
            </span>
          )}
        </div>
      </div>
    );
  }
  if (msg.role === "assistant") {
    return (
      <div style={{ display: "flex", gap: 8 }}>
        <span style={{ color: C.accent, fontWeight: 700, fontSize: 10, width: 60, flexShrink: 0, fontFamily: "inherit" }}>(◉‿◉) zeus</span>
        <div style={{ flex: 1, color: C.fg, fontSize: 12 }}>
          {msg.text}
          {msg.provider_badge && (
            <span style={{ marginLeft: 8, padding: "1px 5px", fontSize: 7, fontWeight: 700, letterSpacing: 1, background: C.bg2, color: C.dim, border: `1px solid ${C.muted}` }}>
              {msg.provider_badge}
            </span>
          )}
        </div>
      </div>
    );
  }
  if (msg.role === "tool_call") {
    const stateColor = msg.status === "running" ? C.amber : msg.status === "success" ? C.green : msg.status === "failed" ? C.red : msg.status === "awaiting_approval" ? C.yellow : C.dim;
    const stateGlyph = msg.status === "running" ? "▸" : msg.status === "success" ? "✓" : msg.status === "failed" ? "✕" : msg.status === "awaiting_approval" ? "⚠" : "○";
    const lines = msg.output ? msg.output.split("\n") : [];
    const showLines = expanded ? lines.length : Math.min(5, lines.length);
    const truncated = lines.length > 5 && !expanded;

    return (
      <div style={{ background: C.bg2, border: `1px solid ${C.muted}`, borderLeft: `2px solid ${stateColor}`, padding: "6px 10px", fontSize: 11, fontFamily: "inherit" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 2 }}>
          <span style={{ color: C.amber }}>⚙</span>
          <span style={{ color: C.amber, fontWeight: 700 }}>tool_call</span>
          <span style={{ color: C.dim }}>·</span>
          <span style={{ color: C.accentBright, fontWeight: 700 }}>{msg.tool}</span>
          <span style={{ color: C.dim }}>{msg.args}</span>
          <span style={{ flex: 1 }} />
          <span style={{ color: stateColor, fontSize: 10, fontWeight: 700 }}>{stateGlyph} {msg.status}{msg.status === "running" ? <AnimatedDots /> : ""}</span>
        </div>
        {msg.output && (
          <div style={{ marginTop: 4, paddingLeft: 14 }}>
            <div style={{ color: C.muted, fontSize: 9 }}>↳ {lines.length} line{lines.length !== 1 ? "s" : ""} returned{truncated ? ` · expand: e` : ""}</div>
            <div style={{
              marginTop: 2, padding: "4px 8px",
              background: C.bg, border: `1px solid ${C.muted}`,
              fontSize: 10, color: C.dim, whiteSpace: "pre",
              maxHeight: expanded ? 400 : "none", overflowY: expanded ? "auto" : "visible",
            }}>
              {lines.slice(0, showLines).join("\n")}
              {truncated && (
                <div onClick={onExpand} style={{ color: C.accent, marginTop: 2, cursor: "pointer", fontWeight: 700 }}>
                  ▾ {lines.length - 5} more lines (press e or click to expand)
                </div>
              )}
            </div>
          </div>
        )}
        {msg.error && (
          <div style={{ marginTop: 4, paddingLeft: 14, color: C.red, fontSize: 10 }}>
            ✕ {msg.error}
          </div>
        )}
      </div>
    );
  }
  return null;
};

/* ═══════════════════════════════════════════════════ */
/* TAB 2 — OFFICE (pixel art)                           */
/* ═══════════════════════════════════════════════════ */
const OfficeTab = ({ focusedAgent, setFocusedAgent }) => {
  // Agents with positions in zones
  const agents = [
    { id: "hermes", name: "Hermes", zone: "Engineering", x: 6, y: 4, status: "coding", task: "implementing onboarding wizard", trust: 95, color: C.accent, channel: "local" },
    { id: "hephaestus", name: "Hephaestus", zone: "Engineering", x: 12, y: 5, status: "reviewing PR", task: "reviewing #2847", trust: 89, color: C.accent, channel: "local" },
    { id: "atlas", name: "Atlas", zone: "Engineering", x: 9, y: 7, status: "running tests", task: "cargo test --workspace", trust: 92, color: C.accent, channel: "discord" },
    { id: "aegis", name: "Aegis", zone: "Comms", x: 26, y: 5, status: "monitoring CI", task: "watching pipelines", trust: 91, color: C.green, channel: "discord" },
    { id: "prometheus", name: "Prometheus", zone: "Comms", x: 32, y: 4, status: "idle", task: "—", trust: 78, color: C.cyan, channel: "discord" },
    { id: "calliope", name: "Calliope", zone: "Research", x: 50, y: 4, status: "browsing", task: "competitor analysis", trust: 85, color: C.amber, channel: "discord" },
    { id: "argus", name: "Argus", zone: "Research", x: 56, y: 6, status: "thinking", task: "synthesizing notes", trust: 88, color: C.purple, channel: "discord" },
    { id: "hestia", name: "Hestia", zone: "Break", x: 18, y: 14, status: "idle", task: "—", trust: 82, color: C.dim, channel: "discord" },
  ];

  const zones = [
    { id: "engineering", name: "ENGINEERING", x: 2, y: 2, w: 18, h: 8, color: C.accent },
    { id: "comms", name: "COMMS", x: 22, y: 2, w: 18, h: 8, color: C.green },
    { id: "research", name: "RESEARCH", x: 42, y: 2, w: 18, h: 8, color: C.amber },
    { id: "break", name: "BREAK ROOM", x: 2, y: 12, w: 28, h: 6, color: C.cyan },
    { id: "kitchen", name: "KITCHEN", x: 32, y: 12, w: 28, h: 6, color: C.purple },
  ];

  const selected = agents.find(a => a.id === focusedAgent);

  return (
    <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
      {/* Pixel canvas */}
      <div style={{ flex: 1, background: C.bg, padding: "10px", overflow: "auto" }}>
        <div style={{
          position: "relative",
          display: "grid",
          gridTemplateColumns: "repeat(64, 14px)",
          gridTemplateRows: "repeat(20, 14px)",
          fontSize: 9,
          fontFamily: "inherit",
          color: C.dim,
        }}>
          {/* Floor pattern */}
          {Array.from({ length: 64 * 20 }).map((_, i) => {
            const x = i % 64, y = Math.floor(i / 64);
            const isDot = (x + y) % 4 === 0;
            return (
              <div key={i} style={{
                width: 14, height: 14,
                gridColumn: x + 1, gridRow: y + 1,
                color: C.muted,
                fontSize: 6,
                textAlign: "center",
                lineHeight: "14px",
              }}>{isDot ? "·" : ""}</div>
            );
          })}

          {/* Zones */}
          {zones.map(z => (
            <div key={z.id} style={{
              gridColumn: `${z.x + 1} / span ${z.w}`,
              gridRow: `${z.y + 1} / span ${z.h}`,
              border: `1px dashed ${z.color}`,
              opacity: 0.5,
              position: "relative",
            }}>
              <div style={{
                position: "absolute", top: -7, left: 6,
                background: C.bg, padding: "0 4px",
                color: z.color, fontSize: 8, fontWeight: 700, letterSpacing: 2,
              }}>{z.name}</div>
            </div>
          ))}

          {/* Agents */}
          {agents.map(a => {
            const isFocused = focusedAgent === a.id;
            return (
              <div
                key={a.id}
                onClick={() => setFocusedAgent(a.id)}
                style={{
                  gridColumn: a.x + 1, gridRow: a.y + 1,
                  width: 14, height: 14,
                  display: "flex", alignItems: "center", justifyContent: "center",
                  cursor: "pointer",
                  position: "relative",
                  zIndex: 5,
                }}
              >
                {/* Trust glow */}
                {a.trust > 85 && (
                  <div style={{
                    position: "absolute", inset: -4,
                    background: `radial-gradient(circle, ${a.color}33 0%, transparent 70%)`,
                    borderRadius: "50%",
                    pointerEvents: "none",
                  }} />
                )}
                {/* Sprite */}
                <div style={{
                  width: 10, height: 10,
                  background: a.color,
                  border: isFocused ? `1px solid ${C.white}` : "none",
                  boxShadow: isFocused ? `0 0 6px ${a.color}` : "none",
                }} />
                {/* Speech bubble for active agents */}
                {a.status !== "idle" && Math.random() > 0.5 && (
                  <div style={{
                    position: "absolute", bottom: 14, left: 10,
                    background: C.bg2,
                    border: `1px solid ${a.color}`,
                    padding: "1px 4px",
                    fontSize: 7, color: a.color,
                    whiteSpace: "nowrap",
                    pointerEvents: "none",
                    zIndex: 10,
                  }}>{a.status}</div>
                )}
                {/* Name label */}
                <div style={{
                  position: "absolute", top: 14, left: -4,
                  fontSize: 7, color: isFocused ? C.white : a.color,
                  whiteSpace: "nowrap",
                  fontWeight: isFocused ? 700 : 400,
                  pointerEvents: "none",
                }}>{a.name}</div>
              </div>
            );
          })}
        </div>
      </div>

      {/* Sidebar (26 cols equivalent) */}
      <div style={{ width: 280, borderLeft: `1px solid ${C.muted}`, background: C.bg2, display: "flex", flexDirection: "column", overflow: "hidden" }}>
        {/* Selected agent panel */}
        {selected ? (
          <div style={{ padding: "10px 12px", borderBottom: `1px solid ${C.muted}` }}>
            <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>FOCUSED AGENT</div>
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
              <div style={{ width: 16, height: 16, background: selected.color }} />
              <span style={{ color: C.white, fontSize: 13, fontWeight: 700 }}>{selected.name}</span>
              <span style={{ color: C.dim, fontSize: 9 }}>· {selected.channel}</span>
            </div>
            <div style={{ fontSize: 10, color: C.dim, lineHeight: 1.7 }}>
              <div><span style={{ color: C.muted, fontWeight: 700, letterSpacing: 1 }}>ZONE</span>  <span style={{ color: C.fg }}>{selected.zone}</span></div>
              <div><span style={{ color: C.muted, fontWeight: 700, letterSpacing: 1 }}>STATUS</span>  <span style={{ color: selected.color }}>{selected.status}</span></div>
              <div><span style={{ color: C.muted, fontWeight: 700, letterSpacing: 1 }}>TASK</span>  <span style={{ color: C.fg }}>{selected.task}</span></div>
              <div><span style={{ color: C.muted, fontWeight: 700, letterSpacing: 1 }}>TRUST</span>  <span style={{ color: selected.trust > 85 ? C.green : C.amber }}>{selected.trust}%</span></div>
            </div>
            <div style={{ marginTop: 8, fontSize: 9, color: C.muted }}>
              <span style={{ color: C.accentDim, fontWeight: 700 }}>m</span> message  ·  <span style={{ color: C.accentDim, fontWeight: 700 }}>Esc</span> clear
            </div>
          </div>
        ) : (
          <div style={{ padding: "10px 12px", borderBottom: `1px solid ${C.muted}`, fontSize: 10, color: C.dim }}>
            <span style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3 }}>NO FOCUS</span>
            <div style={{ marginTop: 4 }}>Press <span style={{ color: C.accent, fontWeight: 700 }}>f</span> to cycle agents, or click a sprite</div>
          </div>
        )}

        {/* Agents list */}
        <div style={{ padding: "10px 12px", borderBottom: `1px solid ${C.muted}`, flex: 1, overflowY: "auto" }}>
          <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>AGENTS · {agents.length}</div>
          {agents.map(a => (
            <div key={a.id} onClick={() => setFocusedAgent(a.id)} style={{
              display: "flex", alignItems: "center", gap: 6, padding: "3px 4px", cursor: "pointer",
              background: focusedAgent === a.id ? C.bg3 : "transparent",
              borderLeft: `2px solid ${focusedAgent === a.id ? a.color : "transparent"}`,
              fontSize: 10,
            }}>
              <div style={{ width: 8, height: 8, background: a.color }} />
              <span style={{ color: focusedAgent === a.id ? C.white : C.fg, flex: 1, fontWeight: focusedAgent === a.id ? 700 : 400 }}>{a.name}</span>
              <span style={{ color: a.status === "idle" ? C.dim : C.green, fontSize: 8 }}>●</span>
              <span style={{ color: C.muted, fontSize: 8, width: 50, textAlign: "right" }}>{a.zone.slice(0, 6)}</span>
            </div>
          ))}
        </div>

        {/* Stats */}
        <div style={{ padding: "10px 12px" }}>
          <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>FLEET STATS</div>
          {[
            ["Ticks", "1,847"],
            ["Active", "6"],
            ["Idle", "2"],
            ["Errors", "0"],
            ["TPS", "8"],
          ].map(([k, v]) => (
            <div key={k} style={{ display: "flex", justifyContent: "space-between", fontSize: 10, color: C.dim, padding: "1px 0" }}>
              <span style={{ color: C.muted, fontWeight: 700, letterSpacing: 1 }}>{k}</span>
              <span style={{ color: C.fg, fontFamily: "inherit" }}>{v}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
};

/* ═══════════════════════════════════════════════════ */
/* TAB 3 — PANTHEON                                     */
/* ═══════════════════════════════════════════════════ */
const PantheonTab = ({ selectedMission, setSelectedMission }) => {
  const missions = [
    { id: "m1", name: "v0.4.7 release prep", status: "active", progress: 68, agents: 4, lead: "Hermes", started: "2h ago" },
    { id: "m2", name: "Onboarding wizard impl", status: "planning", progress: 12, agents: 2, lead: "Hephaestus", started: "30m ago" },
    { id: "m3", name: "Fleet shakedown audit", status: "reviewing", progress: 95, agents: 3, lead: "Aegis", started: "yesterday" },
    { id: "m4", name: "Q1 marketing campaign", status: "active", progress: 42, agents: 2, lead: "Calliope", started: "3 days ago" },
    { id: "m5", name: "DGX Spark integration", status: "completed", progress: 100, agents: 5, lead: "Atlas", started: "1 week ago" },
    { id: "m6", name: "Aegis hardening", status: "draft", progress: 0, agents: 0, lead: "—", started: "—" },
  ];

  const statusColors = {
    draft: C.dim, planning: C.amber, assembling: C.cyan, active: C.green, reviewing: C.purple, completed: C.dim,
  };

  const m = missions.find(x => x.id === selectedMission) || missions[0];

  // Plan card pending
  const pendingPlan = {
    title: "Phase 2 — Tools browser + memory tab",
    proposed_by: "Hephaestus",
    steps: [
      "Add Tab::Tools enum variant + render scaffolding",
      "Wire zeus-talos tool registry into tool browser",
      "Implement schema viewer for each tool",
      "Add execute-on-demand modal with arg editor",
      "Memory tab: tree view of ~/.zeus/workspace/",
      "Mnemosyne FTS+vector search inline",
    ],
    estimated_time: "~6h across 2 sessions",
  };

  // Live event stream
  const events = [
    { t: "14:32:18", agent: "Hephaestus", event: "started step 3/6", color: C.green },
    { t: "14:31:45", agent: "Hermes", event: "approved plan card #847", color: C.amber },
    { t: "14:30:12", agent: "Atlas", event: "completed cargo test (7,801 passed)", color: C.green },
    { t: "14:28:03", agent: "Hephaestus", event: "submitted PR #2847", color: C.cyan },
    { t: "14:25:51", agent: "Aegis", event: "watchdog tick · all green", color: C.dim },
    { t: "14:22:30", agent: "Hermes", event: "assigned task to Hephaestus", color: C.accent },
    { t: "14:20:08", agent: "Hephaestus", event: "joined war room #v047-prep", color: C.cyan },
  ];

  return (
    <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
      {/* Mission list */}
      <div style={{ width: 320, borderRight: `1px solid ${C.muted}`, display: "flex", flexDirection: "column" }}>
        <div style={{ padding: "8px 12px", borderBottom: `1px solid ${C.muted}`, background: C.bg2 }}>
          <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3 }}>MISSIONS · {missions.length}</div>
          <div style={{ color: C.dim, fontSize: 10, marginTop: 2 }}>Active war rooms + scheduled work</div>
        </div>
        <div style={{ flex: 1, overflowY: "auto" }}>
          {missions.map(mn => (
            <div key={mn.id} onClick={() => setSelectedMission(mn.id)} style={{
              padding: "8px 12px", cursor: "pointer",
              background: selectedMission === mn.id ? C.bg3 : "transparent",
              borderLeft: `2px solid ${selectedMission === mn.id ? statusColors[mn.status] : "transparent"}`,
              borderBottom: `1px solid ${C.muted}`,
            }}>
              <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 3 }}>
                <span style={{ color: statusColors[mn.status], fontSize: 8 }}>●</span>
                <span style={{ color: selectedMission === mn.id ? C.white : C.fg, fontSize: 11, fontWeight: 600, flex: 1 }}>{mn.name}</span>
              </div>
              <div style={{ display: "flex", justifyContent: "space-between", fontSize: 9, color: C.dim }}>
                <span style={{ color: statusColors[mn.status], fontWeight: 700, letterSpacing: 1, textTransform: "uppercase" }}>{mn.status}</span>
                <span>{mn.agents} agents · {mn.lead}</span>
              </div>
              {/* Progress bar */}
              <div style={{ marginTop: 4, height: 2, background: C.bg, position: "relative" }}>
                <div style={{ position: "absolute", top: 0, left: 0, height: "100%", width: `${mn.progress}%`, background: statusColors[mn.status] }} />
              </div>
            </div>
          ))}
        </div>
        <div style={{ padding: "6px 12px", borderTop: `1px solid ${C.muted}`, background: C.bg2, fontSize: 9, color: C.dim }}>
          <span style={{ color: C.accentDim, fontWeight: 700 }}>n</span> new mission  <span style={{ color: C.muted }}>·</span>  <span style={{ color: C.accentDim, fontWeight: 700 }}>p</span> pause  <span style={{ color: C.muted }}>·</span>  <span style={{ color: C.accentDim, fontWeight: 700 }}>c</span> cancel
        </div>
      </div>

      {/* Mission detail + war room + events */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
        {/* Mission header */}
        <div style={{ padding: "10px 16px", borderBottom: `1px solid ${C.muted}`, background: C.bg2 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 4 }}>
            <span style={{ color: statusColors[m.status], fontSize: 10 }}>●</span>
            <span style={{ color: C.white, fontSize: 16, fontWeight: 700 }}>{m.name}</span>
            <span style={{ padding: "1px 6px", fontSize: 8, fontWeight: 700, letterSpacing: 2, color: statusColors[m.status], border: `1px solid ${statusColors[m.status]}` }}>{m.status.toUpperCase()}</span>
          </div>
          <div style={{ display: "flex", gap: 16, fontSize: 10, color: C.dim }}>
            <span><span style={{ color: C.accentDim, fontWeight: 700, letterSpacing: 2 }}>LEAD</span> {m.lead}</span>
            <span><span style={{ color: C.accentDim, fontWeight: 700, letterSpacing: 2 }}>AGENTS</span> {m.agents}</span>
            <span><span style={{ color: C.accentDim, fontWeight: 700, letterSpacing: 2 }}>STARTED</span> {m.started}</span>
            <span><span style={{ color: C.accentDim, fontWeight: 700, letterSpacing: 2 }}>PROGRESS</span> <span style={{ color: statusColors[m.status] }}>{m.progress}%</span></span>
          </div>
        </div>

        <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
          {/* War room chat */}
          <div style={{ flex: 1, display: "flex", flexDirection: "column", borderRight: `1px solid ${C.muted}` }}>
            <div style={{ padding: "6px 14px", borderBottom: `1px solid ${C.muted}`, fontSize: 9, color: C.accentDim, fontWeight: 700, letterSpacing: 3 }}>WAR ROOM #{m.id}</div>

            <div style={{ flex: 1, overflowY: "auto", padding: "10px 14px", display: "flex", flexDirection: "column", gap: 6 }}>
              {[
                { agent: "Hermes", time: "14:20", text: "Starting v0.4.7 release prep. Assembling team.", color: C.accent },
                { agent: "Hephaestus", time: "14:21", text: "Copy. I'll take wizard impl + tests.", color: C.accent },
                { agent: "Atlas", time: "14:22", text: "I can run the integration suite once code lands.", color: C.accent },
                { agent: "Aegis", time: "14:23", text: "CI watcher armed. Will alert if anything regresses.", color: C.green },
                { agent: "Hephaestus", time: "14:28", text: "PR #2847 submitted. Phase 1 widgets done.", color: C.accent },
                { agent: "Atlas", time: "14:30", text: "✓ cargo test --workspace · 7,801 passed", color: C.accent },
                { agent: "Hermes", time: "14:31", text: "Approving plan card for Phase 2.", color: C.accent },
                { agent: "Hephaestus", time: "14:32", text: "On it. Step 3/6 in progress.", color: C.accent },
              ].map((msg, i) => (
                <div key={i} style={{ display: "flex", gap: 8, fontSize: 11 }}>
                  <span style={{ color: C.muted, fontSize: 9, width: 38, fontFamily: "inherit", flexShrink: 0 }}>{msg.time}</span>
                  <span style={{ color: msg.color, fontWeight: 700, width: 64, flexShrink: 0 }}>{msg.agent}</span>
                  <span style={{ color: C.fg, flex: 1 }}>{msg.text}</span>
                </div>
              ))}
            </div>

            {/* Pending plan card */}
            <div style={{ borderTop: `2px solid ${C.amber}`, background: C.amberDim, padding: "8px 14px" }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 4 }}>
                <span style={{ color: C.amber, fontSize: 11 }}>⚠</span>
                <span style={{ color: C.amber, fontSize: 9, fontWeight: 700, letterSpacing: 3 }}>PLAN CARD AWAITING APPROVAL</span>
                <span style={{ flex: 1 }} />
                <span style={{ color: C.muted, fontSize: 9 }}>by {pendingPlan.proposed_by}</span>
              </div>
              <div style={{ color: C.white, fontSize: 12, fontWeight: 700, marginBottom: 4 }}>{pendingPlan.title}</div>
              <ol style={{ margin: 0, paddingLeft: 18, color: C.fg, fontSize: 10, lineHeight: 1.6 }}>
                {pendingPlan.steps.map((s, i) => (
                  <li key={i} style={{ color: C.fg }}>{s}</li>
                ))}
              </ol>
              <div style={{ marginTop: 6, display: "flex", alignItems: "center", gap: 10 }}>
                <span style={{ color: C.dim, fontSize: 9, fontStyle: "italic" }}>{pendingPlan.estimated_time}</span>
                <span style={{ flex: 1 }} />
                <button style={{ background: C.green, color: C.bg, border: "none", padding: "3px 12px", fontSize: 9, fontWeight: 700, letterSpacing: 2, fontFamily: "inherit", cursor: "pointer" }}>a APPROVE</button>
                <button style={{ background: "transparent", color: C.amber, border: `1px solid ${C.amber}`, padding: "3px 12px", fontSize: 9, fontWeight: 700, letterSpacing: 2, fontFamily: "inherit", cursor: "pointer" }}>r REDIRECT</button>
                <button style={{ background: "transparent", color: C.red, border: `1px solid ${C.red}`, padding: "3px 12px", fontSize: 9, fontWeight: 700, letterSpacing: 2, fontFamily: "inherit", cursor: "pointer" }}>R REJECT</button>
              </div>
            </div>
          </div>

          {/* Live event feed */}
          <div style={{ width: 280, display: "flex", flexDirection: "column", background: C.bg }}>
            <div style={{ padding: "6px 12px", borderBottom: `1px solid ${C.muted}`, background: C.bg2, fontSize: 9, color: C.accentDim, fontWeight: 700, letterSpacing: 3 }}>LIVE EVENTS</div>
            <div style={{ flex: 1, overflowY: "auto", padding: "8px 12px" }}>
              {events.map((e, i) => (
                <div key={i} style={{ display: "flex", gap: 6, fontSize: 9, padding: "3px 0", borderBottom: `1px solid ${C.muted}` }}>
                  <span style={{ color: C.muted, fontFamily: "inherit", flexShrink: 0 }}>{e.t}</span>
                  <span style={{ color: e.color, fontWeight: 700, flexShrink: 0 }}>{e.agent}</span>
                  <span style={{ color: C.dim, flex: 1, lineHeight: 1.4 }}>{e.event}</span>
                </div>
              ))}
            </div>
            <div style={{ padding: "6px 12px", borderTop: `1px solid ${C.muted}`, background: C.bg2, fontSize: 8, color: C.muted, display: "flex", alignItems: "center", gap: 6 }}>
              <span style={{ color: C.green, fontSize: 8 }}>●</span>
              SSE stream connected · /v1/pantheon/rooms/{m.id}/stream
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

/* ═══════════════════════════════════════════════════ */
/* TAB 4 — TOOLS                                        */
/* ═══════════════════════════════════════════════════ */
const ToolsTab = ({ selectedTool, setSelectedTool, toolFilter, setToolFilter }) => {
  const categories = [
    { id: "core", name: "Core", count: 8, color: C.accent },
    { id: "talos", name: "Talos · macOS", count: 193, color: C.amber },
    { id: "browser", name: "Browser CDP", count: 11, color: C.blue },
    { id: "git", name: "Git", count: 14, color: C.green },
    { id: "files", name: "Files", count: 22, color: C.cyan },
    { id: "shell", name: "Shell", count: 4, color: C.red },
    { id: "memory", name: "Memory", count: 18, color: C.purple },
    { id: "channels", name: "Channels", count: 32, color: C.green },
    { id: "media", name: "Media gen", count: 12, color: C.amber },
    { id: "mcp", name: "MCP", count: 51, color: C.cyan },
  ];

  const tools = [
    { name: "shell", category: "shell", desc: "Execute shell command (sandboxed)", danger: true, schema: '{"command": "string", "cwd?": "string"}' },
    { name: "read_file", category: "files", desc: "Read file contents", schema: '{"path": "string", "offset?": "int", "limit?": "int"}' },
    { name: "write_file", category: "files", desc: "Write or create file", schema: '{"path": "string", "content": "string"}' },
    { name: "apply_patch", category: "files", desc: "Apply unified diff patch", schema: '{"patch": "string"}' },
    { name: "web_fetch", category: "core", desc: "Fetch URL content (allowlisted)", schema: '{"url": "string"}' },
    { name: "git_status", category: "git", desc: "Show working tree status", schema: '{}' },
    { name: "git_commit", category: "git", desc: "Commit staged changes", schema: '{"message": "string", "amend?": "bool"}' },
    { name: "applescript_calendar_create", category: "talos", desc: "Create Calendar event via AppleScript", schema: '{"title": "string", "start": "datetime", "end": "datetime"}' },
    { name: "browser_navigate", category: "browser", desc: "Navigate Chrome to URL", schema: '{"url": "string"}' },
    { name: "browser_click", category: "browser", desc: "Click element by selector", schema: '{"selector": "string"}' },
    { name: "memory_recall", category: "memory", desc: "Mnemosyne hybrid search", schema: '{"query": "string", "limit?": "int"}' },
    { name: "memory_store", category: "memory", desc: "Store fact in Mnemosyne", schema: '{"content": "string", "tags?": "[string]"}' },
    { name: "discord_send", category: "channels", desc: "Send Discord message", schema: '{"channel_id": "string", "content": "string"}' },
    { name: "image_generate", category: "media", desc: "Generate image via configured provider", schema: '{"prompt": "string", "size?": "string"}' },
  ];

  const filtered = tools.filter(t =>
    !toolFilter || t.name.includes(toolFilter.toLowerCase()) || t.desc.toLowerCase().includes(toolFilter.toLowerCase())
  );

  const sel = tools.find(t => t.name === selectedTool) || tools[0];

  return (
    <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
      {/* Categories */}
      <div style={{ width: 200, borderRight: `1px solid ${C.muted}`, display: "flex", flexDirection: "column", background: C.bg2 }}>
        <div style={{ padding: "8px 12px", borderBottom: `1px solid ${C.muted}` }}>
          <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3 }}>CATEGORIES</div>
          <div style={{ color: C.fg, fontSize: 11, fontWeight: 700, marginTop: 2 }}>365 tools</div>
        </div>
        <div style={{ flex: 1, overflowY: "auto" }}>
          {categories.map(c => (
            <div key={c.id} style={{
              display: "flex", alignItems: "center", gap: 8, padding: "5px 12px",
              cursor: "pointer", borderBottom: `1px solid ${C.muted}`,
              borderLeft: `2px solid transparent`,
            }}>
              <span style={{ width: 6, height: 6, background: c.color }} />
              <span style={{ color: C.fg, fontSize: 10, flex: 1 }}>{c.name}</span>
              <span style={{ color: C.dim, fontSize: 9, fontFamily: "inherit" }}>{c.count}</span>
            </div>
          ))}
        </div>
      </div>

      {/* Tool list */}
      <div style={{ width: 360, borderRight: `1px solid ${C.muted}`, display: "flex", flexDirection: "column" }}>
        <div style={{ padding: "8px 12px", borderBottom: `1px solid ${C.muted}`, display: "flex", alignItems: "center", gap: 8, background: C.bg2 }}>
          <span style={{ color: C.dim, fontSize: 11 }}>/</span>
          <input
            value={toolFilter}
            onChange={(e) => setToolFilter(e.target.value)}
            placeholder="filter tools…"
            style={{ flex: 1, background: "transparent", border: "none", color: C.fg, fontFamily: "inherit", fontSize: 10, outline: "none" }}
          />
          <span style={{ color: C.muted, fontSize: 9 }}>{filtered.length}</span>
        </div>
        <div style={{ flex: 1, overflowY: "auto" }}>
          {filtered.map(t => (
            <div key={t.name} onClick={() => setSelectedTool(t.name)} style={{
              padding: "6px 12px", cursor: "pointer",
              background: selectedTool === t.name ? C.bg3 : "transparent",
              borderLeft: `2px solid ${selectedTool === t.name ? C.accent : "transparent"}`,
              borderBottom: `1px solid ${C.muted}`,
            }}>
              <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <span style={{ color: t.danger ? C.red : C.amber, fontSize: 9 }}>⚙</span>
                <span style={{ color: selectedTool === t.name ? C.white : C.fg, fontSize: 11, fontWeight: 600, flex: 1 }}>{t.name}</span>
                {t.danger && <span style={{ color: C.red, fontSize: 7, fontWeight: 700, letterSpacing: 1 }}>● SANDBOXED</span>}
              </div>
              <div style={{ color: C.dim, fontSize: 9, marginTop: 1, paddingLeft: 14 }}>{t.desc}</div>
              <div style={{ color: C.muted, fontSize: 8, marginTop: 1, paddingLeft: 14, letterSpacing: 1 }}>{t.category}</div>
            </div>
          ))}
        </div>
      </div>

      {/* Detail + executor */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "auto" }}>
        <div style={{ padding: "12px 16px", borderBottom: `1px solid ${C.muted}` }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 4 }}>
            <span style={{ color: sel.danger ? C.red : C.amber, fontSize: 14 }}>⚙</span>
            <span style={{ color: C.white, fontSize: 16, fontWeight: 700 }}>{sel.name}</span>
            {sel.danger && <span style={{ padding: "1px 6px", fontSize: 8, fontWeight: 700, letterSpacing: 2, color: C.red, border: `1px solid ${C.red}` }}>SANDBOXED</span>}
          </div>
          <div style={{ color: C.dim, fontSize: 11 }}>{sel.desc}</div>
          <div style={{ color: C.muted, fontSize: 9, marginTop: 4 }}>category · <span style={{ color: C.fg }}>{sel.category}</span></div>
        </div>

        {/* Schema */}
        <div style={{ padding: "12px 16px", borderBottom: `1px solid ${C.muted}` }}>
          <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>SCHEMA</div>
          <pre style={{ background: C.bg, padding: "8px 12px", border: `1px solid ${C.muted}`, fontSize: 10, color: C.cyan, margin: 0, fontFamily: "inherit" }}>{sel.schema}</pre>
        </div>

        {/* Execute */}
        <div style={{ padding: "12px 16px" }}>
          <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>EXECUTE</div>
          <textarea
            placeholder={`{\n  "path": "/etc/hosts"\n}`}
            style={{
              width: "100%", height: 80, background: C.bg, border: `1px solid ${C.muted}`,
              color: C.fg, fontFamily: "inherit", fontSize: 10, padding: "8px 12px",
              outline: "none", resize: "vertical",
            }}
          />
          <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
            <button style={{ background: C.accent, color: C.bg, border: "none", padding: "5px 16px", fontSize: 10, fontWeight: 700, letterSpacing: 2, fontFamily: "inherit", cursor: "pointer" }}>▸ EXECUTE</button>
            <button style={{ background: "transparent", color: C.dim, border: `1px solid ${C.muted}`, padding: "5px 12px", fontSize: 10, fontWeight: 700, letterSpacing: 2, fontFamily: "inherit", cursor: "pointer" }}>VALIDATE</button>
            <span style={{ flex: 1 }} />
            <span style={{ color: C.muted, fontSize: 9, alignSelf: "center" }}>last run · 14:32 · ✓ 24 lines</span>
          </div>
        </div>
      </div>
    </div>
  );
};

/* ═══════════════════════════════════════════════════ */
/* TAB 5 — MEMORY                                       */
/* ═══════════════════════════════════════════════════ */
const MemoryTab = () => {
  const [tab, setTab] = useState("workspace"); // workspace | sessions | mnemosyne

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
      {/* Sub-tabs */}
      <div style={{ display: "flex", borderBottom: `1px solid ${C.muted}`, background: C.bg2 }}>
        {[
          { id: "workspace", label: "Workspace", count: "847 files" },
          { id: "sessions", label: "Sessions", count: "147 sessions" },
          { id: "mnemosyne", label: "Mnemosyne", count: "12,847 facts" },
        ].map(t => (
          <div key={t.id} onClick={() => setTab(t.id)} style={{
            padding: "8px 14px", cursor: "pointer",
            borderBottom: `2px solid ${tab === t.id ? C.accent : "transparent"}`,
            color: tab === t.id ? C.fg : C.dim, fontWeight: tab === t.id ? 700 : 400,
            fontSize: 11, display: "flex", alignItems: "center", gap: 8,
          }}>
            {t.label}
            <span style={{ color: C.muted, fontSize: 9 }}>{t.count}</span>
          </div>
        ))}
      </div>

      <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
        {tab === "workspace" && (
          <>
            <div style={{ width: 320, borderRight: `1px solid ${C.muted}`, padding: "10px 0", overflowY: "auto", fontFamily: "inherit", fontSize: 10 }}>
              <div style={{ padding: "0 12px", color: C.dim, fontSize: 9, marginBottom: 6 }}>~/.zeus/workspace/</div>
              {[
                { n: "AGENTS.md", icon: "📄", color: C.accent, dirty: false },
                { n: "SOUL.md", icon: "📄", color: C.accent, dirty: true },
                { n: "USER.md", icon: "📄", color: C.fg, dirty: false },
                { n: "HEARTBEAT.md", icon: "📄", color: C.fg, dirty: false },
                { n: "├ journals/", icon: "📁", color: C.amber, dirty: false, isDir: true },
                { n: "│ ├ 2026-05-03.md", icon: "📄", color: C.fg, dirty: false, indent: 1, current: true },
                { n: "│ ├ 2026-05-02.md", icon: "📄", color: C.dim, dirty: false, indent: 1 },
                { n: "│ ├ 2026-05-01.md", icon: "📄", color: C.dim, dirty: false, indent: 1 },
                { n: "│ └ ...", icon: "", color: C.muted, dirty: false, indent: 1 },
                { n: "├ projects/", icon: "📁", color: C.amber, dirty: false, isDir: true },
                { n: "│ ├ zeus-tui-onboarding.md", icon: "📄", color: C.fg, dirty: false, indent: 1 },
                { n: "│ ├ pantheon-impl.md", icon: "📄", color: C.dim, dirty: false, indent: 1 },
                { n: "│ └ deploy-fixes.md", icon: "📄", color: C.dim, dirty: false, indent: 1 },
                { n: "├ contexts/", icon: "📁", color: C.amber, dirty: false, isDir: true },
                { n: "│ └ fleet-2026-05.md", icon: "📄", color: C.dim, dirty: false, indent: 1 },
                { n: "└ scratch.md", icon: "📄", color: C.dim, dirty: true },
              ].map((f, i) => (
                <div key={i} style={{
                  padding: "2px 12px", display: "flex", alignItems: "center", gap: 6,
                  background: f.current ? C.bg3 : "transparent",
                  borderLeft: `2px solid ${f.current ? C.accent : "transparent"}`,
                  cursor: "pointer",
                  fontFamily: "inherit",
                }}>
                  <span style={{ color: f.color, whiteSpace: "pre" }}>{f.n}</span>
                  {f.dirty && <span style={{ color: C.amber, fontSize: 9 }}>●</span>}
                </div>
              ))}
            </div>

            <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
              <div style={{ padding: "8px 16px", borderBottom: `1px solid ${C.muted}`, background: C.bg2, display: "flex", alignItems: "center", gap: 10 }}>
                <span style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3 }}>JOURNAL</span>
                <span style={{ color: C.dim, fontSize: 10 }}>2026-05-03.md</span>
                <span style={{ flex: 1 }} />
                <span style={{ color: C.muted, fontSize: 9 }}>last modified · 2 minutes ago</span>
              </div>
              <div style={{ flex: 1, padding: "12px 18px", overflowY: "auto", fontSize: 11, color: C.fg, lineHeight: 1.6 }}>
                <div style={{ color: C.accent, fontSize: 14, fontWeight: 700 }}># Journal · 2026-05-03</div>
                <div style={{ color: C.dim, fontSize: 10, marginTop: 4, marginBottom: 14 }}>Saturday · zeus.local</div>

                <div style={{ color: C.amber, fontWeight: 700, marginBottom: 4 }}>## Sessions</div>
                <div style={{ marginBottom: 12 }}>Worked through the comprehensive onboarding wizard impl PRD with merakizzz. Walked all 19 steps. Locked the feature surface. Track C (Talos gate, [images] migration, heartbeat persistence) confirmed as pre-launch blockers.</div>

                <div style={{ color: C.amber, fontWeight: 700, marginBottom: 4 }}>## Decisions</div>
                <div style={{ marginBottom: 4 }}>- Image gen routes to <span style={{ color: C.accent }}>[talos.image]</span>, not <span style={{ color: C.dim }}>[images]</span></div>
                <div style={{ marginBottom: 4 }}>- ChanConfig forms render stacked, not sequential</div>
                <div style={{ marginBottom: 12 }}>- Memory step pre-selects Ollama if detected at localhost:11434</div>

                <div style={{ color: C.amber, fontWeight: 700, marginBottom: 4 }}>## Open</div>
                <div>Production TUI prototype dispatch — 8 primary tabs + Advanced submenu. Mike requested it after onboarding wizard signoff.</div>
              </div>
            </div>
          </>
        )}

        {tab === "sessions" && (
          <div style={{ flex: 1, padding: "10px 0", overflowY: "auto" }}>
            {[
              { id: "s_2847", time: "14:30", duration: "12m", tools: 47, msgs: 23, status: "active", topic: "TUI prototype design" },
              { id: "s_2846", time: "14:00", duration: "28m", tools: 89, msgs: 41, status: "completed", topic: "Onboarding impl PRD review" },
              { id: "s_2845", time: "13:15", duration: "45m", tools: 142, msgs: 67, status: "completed", topic: "Comprehensive wizard prototype" },
              { id: "s_2844", time: "11:30", duration: "1h 12m", tools: 234, msgs: 98, status: "completed", topic: "Voice / image gen PRDs" },
              { id: "s_2843", time: "yesterday 18:45", duration: "23m", tools: 56, msgs: 34, status: "completed", topic: "Fleet shakedown audit" },
              { id: "s_2842", time: "yesterday 16:20", duration: "55m", tools: 178, msgs: 72, status: "completed", topic: "Pitch deck v5" },
            ].map(s => (
              <div key={s.id} style={{
                padding: "10px 16px", borderBottom: `1px solid ${C.muted}`,
                cursor: "pointer", display: "flex", alignItems: "center", gap: 14,
              }}>
                <span style={{ color: s.status === "active" ? C.green : C.dim, fontSize: 9 }}>●</span>
                <span style={{ color: C.fg, fontSize: 11, fontFamily: "inherit", width: 80, color: C.dim }}>{s.id}</span>
                <span style={{ color: C.muted, fontSize: 10, width: 130 }}>{s.time}</span>
                <span style={{ color: C.fg, fontSize: 11, flex: 1 }}>{s.topic}</span>
                <span style={{ color: C.dim, fontSize: 9 }}>{s.duration} · {s.tools} tools · {s.msgs} msgs</span>
              </div>
            ))}
          </div>
        )}

        {tab === "mnemosyne" && (
          <div style={{ flex: 1, display: "flex", flexDirection: "column" }}>
            <div style={{ padding: "10px 16px", borderBottom: `1px solid ${C.muted}`, background: C.bg2, display: "flex", alignItems: "center", gap: 8 }}>
              <span style={{ color: C.dim, fontSize: 11 }}>/</span>
              <input
                placeholder="hybrid search · BM25 + vector embeddings"
                style={{ flex: 1, background: "transparent", border: "none", color: C.fg, fontFamily: "inherit", fontSize: 11, outline: "none" }}
              />
              <span style={{ color: C.cyan, fontSize: 9 }}>● ollama embedded</span>
            </div>
            <div style={{ flex: 1, overflowY: "auto", padding: "10px 16px" }}>
              <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>RECENT FACTS · 12,847 indexed</div>
              {[
                { t: "Mike confirmed Track C blockers ship in Phase 0 — [talos] always-write, [images]→[talos.image], heartbeat persistence", score: 0.98, age: "2m ago", source: "session 2847" },
                { t: "Production TUI requires 8 primary tabs + Advanced submenu, not 4 — current state is regression from S61", score: 0.94, age: "8m ago", source: "session 2847" },
                { t: "ChanConfig forms must be stacked (all visible) not sequential per merakizzz directive 2026-05-03", score: 0.91, age: "30m ago", source: "session 2846" },
                { t: "Z-Image Turbo on DGX requires steps=1 — multi-step inference returns black PNG", score: 0.88, age: "1h ago", source: "session 2845" },
                { t: "Mac Studio M5 Ultra release tracked — 256GB RAM target for AI inference workloads", score: 0.85, age: "yesterday", source: "session 2843" },
              ].map((r, i) => (
                <div key={i} style={{
                  padding: "8px 10px", marginBottom: 6,
                  background: C.bg2, border: `1px solid ${C.muted}`,
                  borderLeft: `2px solid ${C.cyan}`,
                }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 4, fontSize: 9, color: C.muted }}>
                    <span style={{ color: C.green, fontWeight: 700, fontFamily: "inherit" }}>{r.score.toFixed(2)}</span>
                    <span>·</span>
                    <span>{r.source}</span>
                    <span>·</span>
                    <span>{r.age}</span>
                  </div>
                  <div style={{ color: C.fg, fontSize: 11, lineHeight: 1.5 }}>{r.t}</div>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

/* ═══════════════════════════════════════════════════ */
/* WALLET — zeus-economy + zeus-wallet                  */
/* ═══════════════════════════════════════════════════ */
const WALLET_TITANS = [
  { name: "Hermes", role: "Coordinator", addr: "zeus1q9x…h3rm", token: 48210, credit: 1250, earned: 92400, spent: 44190, color: C.accent, st: "active" },
  { name: "Hephaestus", role: "Backend/Forge", addr: "zeus1qf8…hph2", token: 31980, credit: 840, earned: 61200, spent: 29220, color: C.accent, st: "active" },
  { name: "Atlas", role: "Backend(dual)", addr: "zeus1qa2…atl7", token: 27340, credit: 610, earned: 50100, spent: 22760, color: C.accent, st: "active" },
  { name: "Aegis", role: "Security&CI", addr: "zeus1qe3…aeg1", token: 19750, credit: 1100, earned: 38400, spent: 18650, color: C.green, st: "active" },
  { name: "Calliope", role: "Marketing", addr: "zeus1qc7…cal9", token: 22410, credit: 430, earned: 41900, spent: 19490, color: C.amber, st: "active" },
  { name: "Prometheus", role: "Experimental", addr: "zeus1qp4…prm3", token: 8120, credit: 290, earned: 14600, spent: 6480, color: C.purple, st: "idle" },
  { name: "Argus", role: "Provisioner", addr: "zeus1qg5…arg8", token: 11030, credit: 180, earned: 19200, spent: 8170, color: C.cyan, st: "active" },
  { name: "Hestia", role: "WebPlatform", addr: "zeus1qh1…hst6", token: 6540, credit: 95, earned: 10800, spent: 4260, color: C.dim, st: "idle" },
];
const WALLET_ACT = [
  { k: "recv", who: "Agora→Calliope", amt: 2400, u: "ZEUS", st: "ok", t: "2m", note: "x402 content sale" },
  { k: "sent", who: "You→Hephaestus", amt: 5000, u: "ZEUS", st: "ok", t: "14m", note: "compute top-up" },
  { k: "multi", who: "Hermes→3 titans", amt: 1800, u: "ZEUS", st: "ok", t: "31m", note: "mission payout split" },
  { k: "spnd", who: "Hephaestus→Agora", amt: 499, u: "CR", st: "ok", t: "1h", note: "advanced-codegen skill" },
  { k: "recv", who: "Agora→Atlas", amt: 18, u: "ZEUS", st: "ok", t: "1h", note: "x402 search settle" },
  { k: "mint", who: "Ledger→You", amt: 10000, u: "ZEUS", st: "ok", t: "3h", note: "credit on-ramp" },
  { k: "sent", who: "You→Aegis", amt: 2500, u: "ZEUS", st: "pend", t: "3h", note: "audit retainer" },
  { k: "burn", who: "Prometheus→Ledger", amt: 40, u: "CR", st: "ok", t: "5h", note: "MiniMax inference" },
  { k: "spnd", who: "Calliope→Agora", amt: 1200, u: "ZEUS", st: "fail", t: "6h", note: "insufficient balance" },
];
const WALLET_KIND = {
  recv: { g: "▸", c: C.green, l: "RECV" }, sent: { g: "▸", c: C.accent, l: "SENT" },
  multi: { g: "⋔", c: C.cyan, l: "MULTI" }, spnd: { g: "◇", c: C.amber, l: "SPEND" },
  mint: { g: "◈", c: C.green, l: "MINT" }, burn: { g: "✕", c: C.red, l: "BURN" },
};
const WALLET_STC = { ok: C.green, pend: C.amber, fail: C.red };
const wfmt = n => n.toLocaleString("en-US");
const WALLET_ADDR = "zeus1q7m3k9x2v8p4n6t0h5r3a1c7w9e2d4f6g8b0j2";

const WalletTab = ({ walletView, setWalletView, walletSel }) => {
  const fleetTotal = WALLET_TITANS.reduce((s, t) => s + t.token, 0);
  const VIEWS = [
    { id: "balance", label: "Balance" },
    { id: "send", label: "Send" },
    { id: "receive", label: "Receive" },
    { id: "activity", label: "Activity" },
    { id: "economy", label: "Economy" },
    { id: "security", label: "Security" },
  ];

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", padding: "10px 16px", overflow: "auto" }}>
      <div style={{ display: "flex", gap: 4, marginBottom: 12 }}>
        {VIEWS.map((v, i) => (
          <div key={v.id} onClick={() => setWalletView(v.id)} style={{
            padding: "4px 14px", cursor: "pointer", fontSize: 11, letterSpacing: 1,
            background: walletView === v.id ? C.bg3 : "transparent",
            borderBottom: `2px solid ${walletView === v.id ? C.accent : "transparent"}`,
            color: walletView === v.id ? C.white : C.dim, fontWeight: walletView === v.id ? 700 : 400,
          }}>
            <span style={{ color: C.accentDim, fontWeight: 700, marginRight: 6 }}>{i + 1}</span>{v.label}
          </div>
        ))}
      </div>

      {walletView === "balance" && (
        <div>
          <div style={{ color: C.accentDim, whiteSpace: "pre", fontSize: 12 }}>╭──<span style={{ color: C.accent, fontWeight: 700 }}> HUMAN WALLET </span>────────────────────────────────────────────────╮</div>
          <div style={{ borderLeft: `1px solid ${C.accentDim}`, borderRight: `1px solid ${C.accentDim}`, background: C.bg2, padding: "14px 16px", display: "flex", alignItems: "center", gap: 30 }}>
            <div>
              <div style={{ color: C.dim, fontSize: 10, letterSpacing: 1 }}>ZEUS TOKEN</div>
              <div style={{ color: C.white, fontSize: 30, fontWeight: 700 }}>184,920 <span style={{ color: C.accent, fontSize: 14 }}>ZEUS</span></div>
            </div>
            <div style={{ color: C.muted }}>│</div>
            <div>
              <div style={{ color: C.dim, fontSize: 10, letterSpacing: 1 }}>CREDIT</div>
              <div style={{ color: C.amber, fontSize: 24, fontWeight: 700 }}>4,680 <span style={{ fontSize: 12 }}>CR</span></div>
            </div>
            <div style={{ flex: 1 }} />
            <div style={{ textAlign: "right" }}>
              <div style={{ color: C.dim, fontSize: 10, letterSpacing: 1 }}>FLEET TOTAL</div>
              <div style={{ color: C.accentBright, fontSize: 18, fontWeight: 700 }}>{wfmt(fleetTotal)} ZEUS</div>
            </div>
          </div>
          <div style={{ color: C.accentDim, whiteSpace: "pre", fontSize: 12 }}>╰───────────────────────────────────────────────────────────────╯</div>

          <div style={{ marginTop: 14, color: C.dim, fontSize: 10, letterSpacing: 2 }}>TITAN WALLETS  <span style={{ color: C.muted }}>[j/k to navigate · ↵ economy]</span></div>
          <div style={{ marginTop: 6 }}>
            <div style={{ display: "grid", gridTemplateColumns: "20px 130px 110px 90px 80px 1fr", gap: 8, padding: "3px 8px", color: C.accentDim, fontSize: 10, fontWeight: 700, borderBottom: `1px solid ${C.muted}` }}>
              <span></span><span>TITAN</span><span>ROLE</span><span style={{ textAlign: "right" }}>ZEUS</span><span style={{ textAlign: "right" }}>CREDIT</span><span style={{ textAlign: "right" }}>EARNED / SPENT</span>
            </div>
            {WALLET_TITANS.map((t, i) => (
              <div key={t.name} style={{
                display: "grid", gridTemplateColumns: "20px 130px 110px 90px 80px 1fr", gap: 8, padding: "4px 8px", alignItems: "center",
                background: walletSel === i ? C.bg3 : "transparent",
                borderLeft: `2px solid ${walletSel === i ? t.color : "transparent"}`,
              }}>
                <span style={{ color: t.st === "active" ? C.green : C.muted }}>{t.st === "active" ? "●" : "○"}</span>
                <span style={{ color: walletSel === i ? C.white : C.fg, fontWeight: walletSel === i ? 700 : 400 }}>{walletSel === i ? "▸ " : "  "}{t.name}</span>
                <span style={{ color: C.dim, fontSize: 11 }}>{t.role}</span>
                <span style={{ textAlign: "right", color: C.white, fontWeight: 700 }}>{wfmt(t.token)}</span>
                <span style={{ textAlign: "right", color: C.amber }}>{t.credit}</span>
                <span style={{ textAlign: "right", fontSize: 11 }}>
                  <span style={{ color: C.green }}>↑{wfmt(t.earned)}</span> <span style={{ color: C.muted }}>/</span> <span style={{ color: C.amber }}>↓{wfmt(t.spent)}</span>
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      {walletView === "send" && (
        <div style={{ display: "flex", gap: 16 }}>
          <div style={{ flex: 1.3 }}>
            <div style={{ color: C.accentDim, whiteSpace: "pre", fontSize: 12 }}>╭──<span style={{ color: C.accent, fontWeight: 700 }}> SEND TOKENS </span>──────────────────────────────────╮</div>
            <div style={{ borderLeft: `1px solid ${C.accentDim}`, borderRight: `1px solid ${C.accentDim}`, background: C.bg2, padding: "12px 16px" }}>
              <div style={{ marginBottom: 12 }}>
                <div style={{ color: C.dim, fontSize: 10, letterSpacing: 1, marginBottom: 4 }}>RECIPIENT</div>
                <div style={{ background: C.bg, border: `1px solid ${C.accentDim}`, padding: "8px 10px", color: C.white }}>
                  <span style={{ color: C.accent, marginRight: 8 }}>▸</span>@hephaestus<span style={{ color: C.accent }}>▌</span>
                </div>
                <div style={{ marginTop: 6, display: "flex", gap: 6 }}>
                  {["@hermes", "@aegis", "@atlas"].map(x => <span key={x} style={{ color: C.dim, border: `1px solid ${C.muted}`, padding: "2px 8px", fontSize: 10 }}>{x}</span>)}
                </div>
              </div>
              <div style={{ marginBottom: 12 }}>
                <div style={{ color: C.dim, fontSize: 10, letterSpacing: 1, marginBottom: 4 }}>AMOUNT</div>
                <div style={{ background: C.bg, border: `1px solid ${C.accentDim}`, padding: "10px", color: C.white, fontSize: 24, fontWeight: 700, display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                  <span>5,000</span><span style={{ color: C.accent, fontSize: 13 }}>ZEUS [MAX]</span>
                </div>
              </div>
              <div style={{ marginBottom: 12 }}>
                <div style={{ color: C.dim, fontSize: 10, letterSpacing: 1, marginBottom: 4 }}>MEMO</div>
                <div style={{ background: C.bg, border: `1px solid ${C.muted}`, padding: "8px 10px", color: C.dim }}>compute top-up</div>
              </div>
              <div style={{ borderTop: `1px solid ${C.muted}`, paddingTop: 10, display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                <span style={{ color: C.dim, fontSize: 11 }}>fee 0.001 ZEUS · <span style={{ color: C.cyan }}>x402</span></span>
                <span style={{ background: C.accent, color: C.bg, padding: "5px 16px", fontWeight: 700, fontSize: 11 }}>▸ SIGN [↵]</span>
              </div>
            </div>
            <div style={{ color: C.accentDim, whiteSpace: "pre", fontSize: 12 }}>╰─────────────────────────────────────────────╯</div>
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ color: C.amberDim, whiteSpace: "pre", fontSize: 12 }}>╭──<span style={{ color: C.amber, fontWeight: 700 }}> x402 PAY-FLOW </span>──────────────────╮</div>
            <div style={{ borderLeft: `1px solid ${C.amberDim}`, borderRight: `1px solid ${C.amberDim}`, background: C.bg2, padding: "12px 16px" }}>
              {[["01", "Build tx", "recipient+amount", C.accent], ["02", "Sign Ed25519", "key stays local", C.amber], ["03", "x402 settle", "402→proof", C.cyan], ["04", "Confirmed", "receipt issued", C.green]].map(([n, t, d, c]) => (
                <div key={n} style={{ display: "flex", gap: 10, marginBottom: 12 }}>
                  <span style={{ color: c, fontWeight: 700, opacity: 0.6 }}>{n}</span>
                  <div>
                    <div style={{ color: C.white, fontWeight: 700, fontSize: 12 }}>{t}</div>
                    <div style={{ color: C.dim, fontSize: 10 }}>{d}</div>
                  </div>
                </div>
              ))}
            </div>
            <div style={{ color: C.amberDim, whiteSpace: "pre", fontSize: 12 }}>╰────────────────────────────────╯</div>
          </div>
        </div>
      )}

      {walletView === "receive" && (
        <div style={{ display: "flex", gap: 16 }}>
          <div style={{ flex: 1 }}>
            <div style={{ color: C.green, whiteSpace: "pre", fontSize: 12 }}>╭──<span style={{ fontWeight: 700 }}> RECEIVE · YOUR ADDRESS </span>────────────────╮</div>
            <div style={{ borderLeft: `1px solid ${C.green}`, borderRight: `1px solid ${C.green}`, background: C.bg2, padding: "12px 16px" }}>
              <div style={{ display: "flex", justifyContent: "center", marginBottom: 12 }}>
                <div style={{ lineHeight: 1, fontSize: 8 }}>
                  {Array.from({ length: 13 }).map((_, r) => (
                    <div key={r} style={{ color: C.white, whiteSpace: "pre" }}>
                      {Array.from({ length: 13 }).map((_, c) => {
                        const finder = (r < 3 && c < 3) || (r < 3 && c > 9) || (r > 9 && c < 3);
                        const on = finder || ((r * 7 + c * 5 + r * c) % 3 === 0);
                        return on ? "██" : "  ";
                      }).join("")}
                    </div>
                  ))}
                </div>
              </div>
              <div style={{ color: C.fg, fontSize: 10, wordBreak: "break-all", textAlign: "center", background: C.bg, border: `1px solid ${C.muted}`, padding: "8px", lineHeight: 1.5 }}>{WALLET_ADDR}</div>
              <div style={{ marginTop: 8, display: "flex", gap: 6, justifyContent: "center" }}>
                <span style={{ background: C.accent, color: C.bg, padding: "4px 12px", fontWeight: 700, fontSize: 10 }}>⎘ COPY [c]</span>
                <span style={{ border: `1px solid ${C.muted}`, color: C.dim, padding: "4px 12px", fontSize: 10 }}>↗ SHARE [s]</span>
              </div>
            </div>
            <div style={{ color: C.green, whiteSpace: "pre", fontSize: 12 }}>╰──────────────────────────────────────╯</div>
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ color: C.accentDim, whiteSpace: "pre", fontSize: 12 }}>╭──<span style={{ color: C.accent, fontWeight: 700 }}> RECEIVE TO A TITAN </span>──────────────╮</div>
            <div style={{ borderLeft: `1px solid ${C.accentDim}`, borderRight: `1px solid ${C.accentDim}`, background: C.bg2, padding: "10px 16px" }}>
              {WALLET_TITANS.slice(0, 6).map(t => (
                <div key={t.name} style={{ display: "flex", alignItems: "center", gap: 8, padding: "5px 0", borderBottom: `1px solid ${C.muted}` }}>
                  <span style={{ color: t.color }}>◈</span>
                  <span style={{ color: C.white, width: 90, fontSize: 11 }}>{t.name}</span>
                  <span style={{ color: C.dim, fontSize: 10, flex: 1 }}>{t.addr}</span>
                  <span style={{ color: C.accentDim, fontSize: 10 }}>[QR]</span>
                </div>
              ))}
            </div>
            <div style={{ color: C.accentDim, whiteSpace: "pre", fontSize: 12 }}>╰────────────────────────────────╯</div>
          </div>
        </div>
      )}

      {walletView === "activity" && (
        <div>
          <div style={{ color: C.accentDim, whiteSpace: "pre", fontSize: 12 }}>╭──<span style={{ color: C.accent, fontWeight: 700 }}> TRANSACTION HISTORY </span>───────────────────────────────────────────────╮</div>
          <div style={{ borderLeft: `1px solid ${C.accentDim}`, borderRight: `1px solid ${C.accentDim}`, background: C.bg2, padding: "8px 16px" }}>
            <div style={{ display: "grid", gridTemplateColumns: "70px 1fr 110px 80px 50px", gap: 8, padding: "2px 4px 6px", color: C.accentDim, fontSize: 10, fontWeight: 700, borderBottom: `1px solid ${C.muted}` }}>
              <span>TYPE</span><span>COUNTERPARTY</span><span style={{ textAlign: "right" }}>AMOUNT</span><span>STATUS</span><span style={{ textAlign: "right" }}>TIME</span>
            </div>
            {WALLET_ACT.map((tx, i) => {
              const k = WALLET_KIND[tx.k];
              return (
                <div key={i} style={{ display: "grid", gridTemplateColumns: "70px 1fr 110px 80px 50px", gap: 8, padding: "5px 4px", alignItems: "center", borderBottom: `1px solid ${C.muted}` }}>
                  <span style={{ color: k.c, fontSize: 10, fontWeight: 700 }}>{k.g} {k.l}</span>
                  <span><span style={{ color: C.fg }}>{tx.who}</span> <span style={{ color: C.muted, fontSize: 10 }}>· {tx.note}</span></span>
                  <span style={{ textAlign: "right", color: tx.k === "recv" || tx.k === "mint" ? C.green : C.fg, fontWeight: 700 }}>{tx.k === "recv" || tx.k === "mint" ? "+" : "−"}{wfmt(tx.amt)} <span style={{ color: C.dim, fontSize: 10 }}>{tx.u}</span></span>
                  <span style={{ color: WALLET_STC[tx.st], fontSize: 11 }}>● {tx.st}</span>
                  <span style={{ textAlign: "right", color: C.dim, fontSize: 11 }}>{tx.t}</span>
                </div>
              );
            })}
          </div>
          <div style={{ color: C.accentDim, whiteSpace: "pre", fontSize: 12 }}>╰──────────────────────────────────────────────────────────────────────╯</div>
        </div>
      )}

      {walletView === "economy" && (() => {
        const t = WALLET_TITANS[walletSel]; const net = t.earned - t.spent;
        return (
          <div>
            <div style={{ color: C.dim, fontSize: 10, letterSpacing: 2, marginBottom: 8 }}>SELECT TITAN <span style={{ color: C.muted }}>[j/k]</span></div>
            <div style={{ display: "flex", gap: 6, flexWrap: "wrap", marginBottom: 14 }}>
              {WALLET_TITANS.map((x, i) => (
                <span key={x.name} style={{ padding: "3px 10px", fontSize: 11, border: `1px solid ${walletSel === i ? x.color : C.muted}`, background: walletSel === i ? C.bg3 : "transparent", color: walletSel === i ? x.color : C.dim }}>{x.name}</span>
              ))}
            </div>
            <div style={{ display: "flex", gap: 12, marginBottom: 14 }}>
              {[["EARNED", wfmt(t.earned), C.green, "↑"], ["SPENT", wfmt(t.spent), C.amber, "↓"], ["NET", (net >= 0 ? "+" : "−") + wfmt(Math.abs(net)), net >= 0 ? C.green : C.red, "⋔"]].map(([l, v, c, g]) => (
                <div key={l} style={{ flex: 1, border: `1px solid ${C.muted}`, borderLeft: `2px solid ${c}`, background: C.bg2, padding: "10px 14px" }}>
                  <div style={{ color: c, fontSize: 10, fontWeight: 700 }}>{g} {l}</div>
                  <div style={{ color: C.white, fontSize: 22, fontWeight: 700 }}>{v} <span style={{ color: C.dim, fontSize: 11 }}>ZEUS</span></div>
                </div>
              ))}
            </div>
            <div style={{ color: C.accentDim, whiteSpace: "pre", fontSize: 12 }}>╭──<span style={{ color: C.accent, fontWeight: 700 }}> AGORA · {t.name.toUpperCase()} </span>────────────────────────────────────────╮</div>
            <div style={{ borderLeft: `1px solid ${C.accentDim}`, borderRight: `1px solid ${C.accentDim}`, background: C.bg2, padding: "8px 16px" }}>
              {[["SOLD", "content-generation service", "+2,400", C.green, "2m"], ["BOUGHT", "advanced-codegen skill", "−499 CR", C.amber, "1h"], ["SOLD", "x402 search task", "+18", C.green, "1h"], ["LISTED", "brand-voice skill · 1,200 ZEUS", "pending", C.dim, "4h"]].map(([d, item, amt, c, tm], i) => (
                <div key={i} style={{ display: "flex", alignItems: "center", gap: 12, padding: "5px 0", borderBottom: i < 3 ? `1px solid ${C.muted}` : "none" }}>
                  <span style={{ color: c, fontSize: 10, fontWeight: 700, width: 60 }}>◇ {d}</span>
                  <span style={{ color: C.fg, flex: 1, fontSize: 11 }}>{item}</span>
                  <span style={{ color: c, fontWeight: 700, fontSize: 11 }}>{amt}</span>
                  <span style={{ color: C.muted, fontSize: 10, width: 30, textAlign: "right" }}>{tm}</span>
                </div>
              ))}
            </div>
            <div style={{ color: C.accentDim, whiteSpace: "pre", fontSize: 12 }}>╰──────────────────────────────────────────────────────────╯</div>
          </div>
        );
      })()}

      {walletView === "security" && (
        <div>
          <div style={{ color: C.amberDim, whiteSpace: "pre", fontSize: 12 }}>╭──<span style={{ color: C.amber, fontWeight: 700 }}> KEYPAIR · ED25519 </span>──────────────────────────────────╮</div>
          <div style={{ borderLeft: `1px solid ${C.amberDim}`, borderRight: `1px solid ${C.amberDim}`, background: C.bg2, padding: "12px 16px" }}>
            <div style={{ color: C.fg, fontSize: 12, marginBottom: 4 }}>⚿ Your keys, your titans.</div>
            <div style={{ color: C.dim, fontSize: 11, marginBottom: 12, lineHeight: 1.5 }}>Private keys generated and stored locally. Back up your recovery phrase — without it, access cannot be restored.</div>
            <div style={{ display: "flex", gap: 8 }}>
              <span style={{ background: C.accent, color: C.bg, padding: "5px 14px", fontWeight: 700, fontSize: 11 }}>⚿ REVEAL PHRASE [r]</span>
              <span style={{ border: `1px solid ${C.muted}`, color: C.dim, padding: "5px 14px", fontSize: 11 }}>⤓ EXPORT KEYPAIR [e]</span>
            </div>
          </div>
          <div style={{ color: C.amberDim, whiteSpace: "pre", fontSize: 12 }}>╰──────────────────────────────────────────────╯</div>

          <div style={{ height: 12 }} />

          <div style={{ color: C.accentDim, whiteSpace: "pre", fontSize: 12 }}>╭──<span style={{ color: C.accent, fontWeight: 700 }}> x402 AUTHORIZATIONS · 3 ACTIVE </span>───────────────────────────╮</div>
          <div style={{ borderLeft: `1px solid ${C.accentDim}`, borderRight: `1px solid ${C.accentDim}`, background: C.bg2, padding: "8px 16px" }}>
            {[["Agora Marketplace", "Auto-settle purchases < 500 ZEUS", "12d", C.green], ["Hephaestus", "Compute spend ≤ 5,000 ZEUS/day", "8d", C.accent], ["Calliope", "Content sales auto-receive", "3d", C.amber]].map(([who, scope, since, c], i) => (
              <div key={i} style={{ display: "flex", alignItems: "center", gap: 10, padding: "6px 0", borderBottom: i < 2 ? `1px solid ${C.muted}` : "none" }}>
                <span style={{ color: c }}>◈</span>
                <div style={{ flex: 1 }}>
                  <div style={{ color: C.white, fontSize: 12, fontWeight: 700 }}>{who}</div>
                  <div style={{ color: C.dim, fontSize: 10 }}>{scope}</div>
                </div>
                <span style={{ color: C.muted, fontSize: 10 }}>auth {since}</span>
                <span style={{ border: `1px solid ${C.red}`, color: C.red, padding: "2px 10px", fontSize: 10 }}>REVOKE</span>
              </div>
            ))}
          </div>
          <div style={{ color: C.accentDim, whiteSpace: "pre", fontSize: 12 }}>╰──────────────────────────────────────────────────────────╯</div>
        </div>
      )}
    </div>
  );
};

/* ═══════════════════════════════════════════════════ */
/* TAB 6 — CHANNELS                                     */
/* ═══════════════════════════════════════════════════ */
const ChannelsTab = () => {
  const channels = [
    { id: "discord", name: "Discord", glyph: "DC", color: C.purple, status: "connected", binding: "ZeusBot#0042 in 8 servers", recent: "2m ago", msgs24h: 247, sdk: "Serenity gateway" },
    { id: "telegram", name: "Telegram", glyph: "TG", color: C.blue, status: "connected", binding: "+1 555 0117", recent: "5m ago", msgs24h: 89, sdk: "grammers MTProto" },
    { id: "slack", name: "Slack", glyph: "SL", color: C.green, status: "connected", binding: "novaxai workspace", recent: "23m ago", msgs24h: 34, sdk: "Socket Mode" },
    { id: "email", name: "Email", glyph: "EM", color: C.amber, status: "connected", binding: "[email protected]", recent: "1h ago", msgs24h: 12, sdk: "lettre + IMAP IDLE" },
    { id: "imessage", name: "iMessage", glyph: "iM", color: C.cyan, status: "connected", binding: "via AppleScript bridge", recent: "yesterday", msgs24h: 4, sdk: "AppleScript" },
    { id: "whatsapp", name: "WhatsApp", glyph: "WA", color: C.green, status: "reconnecting", binding: "+1 555 0117 (paired)", recent: "—", msgs24h: 0, sdk: "Cloud API" },
    { id: "signal", name: "Signal", glyph: "SG", color: C.blue, status: "disconnected", binding: "—", recent: "—", msgs24h: 0, sdk: "signal-cli" },
    { id: "matrix", name: "Matrix", glyph: "MX", color: C.accent, status: "disconnected", binding: "—", recent: "—", msgs24h: 0, sdk: "matrix-sdk" },
  ];

  const statusColors = {
    connected: C.green, reconnecting: C.amber, disconnected: C.dim,
  };

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", padding: "10px 16px", overflow: "auto" }}>
      <div style={{ marginBottom: 14, display: "flex", alignItems: "center", gap: 14 }}>
        <div style={{ flex: 1 }}>
          <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Messaging adapters</div>
          <div style={{ color: C.dim, fontSize: 11 }}>8 channels — all running in single zeus-channels process</div>
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          {[["connected", channels.filter(c => c.status === "connected").length], ["reconnecting", channels.filter(c => c.status === "reconnecting").length], ["disconnected", channels.filter(c => c.status === "disconnected").length]].map(([s, n]) => (
            <div key={s} style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 10 }}>
              <span style={{ color: statusColors[s], fontSize: 8 }}>●</span>
              <span style={{ color: C.fg, fontWeight: 700 }}>{n}</span>
              <span style={{ color: C.muted, letterSpacing: 1, textTransform: "uppercase", fontSize: 8 }}>{s}</span>
            </div>
          ))}
        </div>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        {channels.map(c => (
          <div key={c.id} style={{
            display: "flex", alignItems: "center", gap: 12, padding: "10px 14px",
            background: C.bg2, border: `1px solid ${C.muted}`, borderLeft: `2px solid ${c.color}`,
          }}>
            <div style={{
              width: 36, height: 22, display: "flex", alignItems: "center", justifyContent: "center",
              background: c.color, color: C.bg, fontWeight: 700, fontSize: 10, letterSpacing: 1,
            }}>{c.glyph}</div>

            <div style={{ width: 100 }}>
              <div style={{ color: C.white, fontSize: 12, fontWeight: 700 }}>{c.name}</div>
              <div style={{ color: C.muted, fontSize: 9 }}>{c.sdk}</div>
            </div>

            <div style={{ display: "flex", alignItems: "center", gap: 6, width: 130 }}>
              <span style={{ color: statusColors[c.status], fontSize: 9 }}>●</span>
              <span style={{ color: statusColors[c.status], fontSize: 10, fontWeight: 700, letterSpacing: 1, textTransform: "uppercase" }}>{c.status}</span>
            </div>

            <div style={{ flex: 1 }}>
              <div style={{ color: C.fg, fontSize: 11 }}>{c.binding}</div>
              <div style={{ color: C.dim, fontSize: 9 }}>last msg · {c.recent}</div>
            </div>

            <div style={{ width: 90, textAlign: "right" }}>
              <div style={{ color: C.accent, fontSize: 14, fontWeight: 700, fontFamily: "inherit" }}>{c.msgs24h}</div>
              <div style={{ color: C.muted, fontSize: 8, letterSpacing: 1 }}>MSGS / 24H</div>
            </div>

            <div style={{ display: "flex", gap: 4 }}>
              <button style={{ background: "transparent", color: C.dim, border: `1px solid ${C.muted}`, padding: "3px 8px", fontSize: 9, fontWeight: 700, letterSpacing: 1, fontFamily: "inherit", cursor: "pointer" }}>TEST</button>
              <button style={{ background: "transparent", color: C.dim, border: `1px solid ${C.muted}`, padding: "3px 8px", fontSize: 9, fontWeight: 700, letterSpacing: 1, fontFamily: "inherit", cursor: "pointer" }}>EDIT</button>
              {c.status === "connected" && (
                <button style={{ background: "transparent", color: C.amber, border: `1px solid ${C.amber}`, padding: "3px 8px", fontSize: 9, fontWeight: 700, letterSpacing: 1, fontFamily: "inherit", cursor: "pointer" }}>PAUSE</button>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
};

/* ═══════════════════════════════════════════════════ */
/* TAB 7 — APPROVALS                                    */
/* ═══════════════════════════════════════════════════ */
const ApprovalsTab = () => {
  const pending = [
    { id: "ap_47", agent: "Hephaestus", tool: "shell", args: 'rm -rf node_modules && npm install', reason: "rm -rf flagged by Aegis — destructive operation", risk: "high", time: "32s ago" },
    { id: "ap_46", agent: "Hermes", tool: "web_fetch", args: 'https://api.unallowlisted.com/data', reason: "URL not in allowlist", risk: "medium", time: "1m ago" },
    { id: "ap_45", agent: "Atlas", tool: "apply_patch", args: '@@ -147,3 +147,8 @@ ... (47 lines diff)', reason: "patch touches /etc/hosts", risk: "high", time: "3m ago" },
    { id: "ap_44", agent: "Calliope", tool: "discord_send", args: '{"channel": "#announcements", "content": "v0.4.7 ships tomorrow!"}', reason: "channel announcements requires approval", risk: "low", time: "8m ago" },
  ];

  const riskColors = { high: C.red, medium: C.amber, low: C.yellow };

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
      <div style={{ padding: "10px 16px", borderBottom: `1px solid ${C.muted}`, background: C.bg2, display: "flex", alignItems: "center", gap: 14 }}>
        <div style={{ flex: 1 }}>
          <div style={{ color: C.fg, fontSize: 14, fontWeight: 700 }}>
            <span style={{ color: C.amber }}>{pending.length}</span> pending approval{pending.length !== 1 ? "s" : ""}
          </div>
          <div style={{ color: C.dim, fontSize: 10 }}>Aegis sandbox blocked these — review before allowing</div>
        </div>
        <div style={{ fontSize: 10, color: C.dim }}>
          <span style={{ color: C.accentDim, fontWeight: 700 }}>a</span> approve  ·  <span style={{ color: C.accentDim, fontWeight: 700 }}>d</span> deny  ·  <span style={{ color: C.accentDim, fontWeight: 700 }}>A</span> approve all  ·  <span style={{ color: C.accentDim, fontWeight: 700 }}>D</span> deny all
        </div>
      </div>

      <div style={{ flex: 1, overflowY: "auto", padding: "10px 16px", display: "flex", flexDirection: "column", gap: 10 }}>
        {pending.map((p, i) => (
          <div key={p.id} style={{
            background: C.bg2, border: `1px solid ${C.muted}`, borderLeft: `2px solid ${riskColors[p.risk]}`,
            padding: "10px 14px",
          }}>
            <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 6 }}>
              <span style={{ color: riskColors[p.risk], fontSize: 10 }}>⚠</span>
              <span style={{ color: C.white, fontSize: 12, fontWeight: 700 }}>{p.tool}</span>
              <span style={{ color: C.dim, fontSize: 10 }}>by</span>
              <span style={{ color: C.accent, fontSize: 11, fontWeight: 700 }}>{p.agent}</span>
              <span style={{ flex: 1 }} />
              <span style={{ padding: "1px 6px", fontSize: 8, fontWeight: 700, letterSpacing: 2, color: riskColors[p.risk], border: `1px solid ${riskColors[p.risk]}` }}>{p.risk.toUpperCase()} RISK</span>
              <span style={{ color: C.muted, fontSize: 9 }}>{p.time}</span>
            </div>
            <div style={{ marginBottom: 4 }}>
              <span style={{ color: C.muted, fontSize: 8, fontWeight: 700, letterSpacing: 2 }}>ARGS</span>
              <div style={{ background: C.bg, padding: "6px 10px", border: `1px solid ${C.muted}`, fontSize: 10, color: C.cyan, marginTop: 2, fontFamily: "inherit", whiteSpace: "pre-wrap", maxHeight: 100, overflowY: "auto" }}>{p.args}</div>
            </div>
            <div style={{ marginBottom: 8 }}>
              <span style={{ color: C.muted, fontSize: 8, fontWeight: 700, letterSpacing: 2 }}>WHY BLOCKED</span>
              <div style={{ color: riskColors[p.risk], fontSize: 10, marginTop: 2 }}>{p.reason}</div>
            </div>
            <div style={{ display: "flex", gap: 6 }}>
              <button style={{ background: C.green, color: C.bg, border: "none", padding: "4px 14px", fontSize: 10, fontWeight: 700, letterSpacing: 2, fontFamily: "inherit", cursor: "pointer" }}>a APPROVE</button>
              <button style={{ background: "transparent", color: C.red, border: `1px solid ${C.red}`, padding: "4px 14px", fontSize: 10, fontWeight: 700, letterSpacing: 2, fontFamily: "inherit", cursor: "pointer" }}>d DENY</button>
              <button style={{ background: "transparent", color: C.dim, border: `1px solid ${C.muted}`, padding: "4px 12px", fontSize: 10, fontWeight: 700, letterSpacing: 2, fontFamily: "inherit", cursor: "pointer" }}>VIEW FULL</button>
              <span style={{ flex: 1 }} />
              <button style={{ background: "transparent", color: C.amber, border: `1px solid ${C.amber}`, padding: "4px 12px", fontSize: 9, fontWeight: 700, letterSpacing: 1, fontFamily: "inherit", cursor: "pointer" }}>ALWAYS ALLOW THIS PATTERN</button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
};

/* ═══════════════════════════════════════════════════ */
/* TAB 8 — SETTINGS                                     */
/* ═══════════════════════════════════════════════════ */
const SettingsTab = ({ selectedGroup, setSelectedGroup }) => {
  const groups = [
    { id: "llm", name: "LLM", icon: "◇", color: C.accent, fields: [
      { k: "Provider", v: "anthropic", help: "Primary LLM provider" },
      { k: "Model", v: "claude-opus-4-7", help: "Specific model from provider catalog" },
      { k: "Temperature", v: "0.7", help: "Sampling temperature 0.0–2.0" },
      { k: "Max iterations", v: "200", help: "Cooking loop iteration cap" },
      { k: "Fallback chain", v: "openai/gpt-4o, groq/llama-3.3-70b", help: "Comma-separated fallback providers" },
    ]},
    { id: "channels", name: "Channels", icon: "⇌", color: C.green, fields: [
      { k: "Discord", v: "✓ enabled", help: "Discord bot adapter" },
      { k: "Telegram", v: "✓ enabled", help: "Telegram MTProto adapter" },
      { k: "Slack", v: "✓ enabled", help: "Slack Socket Mode adapter" },
      { k: "Email", v: "✓ enabled", help: "SMTP + IMAP IDLE adapter" },
      { k: "iMessage", v: "✓ enabled (macOS)", help: "AppleScript bridge" },
      { k: "WhatsApp", v: "○ disabled", help: "Cloud API adapter" },
      { k: "Signal", v: "○ disabled", help: "signal-cli adapter" },
      { k: "Matrix", v: "○ disabled", help: "matrix-sdk adapter" },
    ]},
    { id: "memory", name: "Memory", icon: "▤", color: C.cyan, fields: [
      { k: "DB path", v: "~/.zeus/mnemosyne.db", help: "SQLite + vector store location" },
      { k: "Embedding provider", v: "ollama", help: "Embedding model provider", dirty: true },
      { k: "Embedding model", v: "nomic-embed-text", help: "Specific embedding model" },
      { k: "FTS enabled", v: "✓ true", help: "SQLite FTS5 full-text index" },
      { k: "Auto-prune", v: "30 days", help: "Old session cleanup threshold" },
    ]},
    { id: "security", name: "Security", icon: "🛡", color: C.red, fields: [
      { k: "Aegis level", v: "standard", help: "Sandbox aggressiveness" },
      { k: "Approval mode", v: "interactive", help: "How approvals are surfaced" },
      { k: "Command allowlist", v: "47 entries", help: "Approved shell commands" },
      { k: "URL allowlist", v: "12 entries", help: "Approved web_fetch URLs" },
      { k: "Audit log", v: "~/.zeus/audit.jsonl", help: "Audit trail location" },
    ]},
    { id: "tools", name: "Tools", icon: "⚙", color: C.amber, fields: [
      { k: "Talos enabled", v: "✓ FORCE-ON (macOS)", help: "macOS automation crate", locked: true },
      { k: "Browser", v: "✓ enabled", help: "Chrome CDP automation" },
      { k: "MCP servers", v: "3 connected", help: "Active MCP server count" },
      { k: "Tool timeout", v: "30s", help: "Per-tool execution timeout" },
    ]},
    { id: "display", name: "Display", icon: "▦", color: C.purple, fields: [
      { k: "Theme", v: "dark", help: "Color theme" },
      { k: "Accent color", v: "fire-orange", help: "UI accent color" },
      { k: "Vim mode", v: "✓ true", help: "Vim-style keybinds" },
      { k: "High contrast", v: "○ false", help: "Accessibility mode" },
      { k: "Animations", v: "✓ true", help: "Enable streaming animations" },
    ]},
    { id: "system", name: "System", icon: "⊕", color: C.dim, fields: [
      { k: "Re-run onboarding", v: "→", help: "Launch zeus onboard --resume", action: true },
      { k: "Daemon status", v: "→", help: "View / restart gateway daemon", action: true },
      { k: "Export config", v: "→", help: "Save config.toml to file", action: true },
      { k: "Build version", v: "0.4.7-rc.3 (a1c4f29)", help: "Current build" },
      { k: "Workspace path", v: "~/.zeus/workspace", help: "Agent workspace location" },
    ]},
  ];

  const sel = groups.find(g => g.id === selectedGroup) || groups[0];

  return (
    <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
      {/* Group list */}
      <div style={{ width: 200, borderRight: `1px solid ${C.muted}`, background: C.bg2, padding: "10px 0" }}>
        <div style={{ padding: "0 12px", marginBottom: 8 }}>
          <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3 }}>SUBSYSTEM</div>
        </div>
        {groups.map(g => {
          const dirtyCount = g.fields.filter(f => f.dirty).length;
          return (
            <div key={g.id} onClick={() => setSelectedGroup(g.id)} style={{
              padding: "6px 12px", cursor: "pointer",
              background: selectedGroup === g.id ? C.bg3 : "transparent",
              borderLeft: `2px solid ${selectedGroup === g.id ? g.color : "transparent"}`,
              display: "flex", alignItems: "center", gap: 8,
            }}>
              <span style={{ color: selectedGroup === g.id ? g.color : C.muted, fontSize: 11, width: 14, textAlign: "center" }}>{g.icon}</span>
              <span style={{ color: selectedGroup === g.id ? C.white : C.fg, fontSize: 11, fontWeight: selectedGroup === g.id ? 700 : 400, flex: 1 }}>{g.name}</span>
              {dirtyCount > 0 && (
                <span style={{ color: C.amber, fontSize: 9 }}>●</span>
              )}
              <span style={{ color: C.muted, fontSize: 9 }}>{g.fields.length}</span>
            </div>
          );
        })}
      </div>

      {/* Fields */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
        <div style={{ padding: "12px 18px", borderBottom: `1px solid ${C.muted}`, background: C.bg2 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <span style={{ color: sel.color, fontSize: 16 }}>{sel.icon}</span>
            <span style={{ color: C.white, fontSize: 16, fontWeight: 700 }}>{sel.name}</span>
            <span style={{ color: C.dim, fontSize: 10 }}>{sel.fields.length} settings</span>
            <span style={{ flex: 1 }} />
            <span style={{ color: C.muted, fontSize: 9 }}>changes save on Enter · Esc to discard</span>
          </div>
        </div>

        <div style={{ flex: 1, overflowY: "auto", padding: "10px 0" }}>
          {sel.fields.map((f, i) => (
            <div key={i} style={{
              padding: "8px 18px", borderBottom: `1px solid ${C.muted}`,
              display: "flex", alignItems: "center", gap: 14,
              background: f.dirty ? "rgba(234, 179, 8, 0.05)" : "transparent",
            }}>
              <div style={{ width: 200, display: "flex", alignItems: "center", gap: 6 }}>
                {f.dirty && <span style={{ color: C.amber, fontSize: 10 }}>*</span>}
                <span style={{ color: C.fg, fontSize: 11, fontWeight: 600 }}>{f.k}</span>
                {f.locked && <span style={{ color: C.red, fontSize: 8, fontWeight: 700, letterSpacing: 1 }}>🔒</span>}
              </div>
              <div style={{ flex: 1 }}>
                <div style={{ color: f.action ? C.accent : f.locked ? C.dim : C.fg, fontSize: 11, fontFamily: "inherit" }}>{f.v}</div>
                <div style={{ color: C.dim, fontSize: 9, marginTop: 1, fontStyle: "italic" }}>{f.help}</div>
              </div>
              {!f.locked && !f.action && (
                <button style={{ background: "transparent", color: C.dim, border: `1px solid ${C.muted}`, padding: "3px 10px", fontSize: 9, fontWeight: 700, letterSpacing: 2, fontFamily: "inherit", cursor: "pointer" }}>EDIT</button>
              )}
              {f.action && (
                <button style={{ background: C.accent, color: C.bg, border: "none", padding: "3px 12px", fontSize: 9, fontWeight: 700, letterSpacing: 2, fontFamily: "inherit", cursor: "pointer" }}>RUN</button>
              )}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
};

/* ═══════════════════════════════════════════════════ */
/* TAB 9 — ADVANCED SUBMENU                             */
/* ═══════════════════════════════════════════════════ */
const AdvancedTab = ({ activeAdv, setActiveAdv }) => {
  if (activeAdv) {
    const t = ADVANCED_TABS.find(x => x.id === activeAdv);
    return (
      <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
        <div style={{ padding: "10px 16px", borderBottom: `1px solid ${C.muted}`, background: C.bg2, display: "flex", alignItems: "center", gap: 12 }}>
          <span onClick={() => setActiveAdv(null)} style={{ color: C.accent, fontSize: 11, cursor: "pointer", fontWeight: 700 }}>← Advanced</span>
          <span style={{ color: C.muted }}>/</span>
          <div style={{ width: 32, height: 18, background: t.color, color: C.bg, display: "flex", alignItems: "center", justifyContent: "center", fontWeight: 700, fontSize: 9, letterSpacing: 1 }}>{t.glyph}</div>
          <span style={{ color: C.white, fontSize: 14, fontWeight: 700 }}>{t.name}</span>
          <span style={{ color: C.dim, fontSize: 10 }}>· {t.desc}</span>
        </div>
        <AdvancedSubview id={activeAdv} />
      </div>
    );
  }

  return (
    <div style={{ flex: 1, padding: "12px 18px", overflowY: "auto" }}>
      <div style={{ marginBottom: 14 }}>
        <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Advanced subsystems</div>
        <div style={{ color: C.dim, fontSize: 11 }}>13 specialized views — every backend feature has a TUI surface</div>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 6 }}>
        {ADVANCED_TABS.map(t => (
          <div key={t.id} onClick={() => setActiveAdv(t.id)} style={{
            background: C.bg2, border: `1px solid ${C.muted}`, borderLeft: `2px solid ${t.color}`,
            padding: "10px 12px", cursor: "pointer",
            display: "flex", alignItems: "center", gap: 10,
          }}>
            <div style={{ width: 32, height: 22, background: C.bg, color: t.color, display: "flex", alignItems: "center", justifyContent: "center", fontWeight: 700, fontSize: 9, letterSpacing: 1, border: `1px solid ${t.color}`, flexShrink: 0 }}>{t.glyph}</div>
            <div style={{ flex: 1 }}>
              <div style={{ color: C.fg, fontSize: 12, fontWeight: 700 }}>{t.name}</div>
              <div style={{ color: C.dim, fontSize: 9, marginTop: 1 }}>{t.desc}</div>
            </div>
            <span style={{ color: C.muted, fontSize: 11 }}>›</span>
          </div>
        ))}
      </div>
    </div>
  );
};

const AdvancedSubview = ({ id }) => {
  // Each advanced subview shows representative content
  if (id === "agents") {
    const agents = [
      { name: "Hermes", host: "MacBook Pro M5", role: "Coordinator", local: true, status: "active", channels: 4 },
      { name: "Hephaestus", host: "MacBook Pro", role: "Backend & Architecture", local: false, status: "active", channels: 2 },
      { name: "Prometheus", host: "Mac Studio VM", role: "Experimental", local: false, status: "idle", channels: 1 },
      { name: "Atlas", host: "Mac Studio M1 Ultra", role: "Backend (dual)", local: false, status: "active", channels: 3 },
      { name: "Calliope", host: "Mac Mini M2", role: "Marketing & Content", local: false, status: "active", channels: 5 },
      { name: "Aegis", host: "Mac Mini M4 Pro", role: "Security & CI", local: false, status: "active", channels: 2 },
      { name: "Argus", host: "FreeBSD 15.0", role: "Fleet Provisioner", local: false, status: "active", channels: 1 },
      { name: "Hestia", host: "FreeBSD 15.0", role: "Web Platform", local: false, status: "active", channels: 1 },
    ];
    return (
      <div style={{ flex: 1, padding: "10px 16px", overflowY: "auto" }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          {agents.map(a => (
            <div key={a.name} style={{ display: "flex", alignItems: "center", gap: 14, padding: "8px 14px", background: C.bg2, border: `1px solid ${C.muted}`, borderLeft: `2px solid ${a.local ? C.accent : C.dim}` }}>
              <span style={{ color: a.status === "active" ? C.green : C.dim, fontSize: 9 }}>●</span>
              <span style={{ color: C.white, fontSize: 12, fontWeight: 700, width: 130 }}>{a.name}</span>
              {a.local && <span style={{ padding: "1px 5px", fontSize: 7, fontWeight: 700, letterSpacing: 2, color: C.accent, border: `1px solid ${C.accent}` }}>LOCAL</span>}
              <span style={{ color: C.dim, fontSize: 10, width: 180 }}>{a.host}</span>
              <span style={{ color: C.fg, fontSize: 11, flex: 1 }}>{a.role}</span>
              <span style={{ color: C.dim, fontSize: 9 }}>{a.channels} channels</span>
              <button style={{ background: "transparent", color: C.dim, border: `1px solid ${C.muted}`, padding: "3px 10px", fontSize: 9, fontWeight: 700, letterSpacing: 2, fontFamily: "inherit", cursor: "pointer" }}>MSG</button>
            </div>
          ))}
        </div>
      </div>
    );
  }

  if (id === "skills") {
    return (
      <div style={{ flex: 1, padding: "12px 18px", overflowY: "auto" }}>
        <div style={{ marginBottom: 12, display: "flex", alignItems: "center", gap: 14 }}>
          <span style={{ color: C.fg, fontSize: 11 }}><span style={{ color: C.accent, fontWeight: 700 }}>23</span> installed</span>
          <span style={{ color: C.muted }}>·</span>
          <span style={{ color: C.fg, fontSize: 11 }}><span style={{ color: C.green, fontWeight: 700 }}>147</span> in marketplace</span>
          <span style={{ flex: 1 }} />
          <button style={{ background: C.accent, color: C.bg, border: "none", padding: "4px 12px", fontSize: 10, fontWeight: 700, letterSpacing: 2, fontFamily: "inherit", cursor: "pointer" }}>BROWSE MARKETPLACE</button>
        </div>
        {[
          { name: "git-flow", category: "Dev", enabled: true, tools: 12 },
          { name: "calendar-pro", category: "Productivity", enabled: true, tools: 8 },
          { name: "deep-synth", category: "Research", enabled: true, tools: 4 },
          { name: "secret-scan", category: "Security", enabled: true, tools: 6 },
          { name: "openclaw-compat", category: "Dev", enabled: false, tools: 14 },
        ].map(s => (
          <div key={s.name} style={{ display: "flex", alignItems: "center", gap: 14, padding: "8px 14px", background: C.bg2, border: `1px solid ${C.muted}`, marginBottom: 4 }}>
            <span style={{ color: s.enabled ? C.green : C.dim, fontSize: 9 }}>{s.enabled ? "✓" : "○"}</span>
            <span style={{ color: C.white, fontSize: 12, fontWeight: 700, width: 200 }}>{s.name}</span>
            <span style={{ color: C.dim, fontSize: 10, letterSpacing: 1 }}>{s.category.toUpperCase()}</span>
            <span style={{ flex: 1 }} />
            <span style={{ color: C.dim, fontSize: 10 }}>{s.tools} tools</span>
            <button style={{ background: "transparent", color: C.dim, border: `1px solid ${C.muted}`, padding: "3px 10px", fontSize: 9, fontWeight: 700, letterSpacing: 2, fontFamily: "inherit", cursor: "pointer" }}>VIEW</button>
            <button style={{ background: "transparent", color: s.enabled ? C.amber : C.green, border: `1px solid ${s.enabled ? C.amber : C.green}`, padding: "3px 10px", fontSize: 9, fontWeight: 700, letterSpacing: 2, fontFamily: "inherit", cursor: "pointer" }}>{s.enabled ? "DISABLE" : "ENABLE"}</button>
          </div>
        ))}
      </div>
    );
  }

  if (id === "voice") {
    return (
      <div style={{ flex: 1, padding: "12px 18px", overflowY: "auto" }}>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 10 }}>
          <div style={{ padding: "12px 14px", background: C.bg2, border: `1px solid ${C.muted}` }}>
            <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>TTS PROVIDER</div>
            <div style={{ color: C.white, fontSize: 14, fontWeight: 700 }}>ElevenLabs</div>
            <div style={{ color: C.dim, fontSize: 10, marginTop: 4 }}>voice · Aria · 11labs_v2</div>
          </div>
          <div style={{ padding: "12px 14px", background: C.bg2, border: `1px solid ${C.muted}` }}>
            <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>STT PROVIDER</div>
            <div style={{ color: C.white, fontSize: 14, fontWeight: 700 }}>Whisper · Groq</div>
            <div style={{ color: C.dim, fontSize: 10, marginTop: 4 }}>whisper-large-v3 · 184ms avg</div>
          </div>
          <div style={{ padding: "12px 14px", background: C.bg2, border: `1px solid ${C.muted}` }}>
            <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>TWILIO</div>
            <div style={{ color: C.white, fontSize: 14, fontWeight: 700 }}>+1 555 0117</div>
            <div style={{ color: C.dim, fontSize: 10, marginTop: 4 }}>3 calls today · 47 min total</div>
          </div>
          <div style={{ padding: "12px 14px", background: C.bg2, border: `1px solid ${C.muted}` }}>
            <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>RECORDINGS</div>
            <div style={{ color: C.white, fontSize: 14, fontWeight: 700 }}>147 sessions</div>
            <div style={{ color: C.dim, fontSize: 10, marginTop: 4 }}>~2.4 GB · ~/.zeus/voice/</div>
          </div>
        </div>
      </div>
    );
  }

  if (id === "economy") {
    return (
      <div style={{ flex: 1, padding: "12px 18px", overflowY: "auto" }}>
        <div style={{ background: C.bg2, border: `1px solid ${C.accent}`, borderLeft: `2px solid ${C.accent}`, padding: "14px 18px", marginBottom: 14 }}>
          <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 4 }}>AGORA WALLET</div>
          <div style={{ display: "flex", alignItems: "baseline", gap: 6 }}>
            <span style={{ color: C.white, fontSize: 28, fontWeight: 700, fontFamily: "inherit" }}>$ 247.83</span>
            <span style={{ color: C.dim, fontSize: 11 }}>USDC</span>
          </div>
          <div style={{ color: C.green, fontSize: 10, marginTop: 4 }}>+$ 12.40 today (skill purchases · x402 settlements)</div>
        </div>

        <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>RECENT TRANSACTIONS</div>
        {[
          { t: "14:32", agent: "Hephaestus", action: "purchased", item: "advanced-codegen skill", amt: "-$ 4.99", color: C.amber },
          { t: "13:15", agent: "Hermes", action: "received", item: "x402 settlement · search task", amt: "+$ 0.18", color: C.green },
          { t: "12:48", agent: "Hermes", action: "paid", item: "MiniMax inference (47k tok)", amt: "-$ 0.04", color: C.red },
          { t: "11:30", agent: "Calliope", action: "received", item: "x402 · content generation", amt: "+$ 2.40", color: C.green },
        ].map((tx, i) => (
          <div key={i} style={{ display: "flex", alignItems: "center", gap: 12, padding: "6px 14px", background: C.bg2, border: `1px solid ${C.muted}`, borderLeft: `2px solid ${tx.color}`, marginBottom: 3 }}>
            <span style={{ color: C.muted, fontSize: 10, fontFamily: "inherit", width: 50 }}>{tx.t}</span>
            <span style={{ color: C.fg, fontSize: 11, fontWeight: 700, width: 110 }}>{tx.agent}</span>
            <span style={{ color: C.dim, fontSize: 10 }}>{tx.action}</span>
            <span style={{ color: C.fg, fontSize: 11, flex: 1 }}>{tx.item}</span>
            <span style={{ color: tx.color, fontSize: 12, fontFamily: "inherit", fontWeight: 700 }}>{tx.amt}</span>
          </div>
        ))}
      </div>
    );
  }

  // Generic fallback for the other advanced subviews
  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", padding: "40px", color: C.dim }}>
      <div style={{ fontSize: 32, marginBottom: 14, color: C.accentDim }}>○</div>
      <div style={{ color: C.fg, fontSize: 14, marginBottom: 6 }}>{ADVANCED_TABS.find(t => t.id === id)?.name}</div>
      <div style={{ color: C.dim, fontSize: 11 }}>{ADVANCED_TABS.find(t => t.id === id)?.desc}</div>
      <div style={{ marginTop: 24, padding: "10px 18px", background: C.bg2, border: `1px solid ${C.muted}`, fontSize: 10, color: C.muted }}>Wires into existing crate · backend ready · TUI surface in progress</div>
    </div>
  );
};

/* ═══════════════════════════════════════════════════ */
/* COMMAND PALETTE                                      */
/* ═══════════════════════════════════════════════════ */
const CommandPalette = ({ open, setOpen, onAction }) => {
  const [query, setQuery] = useState("");
  useEffect(() => { if (!open) setQuery(""); }, [open]);
  if (!open) return null;

  const commands = [
    { type: "tab", label: "Switch to chat", k: "→", action: () => onAction("tab", "chat") },
    { type: "tab", label: "Switch to office", k: "→", action: () => onAction("tab", "office") },
    { type: "tab", label: "Switch to pantheon", k: "→", action: () => onAction("tab", "pantheon") },
    { type: "tab", label: "Switch to tools", k: "→", action: () => onAction("tab", "tools") },
    { type: "tab", label: "Switch to memory", k: "→", action: () => onAction("tab", "memory") },
    { type: "tab", label: "Switch to channels", k: "→", action: () => onAction("tab", "channels") },
    { type: "tab", label: "Switch to approvals", k: "→", action: () => onAction("tab", "approvals") },
    { type: "tab", label: "Switch to settings", k: "→", action: () => onAction("tab", "settings") },
    { type: "tool", label: "Run shell", k: "⚙", action: () => {} },
    { type: "tool", label: "Run web_fetch", k: "⚙", action: () => {} },
    { type: "tool", label: "Run memory_recall", k: "⚙", action: () => {} },
    { type: "slash", label: "/clear · clear chat", k: "/", action: () => {} },
    { type: "slash", label: "/compact · compact context", k: "/", action: () => {} },
    { type: "slash", label: "/spawn · spawn subagent", k: "/", action: () => {} },
    { type: "skill", label: "Invoke git-flow skill", k: "★", action: () => {} },
    { type: "settings", label: "Settings · LLM · Provider", k: "⊕", action: () => onAction("tab", "settings") },
    { type: "settings", label: "Settings · Memory · Embedding model", k: "⊕", action: () => onAction("tab", "settings") },
    { type: "advanced", label: "Open Agents view", k: "⌥", action: () => onAction("adv", "agents") },
    { type: "advanced", label: "Open Voice config", k: "⌥", action: () => onAction("adv", "voice") },
    { type: "advanced", label: "Open Economy / Wallet", k: "⌥", action: () => onAction("adv", "economy") },
  ];

  const filtered = commands.filter(c => !query || c.label.toLowerCase().includes(query.toLowerCase()));
  const typeColors = { tab: C.accent, tool: C.amber, slash: C.cyan, skill: C.yellow, settings: C.purple, advanced: C.green };

  return (
    <div style={{
      position: "absolute", top: 0, left: 0, right: 0, bottom: 0,
      background: "rgba(0,0,0,0.7)", display: "flex", alignItems: "flex-start", justifyContent: "center",
      paddingTop: 80, zIndex: 1000,
    }} onClick={() => setOpen(false)}>
      <div onClick={(e) => e.stopPropagation()} style={{
        width: 600, maxHeight: "70vh", background: C.bg2, border: `1px solid ${C.accent}`,
        boxShadow: `0 0 30px rgba(255, 60, 20, 0.3)`,
        display: "flex", flexDirection: "column", overflow: "hidden",
      }}>
        <div style={{ padding: "12px 16px", borderBottom: `1px solid ${C.muted}`, display: "flex", alignItems: "center", gap: 10 }}>
          <span style={{ color: C.accent, fontSize: 14 }}>▸</span>
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Type command, tool, or setting…"
            style={{ flex: 1, background: "transparent", border: "none", color: C.fg, fontFamily: "inherit", fontSize: 13, outline: "none" }}
          />
          <span style={{ color: C.muted, fontSize: 9 }}>{filtered.length}</span>
          <span style={{ color: C.dim, fontSize: 9 }}>Esc</span>
        </div>
        <div style={{ flex: 1, overflowY: "auto" }}>
          {filtered.slice(0, 12).map((c, i) => (
            <div key={i} onClick={() => { c.action(); setOpen(false); }} style={{
              padding: "8px 16px", borderBottom: `1px solid ${C.muted}`,
              cursor: "pointer", display: "flex", alignItems: "center", gap: 10,
              background: i === 0 ? C.bg3 : "transparent",
              borderLeft: `2px solid ${i === 0 ? C.accent : "transparent"}`,
            }}>
              <span style={{ color: typeColors[c.type], fontSize: 11, width: 18 }}>{c.k}</span>
              <span style={{ color: C.muted, fontSize: 8, fontWeight: 700, letterSpacing: 2, width: 70 }}>{c.type.toUpperCase()}</span>
              <span style={{ color: C.fg, fontSize: 11, flex: 1 }}>{c.label}</span>
              {i === 0 && <span style={{ color: C.accent, fontSize: 9, fontWeight: 700, letterSpacing: 2 }}>↵</span>}
            </div>
          ))}
        </div>
        <div style={{ padding: "6px 16px", borderTop: `1px solid ${C.muted}`, background: C.bg, fontSize: 9, color: C.dim, display: "flex", gap: 14 }}>
          <span><span style={{ color: C.accentDim, fontWeight: 700 }}>↑↓</span> navigate</span>
          <span><span style={{ color: C.accentDim, fontWeight: 700 }}>↵</span> execute</span>
          <span><span style={{ color: C.accentDim, fontWeight: 700 }}>Esc</span> close</span>
          <span style={{ flex: 1 }} />
          <span style={{ color: C.muted }}>Ctrl+K · :</span>
        </div>
      </div>
    </div>
  );
};

/* ═══════════════════════════════════════════════════ */
/* MAIN COMPONENT                                       */
/* ═══════════════════════════════════════════════════ */

export default function ZeusProductionTUI() {
  const [activeTab, setActiveTab] = useState("chat");
  const [activeAdv, setActiveAdv] = useState(null);
  const [walletView, setWalletView] = useState("balance");
  const [walletSel, setWalletSel] = useState(0);
  const [paletteOpen, setPaletteOpen] = useState(false);

  // Chat state
  const [messages, setMessages] = useState([
    { role: "user", text: "Walk me through the production TUI prototype design — what's in each tab?" },
    { role: "assistant", text: "I built 8 primary tabs plus an Advanced submenu mirroring the historical S61 structure. Each backend feature now has a TUI surface. Here's the breakdown:", provider_badge: "claude-opus-4-7" },
    { role: "tool_call", tool: "memory_recall", args: '{"query": "S61 tab structure", "limit": 5}', status: "success", output: "5 facts retrieved:\n  • S61 design: 8 primary tabs + Advanced submenu\n  • Reverted in S78 cleanup, simplified to 4 tabs\n  • merakizzz directive 2026-05-03: restore the 8-tab structure\n  • All backend features must have TUI surface\n  • Advanced submenu = WebUI sidebar parity" },
    { role: "tool_call", tool: "read_file", args: '{"path": "crates/zeus-tui/src/app.rs", "limit": 50}', status: "success", output: "Line 1305: enum Tab { Chat, Office, Pantheon, Settings }\nLine 1310: impl Tab {\nLine 1311:   pub fn cycle_next(&self) -> Self {\nLine 1312:     match self {\nLine 1313:       Self::Chat => Self::Office,\nLine 1314:       Self::Office => Self::Pantheon,\nLine 1315:       Self::Pantheon => Self::Settings,\nLine 1316:       Self::Settings => Self::Chat,\nLine 1317:     }\nLine 1318:   }\nLine 1319: }" },
    { role: "tool_call", tool: "shell", args: '{"command": "grep -r \\"Tab::\\" crates/zeus-tui/src/ | wc -l"}', status: "success", output: "47" },
    { role: "assistant", text: "The current code has 4 tabs (chat / office / pantheon / settings) — a regression from the historical 23-screen design. Restoring the 8-primary structure means:\n\n• **chat** — primary work surface, message queue, slash overlay, live tool stream\n• **office** — pixel-art presence with agent focus panel\n• **pantheon** — mission list + war room + plan card approvals\n• **tools** — 365-tool browser with category nav and executor\n• **memory** — workspace tree + sessions + Mnemosyne semantic search\n• **channels** — 8 adapter management with status + test\n• **approvals** — Aegis pending tool exec queue\n• **settings** — 2-pane subsystem grouped editor\n\nPlus an Advanced submenu with 13 specialized views: agents, skills, MCP, projects, canvas, voice, nodecomms, vectorstores, economy, extensions, knowledge graph, spawner, deploy.", provider_badge: "claude-opus-4-7" },
  ]);
  const [queue, setQueue] = useState([]);
  const [input, setInput] = useState("");
  const [isStreaming, setIsStreaming] = useState(false);
  const [cookingIter, setCookingIter] = useState(3);
  const [cookingTools, setCookingTools] = useState(12);
  const [expandedMsg, setExpandedMsg] = useState(null);

  // Other tab state
  const [focusedAgent, setFocusedAgent] = useState("hermes");
  const [selectedMission, setSelectedMission] = useState("m1");
  const [selectedTool, setSelectedTool] = useState("read_file");
  const [toolFilter, setToolFilter] = useState("");
  const [selectedSettingsGroup, setSelectedSettingsGroup] = useState("llm");
  const [unreadByTab, setUnreadByTab] = useState({ pantheon: 1, channels: 3, approvals: 4 });

  // Keyboard handlers
  useEffect(() => {
    const handler = (e) => {
      const tag = e.target?.tagName;
      const isInput = tag === "INPUT" || tag === "TEXTAREA";

      // Ctrl+K palette
      if ((e.ctrlKey || e.metaKey) && e.key === "k") {
        e.preventDefault();
        setPaletteOpen(true);
        return;
      }
      // : palette
      if (e.key === ":" && !isInput) {
        e.preventDefault();
        setPaletteOpen(true);
        return;
      }
      if (paletteOpen && e.key === "Escape") {
        e.preventDefault();
        setPaletteOpen(false);
        return;
      }
      if (isInput) return;

      // Tab to cycle
      if (e.key === "Tab" && !e.shiftKey) {
        e.preventDefault();
        const idx = PRIMARY_TABS.findIndex(t => t.id === activeTab);
        const next = PRIMARY_TABS[(idx + 1) % PRIMARY_TABS.length];
        setActiveTab(next.id);
        if (next.id !== "advanced") setActiveAdv(null);
      }
      if (e.key === "Tab" && e.shiftKey) {
        e.preventDefault();
        const idx = PRIMARY_TABS.findIndex(t => t.id === activeTab);
        const prev = PRIMARY_TABS[(idx - 1 + PRIMARY_TABS.length) % PRIMARY_TABS.length];
        setActiveTab(prev.id);
        if (prev.id !== "advanced") setActiveAdv(null);
      }
      // Wallet tab navigation
      if (activeTab === "wallet" && !isInput) {
        const wv = ["balance", "send", "receive", "activity", "economy", "security"];
        if (e.key >= "1" && e.key <= "6") { setWalletView(wv[parseInt(e.key) - 1]); }
        if (e.key === "j" || e.key === "ArrowDown") setWalletSel(s => Math.min(WALLET_TITANS.length - 1, s + 1));
        if (e.key === "k" || e.key === "ArrowUp") setWalletSel(s => Math.max(0, s - 1));
        if (e.key === "Enter" && (walletView === "balance")) setWalletView("economy");
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [activeTab, paletteOpen, walletView]);

  const handleSubmit = () => {
    if (!input.trim()) return;
    if (isStreaming) {
      setQueue([...queue, { text: input, ts: Date.now() }]);
      setInput("");
    } else {
      setMessages([...messages, { role: "user", text: input }]);
      setInput("");
      setIsStreaming(true);
      setTimeout(() => {
        setMessages(m => [...m, { role: "assistant", text: "Got it. Let me work on that.", provider_badge: "claude-opus-4-7" }]);
        setIsStreaming(false);
      }, 2000);
    }
  };

  const handleCancelLast = () => {
    if (queue.length > 0) setQueue(queue.slice(0, -1));
  };

  const handleAction = (type, target) => {
    if (type === "tab") {
      setActiveTab(target);
      setActiveAdv(null);
    } else if (type === "adv") {
      setActiveTab("advanced");
      setActiveAdv(target);
    }
  };

  // Resolve content for active tab
  let content;
  let hints = [];
  let status = "";

  if (activeTab === "chat") {
    content = <ChatTab
      messages={messages} queue={queue} input={input} setInput={setInput}
      isStreaming={isStreaming} cookingIter={cookingIter} cookingTools={cookingTools} totalTools={147}
      expandedMsg={expandedMsg} setExpandedMsg={setExpandedMsg}
      onSubmit={handleSubmit} onCancelLast={handleCancelLast}
    />;
    hints = [
      { k: "↵", v: "send" },
      { k: "↑↓", v: "scroll" },
      { k: "Esc", v: "clear input / cancel queue" },
      { k: "Ctrl+L", v: "clear chat" },
      { k: "/", v: "commands" },
      { k: "e", v: "expand tool output" },
    ];
    status = isStreaming ? `cooking · iter ${cookingIter}/8 · ${cookingTools} tools` : "ready";
  } else if (activeTab === "office") {
    content = <OfficeTab focusedAgent={focusedAgent} setFocusedAgent={setFocusedAgent} />;
    hints = [
      { k: "f", v: "focus" },
      { k: "m", v: "memo" },
      { k: "R", v: "reconnect" },
      { k: "Esc", v: "clear focus" },
      { k: "?", v: "help" },
    ];
    status = "8 agents · 6 active · 2 idle · TPS 8";
  } else if (activeTab === "pantheon") {
    content = <PantheonTab selectedMission={selectedMission} setSelectedMission={setSelectedMission} />;
    hints = [
      { k: "n", v: "new mission" },
      { k: "p", v: "pause" },
      { k: "c", v: "cancel" },
      { k: "↵", v: "open" },
      { k: "a", v: "approve plan" },
      { k: "r", v: "redirect" },
    ];
    status = "6 missions · 3 active · 1 plan card pending";
  } else if (activeTab === "tools") {
    content = <ToolsTab selectedTool={selectedTool} setSelectedTool={setSelectedTool} toolFilter={toolFilter} setToolFilter={setToolFilter} />;
    hints = [
      { k: "/", v: "filter" },
      { k: "↵", v: "select" },
      { k: "x", v: "execute" },
      { k: "v", v: "view schema" },
      { k: "c", v: "category" },
    ];
    status = "365 tools · 14 categories";
  } else if (activeTab === "memory") {
    content = <MemoryTab />;
    hints = [
      { k: "↵", v: "open" },
      { k: "/", v: "search" },
      { k: "Tab", v: "switch view" },
      { k: "g", v: "go to graph" },
    ];
    status = "847 files · 147 sessions · 12,847 facts";
  } else if (activeTab === "channels") {
    content = <ChannelsTab />;
    hints = [
      { k: "t", v: "test" },
      { k: "e", v: "edit" },
      { k: "p", v: "pause" },
      { k: "r", v: "reconnect" },
    ];
    status = "5 connected · 1 reconnecting · 2 disconnected";
  } else if (activeTab === "wallet") {
    content = <WalletTab walletView={walletView} setWalletView={setWalletView} walletSel={walletSel} />;
    hints = [
      { k: "1-6", v: "view" },
      { k: "j/k", v: "navigate" },
      { k: "↵", v: "sign / open" },
      { k: "c", v: "copy addr" },
      { k: "r", v: "reveal phrase" },
    ];
    status = `184,920 ZEUS · 4,680 CR · ${WALLET_TITANS.length} titan wallets`;
  } else if (activeTab === "approvals") {
    content = <ApprovalsTab />;
    hints = [
      { k: "a", v: "approve" },
      { k: "d", v: "deny" },
      { k: "A", v: "approve all" },
      { k: "D", v: "deny all" },
      { k: "v", v: "view full" },
    ];
    status = "4 pending · 2 high risk · 1 medium · 1 low";
  } else if (activeTab === "settings") {
    content = <SettingsTab selectedGroup={selectedSettingsGroup} setSelectedGroup={setSelectedSettingsGroup} />;
    hints = [
      { k: "↑↓", v: "navigate" },
      { k: "↵", v: "edit" },
      { k: "?", v: "help" },
      { k: "Esc", v: "discard" },
    ];
    status = "1 unsaved change · 7 subsystems";
  } else if (activeTab === "advanced") {
    content = <AdvancedTab activeAdv={activeAdv} setActiveAdv={setActiveAdv} />;
    hints = [
      { k: "↵", v: "open" },
      { k: "↑↓", v: "navigate" },
      { k: "Esc", v: "back" },
    ];
    status = activeAdv ? `advanced · ${ADVANCED_TABS.find(t => t.id === activeAdv)?.name}` : "13 advanced subsystems";
  }

  return (
    <>
      <style>{`
        @import url('https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@300;400;500;700&display=swap');
        *, *::before, *::after { margin:0; padding:0; box-sizing:border-box; }
        body { background: ${C.bg}; overflow: hidden; }
        ::selection { background: ${C.accentDim}; color: ${C.white}; }
        input::placeholder, textarea::placeholder { color: ${C.muted}; }
        ::-webkit-scrollbar { width: 4px; height: 4px; }
        ::-webkit-scrollbar-track { background: transparent; }
        ::-webkit-scrollbar-thumb { background: ${C.muted}; border-radius: 2px; }
      `}</style>

      <div style={{
        fontFamily: "'JetBrains Mono', monospace", fontSize: 12, lineHeight: 1.5,
        color: C.fg, background: C.bg, height: "100vh",
        display: "flex", flexDirection: "column", overflow: "hidden",
        position: "relative",
      }}>
        <TopBar
          ctxPercent={47}
          hostname="zeus.local"
          model="claude-opus-4-7"
          gatewayVersion="0.4.7-rc.3"
          connState="connected"
        />
        <TabBar active={activeTab} setActive={(t) => { setActiveTab(t); if (t !== "advanced") setActiveAdv(null); }} unreadByTab={unreadByTab} pendingApprovals={4} />

        <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
          {content}
        </div>

        <HintBar hints={hints} status={status} queueCount={queue.length} />

        <CommandPalette open={paletteOpen} setOpen={setPaletteOpen} onAction={handleAction} />
      </div>
    </>
  );
}
