import { useState, useEffect, useRef, useCallback } from "react";

/* ═══ RETRO RPG PALETTE ═══ */
const P = {
  plank1:"#3d2b1a", plank2:"#4a3422", plank3:"#352418", plank4:"#483020",
  plankHi:"#5a3e2a", plankLine:"#2a1e12",
  tile1:"#2e3638", tile2:"#343e40",
  carpet1:"#4a2028", carpet2:"#3e1a22", carpet3:"#562830",
  wall1:"#2a2420", wall2:"#322c28", wall3:"#3a3430", wallTrim:"#4a3a2e",
  wallpaper1:"#2e2822", wallpaper2:"#342e28",
  deskTop:"#6a5238", deskFront:"#5a4430", deskSide:"#4a3828", deskLeg:"#3a2a1a",
  shelf1:"#5a4230", shelf2:"#4a3828", shelf3:"#6a5440",
  crtScreen:"#1a3048", crt2:"#0e1824",
  crtText1:"#40c080", crtText2:"#30a060", crtText3:"#60e0a0",
  crtFrame:"#4a4a5a", crtBase:"#3a3a4a", led1:"#40ff80",
  chairSeat:"#6a3a2a", chairBack:"#5a3020", chairWheel:"#2a2a2a",
  sofa1:"#7a4a32", sofa2:"#6a3e2a", sofa3:"#8a5a3e", sofaPillow:"#9a6a48",
  leaf1:"#2d6828", leaf2:"#3a7a30", leaf3:"#1e5a1a", leaf4:"#4a8a38",
  pot1:"#7a4a2a", pot2:"#6a3e22",
  lampShade:"#d4a040", lampGlow:"#ffd880", lampPost:"#5a4a3a", warmGlow1:"#3a2a18",
  book1:"#a03020", book2:"#2050a0", book3:"#20a050", book4:"#a0a020",
  book5:"#8030a0", book6:"#a06020", book7:"#206080",
  mug1:"#e0d0c0", mug2:"#c0b0a0", coffee:"#3a1e0e",
  paper:"#d8d0c0",
  frame1:"#5a4a3a", frame2:"#4a3a2a",
  whiteboard:"#d0d8e0", wbBorder:"#8a8a9a", wbText:"#3a4a5a",
  clock:"#d0c8b8", clockHand:"#2a2a2a",
  cooler1:"#a0b8c8", cooler2:"#8aa0b0", waterBlue:"#60a0d0",
  machine1:"#4a4a4a", machine2:"#3a3a3a", machineBtn:"#ff3c14",
  poster1:"#a04040", poster2:"#4040a0",
  bg:"#0a0a0f", fg:"#d4cfc8", dim:"#5a5650", muted:"#3a3632",
  accent:"#ff3c14", accentDim:"#a0301a",
  green:"#22c55e", yellow:"#eab308", blue:"#3b82f6", cyan:"#06b6d4", red:"#ef4444",
  white:"#f0ece6", warmBg:"#12100e",
};

const _ = null;

/* ═══ BUILD OFFICE (96x48 pixel grid) ═══ */
const buildOffice = () => {
  const W = 96, H = 48;
  const g = Array.from({ length: H }, () => Array(W).fill(P.plank1));
  const rect = (x1,y1,x2,y2,c) => { for(let y=y1;y<=y2;y++) for(let x=x1;x<=x2;x++) if(y>=0&&y<H&&x>=0&&x<W) g[y][x]=c; };
  const px = (x,y,c) => { if(y>=0&&y<H&&x>=0&&x<W) g[y][x]=c; };

  // Floor planks
  for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
    const pi = Math.floor((x + (y % 2) * 3) / 6) % 4;
    g[y][x] = [P.plank1, P.plank2, P.plank3, P.plank4][pi];
    if ((x + (y % 2) * 3) % 6 === 0) g[y][x] = P.plankLine;
    if ((x * 7 + y * 13) % 97 === 0) g[y][x] = P.plankHi;
  }

  // Walls
  for (let y = 0; y < 6; y++) for (let x = 0; x < W; x++) {
    if (y < 2) g[y][x] = P.wall1;
    else if (y < 4) g[y][x] = ((x+y)%8 < 4) ? P.wallpaper1 : P.wallpaper2;
    else if (y === 4) g[y][x] = P.wall3;
    else g[y][x] = P.wallTrim;
  }

  // Carpet (break room)
  for (let y = 30; y < 44; y++) for (let x = 56; x < 90; x++) {
    const edge = x===56||x===89||y===30||y===43;
    g[y][x] = edge ? P.carpet3 : ((x+y)%2===0 ? P.carpet1 : P.carpet2);
  }

  // Tile (kitchen)
  for (let y = 30; y < 44; y++) for (let x = 2; x < 22; x++) {
    g[y][x] = ((x+y)%2===0) ? P.tile1 : P.tile2;
  }

  // ── ENGINEERING (3 desks with CRTs) ──
  for (let d = 0; d < 3; d++) {
    const dx = 4 + d * 14;
    rect(dx,12,dx+10,12,P.deskTop); rect(dx,13,dx+10,13,P.deskFront); rect(dx,14,dx+10,14,P.deskSide);
    px(dx,15,P.deskLeg); px(dx+10,15,P.deskLeg);
    // CRT
    rect(dx+2,8,dx+8,8,P.crtFrame);
    px(dx+1,9,P.crtFrame); rect(dx+2,9,dx+8,9,P.crtScreen); px(dx+9,9,P.crtFrame);
    px(dx+1,10,P.crtFrame); rect(dx+2,10,dx+8,10,P.crtScreen); px(dx+9,10,P.crtFrame);
    px(dx+1,11,P.crtFrame); rect(dx+2,11,dx+8,11,P.crtScreen); px(dx+9,11,P.crtFrame);
    rect(dx+2,12,dx+8,12,P.crtFrame);
    // Green screen text
    px(dx+3,9,P.crtText1); px(dx+5,9,P.crtText2); px(dx+7,9,P.crtText1);
    px(dx+3,10,P.crtText2); px(dx+4,10,P.crtText3); px(dx+6,10,P.crtText1);
    px(dx+3,11,P.crtText1); px(dx+5,11,P.crtText3); px(dx+8,11,P.led1);
    // Chair
    rect(dx+4,18,dx+6,18,P.chairSeat); rect(dx+4,19,dx+6,19,P.chairBack);
    px(dx+3,20,P.chairWheel); px(dx+7,20,P.chairWheel);
    px(dx+1,12,P.paper); px(dx+9,12,P.mug1);
  }

  // Whiteboard
  rect(10,2,30,2,P.wbBorder); rect(10,3,30,4,P.whiteboard); rect(10,5,30,5,P.wbBorder);
  for (let x = 12; x < 29; x += 3) { px(x,3,P.wbText); px(x+1,3,P.wbText); }
  px(28,4,P.accent); px(29,4,P.blue);

  // ── COMMS (dual monitors) ──
  for (let d = 0; d < 2; d++) {
    const dx = 54 + d * 18;
    rect(dx,12,dx+14,12,P.deskTop); rect(dx,13,dx+14,13,P.deskFront); rect(dx,14,dx+14,14,P.deskSide);
    for (let m = 0; m < 2; m++) {
      const mx = dx + 1 + m * 7;
      rect(mx,8,mx+5,8,P.crtFrame);
      px(mx-1,9,P.crtFrame); rect(mx,9,mx+5,9,P.crt2); px(mx+6,9,P.crtFrame);
      px(mx-1,10,P.crtFrame); rect(mx,10,mx+5,10,P.crt2); px(mx+6,10,P.crtFrame);
      px(mx-1,11,P.crtFrame); rect(mx,11,mx+5,11,P.crt2); px(mx+6,11,P.crtFrame);
      rect(mx,12,mx+5,12,P.crtFrame);
      px(mx+1,9,P.blue); px(mx+3,10,P.cyan); px(mx+2,11,P.blue); px(mx+5,11,P.led1);
    }
    rect(dx+5,18,dx+9,18,P.chairSeat); rect(dx+5,19,dx+9,19,P.chairBack);
  }
  // Posters
  rect(60,2,64,4,P.frame1); rect(61,3,63,3,P.poster2);
  rect(70,2,74,4,P.frame2); rect(71,3,73,3,P.poster1);

  // ── RESEARCH (bookshelf + L-desk) ──
  for (let y = 24; y < 38; y++) { rect(24,y,28,y,P.shelf1); if(y%3===0) rect(24,y,28,y,P.shelf2); }
  const bc = [P.book1,P.book2,P.book3,P.book4,P.book5,P.book6,P.book7];
  for (let y = 25; y < 37; y++) if(y%3!==0) for(let x=25;x<28;x++) px(x,y,bc[(x*3+y*7)%bc.length]);

  rect(32,30,46,30,P.deskTop); rect(32,31,46,31,P.deskFront);
  rect(46,30,50,30,P.deskTop); rect(46,31,50,31,P.deskFront);
  // CRT on research desk
  rect(35,27,41,27,P.crtFrame);
  rect(34,28,42,28,P.crtFrame); rect(35,28,41,28,P.crtScreen);
  rect(34,29,42,29,P.crtFrame); rect(35,29,41,29,P.crtScreen);
  rect(35,30,41,30,P.crtFrame);
  px(36,28,P.crtText3); px(38,28,P.crtText1); px(36,29,P.crtText2); px(39,29,P.crtText1);
  px(43,30,P.paper); px(47,30,P.paper);
  rect(37,34,39,34,P.chairSeat); rect(37,35,39,35,P.chairBack);

  // ── BREAK ROOM (sofa + table) ──
  rect(62,32,82,32,P.sofa3); rect(62,33,82,33,P.sofa1); rect(62,34,82,34,P.sofa2);
  rect(62,35,66,35,P.sofa2); rect(78,35,82,35,P.sofa2);
  px(65,33,P.sofaPillow); px(70,33,P.sofaPillow); px(75,33,P.sofaPillow); px(80,33,P.sofaPillow);
  rect(68,37,76,37,P.shelf3); rect(68,38,76,38,P.deskFront);
  px(70,37,P.mug1); px(71,37,P.coffee); px(74,37,P.mug2);

  // ── KITCHEN ──
  rect(4,32,8,32,P.machine1); rect(4,33,8,33,P.machine2); rect(4,34,8,34,P.machine1);
  px(6,33,P.machineBtn); px(7,33,P.led1); px(5,32,P.mug1);
  rect(12,31,15,31,P.cooler1); rect(12,32,15,32,P.cooler2); rect(12,33,15,33,P.cooler1);
  px(13,31,P.waterBlue); px(14,31,P.waterBlue);
  rect(2,36,20,36,P.deskTop); rect(2,37,20,37,P.deskFront);

  // ── DECOR ──
  const plant = (x,y) => {
    px(x,y-3,P.leaf2); px(x+1,y-3,P.leaf1);
    px(x-1,y-2,P.leaf3); px(x,y-2,P.leaf1); px(x+1,y-2,P.leaf4); px(x+2,y-2,P.leaf2);
    px(x-1,y-1,P.leaf4); px(x,y-1,P.leaf2); px(x+1,y-1,P.leaf1); px(x+2,y-1,P.leaf3);
    px(x,y,P.pot1); px(x+1,y,P.pot1); px(x,y+1,P.pot2); px(x+1,y+1,P.pot2);
  };
  plant(2,9); plant(42,9); plant(50,9); plant(88,9); plant(22,28); plant(52,40);

  const lamp = (x,y) => {
    px(x,y-1,P.lampShade); px(x+1,y-1,P.lampShade);
    px(x,y,P.lampGlow); px(x+1,y,P.lampGlow); px(x,y+1,P.lampPost);
    for(let dy=0;dy<3;dy++) for(let dx=-1;dx<3;dx++) {
      const gy=y+2+dy, gx=x+dx;
      if(gy<H&&gx>=0&&gx<W) g[gy][gx]=P.warmGlow1;
    }
  };
  lamp(3,8); lamp(40,8); lamp(88,8);

  // Ceiling lights
  for (const lx of [10,26,42,58,74]) for(let x=lx;x<lx+8;x++) { px(x,0,P.lampGlow); px(x,1,"#e0d8c0"); }

  // Clock
  rect(44,2,48,2,P.frame1); rect(44,3,48,4,P.clock); px(46,3,P.clockHand); px(47,3,P.clockHand); px(46,4,P.clockHand);

  // Zone dividers
  for(let y=6;y<H;y++) if(y%2===0) { px(48,y,P.wallTrim); px(49,y,P.wallTrim); }
  for(let x=0;x<W;x++) if(x%2===0) { px(x,22,P.wallTrim); px(x,23,P.wallTrim); }

  return g;
};

/* ═══ SPRITE (10x14 pixels) ═══ */
const CHARS = {
  zeus: { hair:"#2a1208",skin:"#e8c8a0",eye:"#1a1a2e",shirt:"#a03018",shirt2:"#802010",belt:"#3a2a1a",pants:"#1a1a2e",pants2:"#14142a",shoe:"#1a1208",badge:"#ff3c14" },
  hermes: { hair:"#4a3828",skin:"#d8b890",eye:"#1a3048",shirt:"#2a5a8a",shirt2:"#1a4a7a",belt:"#2a2a3a",pants:"#1a2a3a",pants2:"#142430",shoe:"#1a1208",badge:"#3b82f6" },
  athena: { hair:"#6a4a2a",skin:"#e0c0a0",eye:"#2d5a28",shirt:"#2d6828",shirt2:"#1e5a1a",belt:"#3a3a1a",pants:"#1a2a1a",pants2:"#142a14",shoe:"#1a1208",badge:"#06b6d4" },
  prometheus: { hair:"#3a2818",skin:"#d8b088",eye:"#4a3a10",shirt:"#6a5a10",shirt2:"#5a4a08",belt:"#4a3a2a",pants:"#2a2a1a",pants2:"#242418",shoe:"#1a1208",badge:"#eab308" },
};

const mkSprite = (c, frame = 0) => {
  const { hair,skin,eye,shirt,shirt2,belt,pants,pants2,shoe,badge } = c;
  const a = frame % 4 < 2;
  return [
    [_,_,_,hair,hair,hair,hair,_,_,_],
    [_,_,hair,hair,hair,hair,hair,hair,_,_],
    [_,hair,hair,hair,hair,hair,hair,hair,hair,_],
    [_,hair,skin,skin,skin,skin,skin,skin,hair,_],
    [_,_,skin,eye,skin,skin,eye,skin,_,_],
    [_,_,skin,skin,skin,skin,skin,skin,_,_],
    [_,_,_,skin,skin,skin,skin,_,_,_],
    [_,_,shirt,shirt,badge,shirt,shirt,shirt,_,_],
    [_,a?skin:shirt,shirt,shirt2,shirt,shirt,shirt2,shirt,a?_:skin,_],
    [_,_,shirt,shirt,shirt,shirt,shirt,shirt,_,_],
    [_,_,belt,belt,belt,belt,belt,belt,_,_],
    [_,_,pants,pants,_,_,pants,pants,_,_],
    [_,_,pants2,pants2,_,_,pants2,pants2,_,_],
    [_,_,shoe,a?_:shoe,_,a?shoe:_,shoe,_,_,_],
  ];
};

/* ═══ GAME STATE ═══ */
const ZONES = { engineering:{x:18,y:20}, comms:{x:64,y:20}, research:{x:38,y:36}, breakroom:{x:72,y:37}, kitchen:{x:10,y:38} };
const S2Z = { idle:"breakroom", writing:"engineering", executing:"engineering", researching:"research", syncing:"comms", error:"research" };
const SC = { idle:P.dim, writing:P.accent, executing:P.green, researching:P.cyan, syncing:P.blue, error:P.red };
const SL = { idle:"IDLE", writing:"WRITING", executing:"EXEC", researching:"RESEARCH", syncing:"SYNC", error:"ERROR" };
const TK = {
  idle:["Coffee break ☕","Stretching 🧘","Snack time 🍪","Water cooler 💧"],
  writing:["Writing SOUL.md","Drafting PR #42","Updating docs","Composing email"],
  executing:["cargo build --release","cargo test --workspace","shell: deploy.sh","git push main","docker compose up"],
  researching:["LLM accuracy benchmark","Paper review","Vector tuning","Embedding compare"],
  syncing:["Telegram: 12 msgs","Discord: #ops-alerts","Slack notification","Matrix sync"],
};

const INIT = [
  { id:"zeus", name:"Zeus Prime", zone:"engineering", state:"executing", x:18, y:20, tx:18, ty:20, frame:0, task:"cargo build --release", model:"claude-sonnet-4", char:CHARS.zeus },
  { id:"hermes", name:"Hermes", zone:"comms", state:"syncing", x:64, y:20, tx:64, ty:20, frame:0, task:"Telegram: 47 msgs sent", model:"gpt-4o", char:CHARS.hermes },
  { id:"athena", name:"Athena", zone:"research", state:"researching", x:38, y:36, tx:38, ty:36, frame:0, task:"LLM accuracy benchmark", model:"llama-3.3-70b", char:CHARS.athena },
  { id:"prometheus", name:"Prometheus", zone:"breakroom", state:"idle", x:72, y:37, tx:72, ty:37, frame:0, task:"Coffee break ☕", model:"claude-sonnet-4", char:CHARS.prometheus },
];

const MEMO = {
  date: "Yesterday, March 27",
  lines: [
    { w:"Zeus Prime", c:P.accent, t:"Deployed NovaTradeEngine v0.4.2 to staging." },
    { w:"", c:P.dim, t:"  Binary 12.2MB, 24/24 tests passed." },
    { w:"Hermes", c:P.blue, t:"Sent 147 messages across Telegram + Discord." },
    { w:"", c:P.dim, t:"  Release notes → #ops-alerts channel." },
    { w:"Athena", c:P.cyan, t:"Completed LLM benchmark (8 models tested)." },
    { w:"", c:P.dim, t:"  claude-sonnet-4 leads reasoning (94.2%)." },
    { w:"Prometheus", c:P.yellow, t:"Ran 8 heartbeat checks. All green." },
    { w:"", c:P.dim, t:"  Next maintenance: 03:00 UTC Saturday." },
  ],
};

/* ═══ HALF-BLOCK RENDERER ═══ */
const HB = ({ grid }) => {
  const rows = [];
  for (let y = 0; y < grid.length; y += 2) {
    const cells = [];
    const row = grid[y];
    const rowB = grid[y + 1];
    for (let x = 0; x < (row ? row.length : 0); x++) {
      const t = row ? row[x] : null;
      const b = rowB ? rowB[x] : null;
      cells.push(
        <span key={x} style={{ color: t || b || "transparent", backgroundColor: b || "transparent" }}>
          {"\u2580"}
        </span>
      );
    }
    rows.push(
      <div key={y} style={{ height: 18, lineHeight: "18px", whiteSpace: "pre", letterSpacing: 0 }}>
        {cells}
      </div>
    );
  }
  return <div>{rows}</div>;
};

/* ═══ MAIN ═══ */
export default function TheOffice() {
  const [agents, setAgents] = useState(INIT);
  const [tick, setTick] = useState(0);
  const [showMemo, setShowMemo] = useState(false);
  const [showHelp, setShowHelp] = useState(false);
  const [focus, setFocus] = useState(null);
  const [events, setEvents] = useState([]);
  const bgRef = useRef(buildOffice());
  const tickRef = useRef(0);

  // Game loop (8 TPS)
  useEffect(() => {
    const iv = setInterval(() => {
      setTick(t => t + 1);
      tickRef.current++;
      setAgents(prev => prev.map(ag => {
        const n = { ...ag, frame: ag.frame + 1 };
        // Random state change
        if (tickRef.current % (12 + Math.floor(Math.random() * 8)) === 0) {
          const states = ["idle","writing","executing","researching","syncing"];
          const ns = states[Math.floor(Math.random() * states.length)];
          const nz = S2Z[ns];
          const tgt = ZONES[nz];
          n.state = ns;
          n.zone = nz;
          n.tx = tgt.x + Math.floor(Math.random() * 12) - 6;
          n.ty = tgt.y + Math.floor(Math.random() * 4) - 2;
          const tl = TK[ns];
          n.task = tl[Math.floor(Math.random() * tl.length)];
          setEvents(ev => [{
            ts: new Date().toLocaleTimeString("en-US", { hour12: false }),
            agent: ag.name, msg: "\u2192 " + SL[ns] + ": " + n.task, color: SC[ns]
          }, ...ev].slice(0, 20));
        }
        // Move toward target
        const dx = n.tx - n.x, dy = n.ty - n.y;
        if (Math.abs(dx) > 0.5) n.x += Math.sign(dx) * Math.min(2, Math.abs(dx));
        if (Math.abs(dy) > 0.5) n.y += Math.sign(dy) * Math.min(1, Math.abs(dy));
        return n;
      }));
    }, 125);
    return () => clearInterval(iv);
  }, []);

  // Keyboard
  useEffect(() => {
    const h = (e) => {
      if (e.key === "m") setShowMemo(p => !p);
      if (e.key === "?") setShowHelp(p => !p);
      if (e.key === "Escape") { setShowMemo(false); setShowHelp(false); setFocus(null); }
      if (e.key === "Tab") {
        e.preventDefault();
        setFocus(f => {
          const ids = INIT.map(a => a.id);
          return ids[(ids.indexOf(f) + 1) % ids.length];
        });
      }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, []);

  // Compose scene
  const compose = useCallback(() => {
    const s = bgRef.current.map(r => [...r]);
    const sorted = [...agents].sort((a, b) => a.y - b.y);
    for (const ag of sorted) {
      const sp = mkSprite(ag.char, ag.frame);
      const sx = Math.round(ag.x);
      const sy = Math.round(ag.y) - sp.length;
      for (let r = 0; r < sp.length; r++) {
        for (let c = 0; c < sp[r].length; c++) {
          const pixel = sp[r][c];
          if (pixel && sy + r >= 0 && sy + r < s.length && sx + c >= 0 && sx + c < s[0].length) {
            s[sy + r][sx + c] = pixel;
          }
        }
      }
    }
    return s;
  }, [agents]);

  const scene = compose();
  const ts = new Date().toLocaleTimeString("en-US", { hour12: false });

  return (
    <>
      <style>{`
        @import url('https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@300;400;500;700&display=swap');
        *, *::before, *::after { margin:0; padding:0; box-sizing:border-box; }
        body { background: ${P.bg}; overflow: hidden; }
        ::selection { background: ${P.accentDim}; color: ${P.white}; }
      `}</style>

      <div style={{ fontFamily: "'JetBrains Mono', monospace", fontSize: 12, lineHeight: "18px", color: P.fg, background: P.bg, height: "100vh", display: "flex", flexDirection: "column", overflow: "hidden" }}>

        {/* TOP BAR */}
        <div style={{ height: 22, background: P.warmBg, borderBottom: `1px solid ${P.muted}`, display: "flex", alignItems: "center", padding: "0 10px", gap: 6, flexShrink: 0 }}>
          <span style={{ color: P.accent, fontWeight: 700, fontSize: 9, letterSpacing: 3 }}>ZEUS</span>
          <span style={{ color: P.muted }}>{"\u2502"}</span>
          <span style={{ color: P.accentDim, fontSize: 9, fontWeight: 700 }}>THE OFFICE</span>
          <span style={{ color: P.muted }}>{"\u2502"}</span>
          <span style={{ color: P.dim, fontSize: 9 }}>Retro Pixel-Art Fleet Dashboard</span>
          <span style={{ flex: 1 }} />
          <span style={{ color: P.green, fontSize: 8 }}>{"\u25CF"}</span>
          <span style={{ color: P.dim, fontSize: 9 }}>WS:OK</span>
          <span style={{ color: P.muted }}>{"\u2502"}</span>
          <span style={{ color: P.dim, fontSize: 9 }}>30 FPS</span>
          <span style={{ color: P.muted }}>{"\u2502"}</span>
          <span style={{ color: P.dim, fontSize: 9 }}>8 TPS</span>
        </div>

        <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
          {/* SCENE */}
          <div style={{ flex: 1, position: "relative", overflow: "hidden" }}>
            <HB grid={scene} />

            {/* Speech bubbles */}
            {agents.map(ag => {
              const bx = Math.round(ag.x) * 7.5 + 4;
              const by = (Math.round(ag.y) - 16) * 9 + 22;
              const isFoc = focus === ag.id;
              return (
                <div key={ag.id + "b"} style={{
                  position: "absolute",
                  left: Math.max(4, bx - 50),
                  top: Math.max(22, by),
                  padding: "2px 7px",
                  background: isFoc ? "rgba(255,60,20,0.15)" : "rgba(10,10,15,0.92)",
                  border: `1px solid ${isFoc ? P.accent : P.muted}`,
                  borderRadius: 3,
                  fontSize: 8,
                  zIndex: 10,
                  pointerEvents: "none",
                  whiteSpace: "nowrap",
                  maxWidth: 200,
                }}>
                  <span style={{ fontWeight: 700, color: SC[ag.state] }}>{ag.name}</span>
                  <span style={{ color: P.muted }}> {"\u00B7"} </span>
                  <span style={{ color: P.dim }}>{ag.task}</span>
                </div>
              );
            })}

            {/* Zone labels */}
            {[
              { n: "\u26A1 ENGINEERING", x: "1%", y: "14%", c: P.accent },
              { n: "\uD83D\uDCE1 COMMS", x: "58%", y: "14%", c: P.blue },
              { n: "\uD83D\uDD2C RESEARCH", x: "26%", y: "52%", c: P.cyan },
              { n: "\u2615 BREAK ROOM", x: "60%", y: "62%", c: P.yellow },
              { n: "\uD83C\uDF73 KITCHEN", x: "1%", y: "64%", c: P.dim },
            ].map(z => (
              <div key={z.n} style={{ position: "absolute", left: z.x, top: z.y, fontSize: 7, fontWeight: 700, letterSpacing: 2, color: z.c, opacity: 0.35, pointerEvents: "none" }}>{z.n}</div>
            ))}
          </div>

          {/* SIDEBAR */}
          <div style={{ width: 220, background: P.warmBg, borderLeft: `1px solid ${P.muted}`, display: "flex", flexDirection: "column", flexShrink: 0 }}>
            <div style={{ padding: "6px 8px", borderBottom: `1px solid ${P.muted}` }}>
              <span style={{ fontSize: 8, fontWeight: 700, letterSpacing: 3, color: P.accentDim }}>FLEET STATUS</span>
            </div>
            <div style={{ overflow: "auto", padding: "2px 4px", flex: 1 }}>
              {agents.map(ag => (
                <div key={ag.id} onClick={() => setFocus(ag.id)} style={{
                  padding: "5px 6px", borderRadius: 3, marginBottom: 1, cursor: "pointer",
                  background: focus === ag.id ? "rgba(255,60,20,0.06)" : "transparent",
                  borderLeft: `2px solid ${focus === ag.id ? P.accent : "transparent"}`,
                }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
                    <span style={{ width: 5, height: 5, borderRadius: 3, background: SC[ag.state], boxShadow: `0 0 4px ${SC[ag.state]}`, flexShrink: 0 }} />
                    <span style={{ fontSize: 10, fontWeight: focus === ag.id ? 700 : 400, color: focus === ag.id ? P.white : P.fg, flex: 1 }}>{ag.name}</span>
                    <span style={{ fontSize: 7, fontWeight: 700, letterSpacing: 1, color: SC[ag.state], padding: "1px 3px", borderRadius: 2, background: SC[ag.state] + "15" }}>{SL[ag.state]}</span>
                  </div>
                  <div style={{ fontSize: 8, color: P.dim, marginTop: 1, paddingLeft: 9, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{ag.task}</div>
                  <div style={{ fontSize: 8, color: P.muted, paddingLeft: 9 }}>{ag.model}</div>
                </div>
              ))}
            </div>

            {/* Event log */}
            <div style={{ borderTop: `1px solid ${P.muted}`, padding: "6px 8px" }}>
              <span style={{ fontSize: 8, fontWeight: 700, letterSpacing: 3, color: P.accentDim }}>EVENT LOG</span>
            </div>
            <div style={{ overflow: "auto", maxHeight: 140, padding: "0 4px" }}>
              {events.slice(0, 8).map((ev, i) => (
                <div key={i} style={{ padding: "2px 4px", fontSize: 8, borderLeft: `1px solid ${i === 0 ? ev.color : "transparent"}`, background: i === 0 ? ev.color + "08" : "transparent" }}>
                  <span style={{ color: P.muted }}>{ev.ts}</span>
                  <span style={{ color: ev.color, fontWeight: 500 }}> {ev.agent}</span>
                  <span style={{ color: P.dim }}> {ev.msg}</span>
                </div>
              ))}
            </div>

            {/* Zones */}
            <div style={{ borderTop: `1px solid ${P.muted}`, padding: "6px 8px" }}>
              <span style={{ fontSize: 8, fontWeight: 700, letterSpacing: 3, color: P.accentDim }}>ZONES</span>
              {Object.keys(ZONES).map(z => {
                const cnt = agents.filter(a => a.zone === z).length;
                const zc = z === "engineering" ? P.accent : z === "comms" ? P.blue : z === "research" ? P.cyan : z === "breakroom" ? P.yellow : P.dim;
                return (
                  <div key={z} style={{ display: "flex", alignItems: "center", gap: 4, padding: "1px 0" }}>
                    <span style={{ width: 4, height: 4, borderRadius: 2, background: zc }} />
                    <span style={{ fontSize: 9, color: P.dim, flex: 1, textTransform: "capitalize" }}>{z}</span>
                    <span style={{ fontSize: 9, color: cnt > 0 ? zc : P.muted }}>{cnt}</span>
                  </div>
                );
              })}
            </div>
          </div>
        </div>

        {/* MEMO OVERLAY */}
        {showMemo && (
          <div style={{ position: "absolute", top: 50, left: "50%", transform: "translateX(-50%)", width: 480, background: P.warmBg, border: `1px solid ${P.muted}`, borderRadius: 4, zIndex: 100 }}>
            <div style={{ padding: "6px 10px", borderBottom: `1px solid ${P.muted}`, display: "flex" }}>
              <span style={{ color: P.accent, fontWeight: 700, fontSize: 9, letterSpacing: 2 }}>{"\uD83D\uDCCB"} YESTERDAY'S MEMO</span>
              <span style={{ flex: 1 }} />
              <span style={{ color: P.dim, fontSize: 8 }}>{MEMO.date}</span>
              <span onClick={() => setShowMemo(false)} style={{ color: P.dim, cursor: "pointer", marginLeft: 8 }}>{"\u2715"}</span>
            </div>
            <div style={{ padding: "8px 12px" }}>
              {MEMO.lines.map((l, i) => (
                <div key={i} style={{ fontSize: 9, lineHeight: "15px", color: l.w ? P.fg : P.dim }}>
                  {l.w && <span style={{ color: l.c, fontWeight: 700 }}>{l.w}: </span>}
                  <span>{l.t}</span>
                </div>
              ))}
            </div>
            <div style={{ padding: "4px 12px 8px", borderTop: `1px solid ${P.muted}` }}>
              <span style={{ fontSize: 8, color: P.muted }}>Generated by Mnemosyne at 00:00 UTC</span>
            </div>
          </div>
        )}

        {/* HELP */}
        {showHelp && (
          <div style={{ position: "absolute", top: 50, right: 240, width: 220, background: P.warmBg, border: `1px solid ${P.muted}`, borderRadius: 4, zIndex: 100, padding: "8px 10px" }}>
            <div style={{ fontSize: 9, fontWeight: 700, letterSpacing: 2, color: P.accentDim, marginBottom: 6 }}>CONTROLS</div>
            {[["M","Yesterday's memo"],["Tab","Cycle agent focus"],["?","Toggle help"],["Esc","Close / unfocus"]].map(([k, v]) => (
              <div key={k} style={{ display: "flex", gap: 6, padding: "2px 0" }}>
                <span style={{ fontSize: 9, color: P.accentDim, fontWeight: 700, width: 26 }}>{k}</span>
                <span style={{ fontSize: 9, color: P.dim }}>{v}</span>
              </div>
            ))}
          </div>
        )}

        {/* STATUS BAR */}
        <div style={{ height: 20, background: P.warmBg, borderTop: `1px solid ${P.muted}`, display: "flex", alignItems: "center", padding: "0 10px", gap: 8, flexShrink: 0 }}>
          <span style={{ color: P.green, fontSize: 8 }}>{"\u25CF"}</span>
          <span style={{ color: P.dim, fontSize: 9 }}>the-office</span>
          <span style={{ color: P.muted }}>{"\u2502"}</span>
          <span style={{ color: P.dim, fontSize: 9 }}>{agents.length} agents</span>
          <span style={{ color: P.muted }}>{"\u2502"}</span>
          <span style={{ color: P.dim, fontSize: 9 }}>{agents.filter(a => a.state !== "idle").length} active</span>
          <span style={{ color: P.muted }}>{"\u2502"}</span>
          <span style={{ color: P.dim, fontSize: 9 }}>tick {tick}</span>
          <span style={{ flex: 1 }} />
          <span style={{ color: P.accentDim, fontSize: 9, fontWeight: 700 }}>M</span>
          <span style={{ color: P.dim, fontSize: 9 }}>Memo</span>
          <span style={{ color: P.muted }}>{"\u2502"}</span>
          <span style={{ color: P.accentDim, fontSize: 9, fontWeight: 700 }}>Tab</span>
          <span style={{ color: P.dim, fontSize: 9 }}>Focus</span>
          <span style={{ color: P.muted }}>{"\u2502"}</span>
          <span style={{ color: P.accentDim, fontSize: 9, fontWeight: 700 }}>?</span>
          <span style={{ color: P.dim, fontSize: 9 }}>Help</span>
          <span style={{ color: P.muted }}>{"\u2502"}</span>
          <span style={{ color: P.dim, fontSize: 9 }}>{ts}</span>
        </div>
      </div>
    </>
  );
}
