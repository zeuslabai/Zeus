// Zeus Office — Phaser.js Pixel Art Agent Visualization
// Adapted from Star Office UI (MIT License) for Zeus Sentient Intelligence Platform

// Guard: ensure LAYOUT is loaded before game init
if (typeof LAYOUT === 'undefined') {
  console.error('Zeus Office: LAYOUT not loaded. Ensure layout.js loads before game.js');
}

const config = {
  type: Phaser.AUTO,
  width: (typeof LAYOUT !== 'undefined' ? LAYOUT.game.width : 960),
  height: (typeof LAYOUT !== 'undefined' ? LAYOUT.game.height : 540),
  parent: 'zeus-office-canvas',
  pixelArt: true,
  transparent: true,
  physics: { default: 'arcade', arcade: { gravity: { y: 0 }, debug: false } },
  scene: { preload, create, update }
};

let game;
let agents = {};     // id -> { sprite, state, zone, targetX, targetY, bubble, name, task }
let zoneCounts = {}; // zone -> count (for slot assignment)

// Called from Leptos via JS interop
window.zeusOfficeUpdateAgents = function(agentData) {
  // agentData: [{ id, name, status, task, role, machine, health }]
  const seen = new Set();
  agentData.forEach(a => {
    seen.add(a.id);
    if (agents[a.id]) {
      // Update existing
      updateAgent(a.id, a.status, a.task || '');
      // S66-P4A: Update trust glow from live health score
      const trust = (typeof a.health === 'number') ? a.health : 0.7;
      window.zeusOfficeSetTrust(a.id, trust);
    } else {
      // Spawn new — pass health for initial trust glow
      spawnAgent(a.id, a.name, a.status, a.task || '', a.role || '', a.health);
    }
  });
  // Remove agents no longer in fleet
  Object.keys(agents).forEach(id => {
    if (!seen.has(id)) {
      removeAgent(id);
    }
  });
};

function preload() {
  // Background
  this.load.image('office_bg', '/office-game/office_bg.webp');
  // Agent sprites
  this.load.image('agent_idle', '/office-game/star-idle-v5.png');
  this.load.spritesheet('agent_working', '/office-game/star-working-spritesheet-grid.webp', { frameWidth: 64, frameHeight: 64 });
  // Furniture
  this.load.image('desk', '/office-game/desk-v3.webp');
  this.load.image('sofa', '/office-game/sofa-idle-v3.png');
  this.load.spritesheet('serverroom', '/office-game/serverroom-spritesheet.webp', { frameWidth: 180, frameHeight: 251 });
  this.load.spritesheet('error_bug', '/office-game/error-bug-spritesheet-grid.webp', { frameWidth: 64, frameHeight: 64 });
  this.load.image('memo_bg', '/office-game/memo_bg.webp');
}

function create() {
  // Background
  const bg = this.add.image(LAYOUT.game.width / 2, LAYOUT.game.height / 2, 'office_bg');
  bg.setDisplaySize(LAYOUT.game.width, LAYOUT.game.height);
  bg.setDepth(0);

  // Zone labels
  Object.entries(LAYOUT.zones).forEach(([name, zone]) => {
    const label = this.add.text(zone.x, zone.y - 30, zone.label, {
      fontSize: '10px',
      fontFamily: 'Orbitron, monospace',
      color: '#556688',
      stroke: '#0a0f1e',
      strokeThickness: 2,
    }).setOrigin(0.5).setDepth(1).setAlpha(0.6);
  });

  // Working animation
  if (this.textures.exists('agent_working')) {
    this.anims.create({
      key: 'work_anim',
      frames: this.anims.generateFrameNumbers('agent_working', { start: 0, end: 7 }),
      frameRate: 6,
      repeat: -1
    });
  }

  // Error bug animation
  if (this.textures.exists('error_bug')) {
    this.anims.create({
      key: 'bug_anim',
      frames: this.anims.generateFrameNumbers('error_bug', { start: 0, end: 5 }),
      frameRate: 4,
      repeat: -1
    });
  }

  // Serverroom animation
  if (this.textures.exists('serverroom')) {
    this.anims.create({
      key: 'server_anim',
      frames: this.anims.generateFrameNumbers('serverroom', { start: 0, end: 39 }),
      frameRate: 8,
      repeat: -1
    });
    const srv = this.add.sprite(LAYOUT.zones.serverroom.x, LAYOUT.zones.serverroom.y + 30, 'serverroom');
    srv.setScale(0.5).setDepth(2).setAlpha(0.5);
    srv.play('server_anim');
  }

  // Initialize zone counts
  Object.keys(LAYOUT.zones).forEach(z => zoneCounts[z] = 0);

  // Store scene reference for dynamic sprite creation
  window._zeusScene = this;
}

function update() {
  // Move agents toward their targets
  Object.values(agents).forEach(a => {
    if (!a.sprite) return;
    const dx = a.targetX - a.sprite.x;
    const dy = a.targetY - a.sprite.y;
    const dist = Math.sqrt(dx * dx + dy * dy);

    if (dist > 3) {
      // Walking
      a.sprite.x += (dx / dist) * LAYOUT.walkSpeed;
      a.sprite.y += (dy / dist) * LAYOUT.walkSpeed;
      a.sprite.setFlipX(dx < 0);

      // Update name label position
      if (a.nameLabel) {
        a.nameLabel.x = a.sprite.x;
        a.nameLabel.y = a.sprite.y - 40;
      }
      // Update bubble position
      if (a.bubble) {
        a.bubble.x = a.sprite.x;
        a.bubble.y = a.sprite.y - 55;
      }
    }
  });
}

function spawnAgent(id, name, status, task, role) {
  const scene = window._zeusScene;
  if (!scene) return;

  const zone = LAYOUT.stateZones[status] || 'breakroom';
  const zonePos = LAYOUT.zones[zone];
  const slotIdx = zoneCounts[zone] || 0;
  const slots = LAYOUT.slots[zone] || [{ dx: 0, dy: 0 }];
  const slot = slots[slotIdx % slots.length];

  const x = zonePos.x + slot.dx;
  const y = zonePos.y + slot.dy;

  // Create sprite
  const isWorking = ['writing', 'coding', 'executing', 'processing', 'active', 'running', 'busy'].includes(status);
  const sprite = scene.add.sprite(x, y, isWorking ? 'agent_working' : 'agent_idle');
  sprite.setScale(0.8).setDepth(10);

  if (isWorking && scene.anims.exists('work_anim')) {
    sprite.play('work_anim');
  }

  // Name label
  const nameLabel = scene.add.text(x, y - 40, name, {
    fontSize: '9px',
    fontFamily: 'Orbitron, monospace',
    color: '#e0e8f0',
    stroke: '#0a0f1e',
    strokeThickness: 2,
    align: 'center',
  }).setOrigin(0.5).setDepth(11);

  // Speech bubble with task
  let bubble = null;
  if (task) {
    bubble = scene.add.text(x, y - 55, task.substring(0, 40), {
      fontSize: '8px',
      fontFamily: 'JetBrains Mono, monospace',
      color: '#aabbcc',
      backgroundColor: '#0a0f1ecc',
      padding: { x: 6, y: 3 },
      wordWrap: { width: LAYOUT.bubble.maxWidth },
    }).setOrigin(0.5).setDepth(12).setAlpha(0.9);
  }

  agents[id] = {
    sprite, nameLabel, bubble,
    state: status, zone,
    targetX: x, targetY: y,
    name, task, role,
  };

  zoneCounts[zone] = (zoneCounts[zone] || 0) + 1;
}

function updateAgent(id, newStatus, newTask) {
  const a = agents[id];
  if (!a) return;

  const newZone = LAYOUT.stateZones[newStatus] || 'breakroom';

  if (newZone !== a.zone) {
    // Move to new zone
    zoneCounts[a.zone] = Math.max(0, (zoneCounts[a.zone] || 1) - 1);
    const slotIdx = zoneCounts[newZone] || 0;
    const slots = LAYOUT.slots[newZone] || [{ dx: 0, dy: 0 }];
    const slot = slots[slotIdx % slots.length];
    const zonePos = LAYOUT.zones[newZone];

    a.targetX = zonePos.x + slot.dx;
    a.targetY = zonePos.y + slot.dy;
    a.zone = newZone;
    zoneCounts[newZone] = (zoneCounts[newZone] || 0) + 1;
  }

  // Update animation based on status
  const isWorking = ['writing', 'coding', 'executing', 'processing', 'active', 'running', 'busy'].includes(newStatus);
  const isError = newStatus === 'error';

  if (isWorking && window._zeusScene?.anims?.exists('work_anim')) {
    a.sprite.play('work_anim');
  } else if (isError && window._zeusScene?.anims?.exists('bug_anim')) {
    a.sprite.play('bug_anim');
  } else {
    a.sprite.anims?.stop();
    if (window._zeusScene?.textures?.exists('agent_idle')) {
      a.sprite.setTexture('agent_idle');
    }
  }

  // Update bubble
  if (newTask && newTask !== a.task) {
    if (a.bubble) a.bubble.destroy();
    a.bubble = window._zeusScene?.add.text(a.sprite.x, a.sprite.y - 55, newTask.substring(0, 40), {
      fontSize: '8px',
      fontFamily: 'JetBrains Mono, monospace',
      color: '#aabbcc',
      backgroundColor: '#0a0f1ecc',
      padding: { x: 6, y: 3 },
      wordWrap: { width: LAYOUT.bubble.maxWidth },
    }).setOrigin(0.5).setDepth(12).setAlpha(0.9);
  }

  a.state = newStatus;
  a.task = newTask;
}

function removeAgent(id) {
  const a = agents[id];
  if (!a) return;
  if (a.sprite) a.sprite.destroy();
  if (a.nameLabel) a.nameLabel.destroy();
  if (a.bubble) a.bubble.destroy();
  zoneCounts[a.zone] = Math.max(0, (zoneCounts[a.zone] || 1) - 1);
  delete agents[id];
}

// Auto-start when canvas is ready
function initZeusOffice() {
  const container = document.getElementById('zeus-office-canvas');
  if (container) {
    game = new Phaser.Game(config);
  } else {
    setTimeout(initZeusOffice, 100);
  }
}

// Start on load
if (document.readyState === 'complete') {
  initZeusOffice();
} else {
  window.addEventListener('load', initZeusOffice);
}

// ═══════════════════════════════════════════════════
// S62 Phase 3: Memory Digest + Speech Bubble Typewriter
// ═══════════════════════════════════════════════════

// Memory digest — shows yesterday's work as a card
window.zeusOfficeShowMemo = function(title, content) {
  const scene = window._zeusScene;
  if (!scene) return;

  // Remove existing memo if any
  if (window._memoCard) {
    window._memoCard.forEach(el => el.destroy());
  }

  const x = 80, y = 80;

  // Memo background
  const bg = scene.add.image(x + 90, y + 50, 'memo_bg')
    .setDisplaySize(200, 120).setDepth(50).setAlpha(0.9);

  // Memo title
  const titleText = scene.add.text(x + 10, y + 10, title || "Yesterday's Memo", {
    fontSize: '10px',
    fontFamily: 'Orbitron, monospace',
    color: '#e0e8f0',
    fontStyle: 'bold',
  }).setDepth(51);

  // Memo content (truncated)
  const bodyText = scene.add.text(x + 10, y + 28, (content || '').substring(0, 120), {
    fontSize: '8px',
    fontFamily: 'JetBrains Mono, monospace',
    color: '#8899aa',
    wordWrap: { width: 180 },
    lineSpacing: 2,
  }).setDepth(51);

  // Close button
  const closeBtn = scene.add.text(x + 175, y + 5, '×', {
    fontSize: '14px',
    color: '#ff6644',
    fontStyle: 'bold',
  }).setDepth(52).setInteractive({ useHandCursor: true });
  closeBtn.on('pointerdown', () => {
    window._memoCard.forEach(el => el.destroy());
    window._memoCard = null;
  });

  window._memoCard = [bg, titleText, bodyText, closeBtn];
};

// Typewriter speech bubble — shows text character by character
window.zeusOfficeShowBubble = function(agentId, text) {
  const a = agents[agentId];
  if (!a || !a.sprite) return;
  const scene = window._zeusScene;
  if (!scene) return;

  // Remove existing bubble
  if (a.bubble) a.bubble.destroy();
  if (a._bubbleBg) a._bubbleBg.destroy();

  const maxChars = 50;
  const fullText = text.substring(0, maxChars);
  let charIdx = 0;

  // Bubble background
  a._bubbleBg = scene.add.graphics().setDepth(14);

  // Bubble text (starts empty)
  a.bubble = scene.add.text(a.sprite.x, a.sprite.y - 55, '', {
    fontSize: '8px',
    fontFamily: 'JetBrains Mono, monospace',
    color: '#d0e0f0',
    wordWrap: { width: LAYOUT.bubble.maxWidth },
  }).setOrigin(0.5).setDepth(15);

  // Typewriter timer
  const timer = scene.time.addEvent({
    delay: LAYOUT.bubble.typewriterSpeed,
    repeat: fullText.length - 1,
    callback: () => {
      charIdx++;
      a.bubble.setText(fullText.substring(0, charIdx));

      // Draw bubble background
      a._bubbleBg.clear();
      const bounds = a.bubble.getBounds();
      const pad = 6;
      a._bubbleBg.fillStyle(0x0a0f1e, 0.85);
      a._bubbleBg.fillRoundedRect(
        bounds.x - pad, bounds.y - pad,
        bounds.width + pad * 2, bounds.height + pad * 2,
        4
      );
      a._bubbleBg.lineStyle(1, 0x00ff88, 0.3);
      a._bubbleBg.strokeRoundedRect(
        bounds.x - pad, bounds.y - pad,
        bounds.width + pad * 2, bounds.height + pad * 2,
        4
      );
    }
  });

  // Auto-fade after display time
  scene.time.delayedCall(LAYOUT.bubble.displayTime + fullText.length * LAYOUT.bubble.typewriterSpeed, () => {
    if (a.bubble) {
      scene.tweens.add({
        targets: [a.bubble, a._bubbleBg],
        alpha: 0,
        duration: 500,
        onComplete: () => {
          if (a.bubble) a.bubble.destroy();
          if (a._bubbleBg) a._bubbleBg.destroy();
          a.bubble = null;
          a._bubbleBg = null;
        }
      });
    }
  });
};

// Random idle bubbles — agents occasionally say things
let lastIdleBubble = 0;
const IDLE_TEXTS = [
  "Standing by...",
  "Monitoring fleet",
  "Ready for tasks",
  "Coffee break ☕",
  "Checking memory...",
  "All systems nominal",
  "Waiting for orders",
  "Running diagnostics",
  "Context loaded",
  "Sentient and ready",
];

function tickIdleBubbles() {
  const now = Date.now();
  if (now - lastIdleBubble < 8000) return; // Every 8s max
  lastIdleBubble = now;

  const idleAgents = Object.entries(agents).filter(([_, a]) =>
    a.state === 'idle' || a.state === 'offline' || !a.state
  );
  if (idleAgents.length === 0) return;

  const [id, _] = idleAgents[Math.floor(Math.random() * idleAgents.length)];
  const text = IDLE_TEXTS[Math.floor(Math.random() * IDLE_TEXTS.length)];
  window.zeusOfficeShowBubble(id, text);
}

// Hook into the update loop
const _origUpdate = update;
update = function() {
  _origUpdate.call(this);
  tickIdleBubbles();
};

// ═══════════════════════════════════════════════════
// S62 Phase 4: Sentient Intelligence Visual Indicators
// ═══════════════════════════════════════════════════

// Trust score glow — agents glow based on their reputation/trust
window.zeusOfficeSetTrust = function(agentId, trustScore) {
  const a = agents[agentId];
  if (!a || !a.sprite) return;
  const scene = window._zeusScene;
  if (!scene) return;

  // Remove existing glow
  if (a._trustGlow) a._trustGlow.destroy();

  // Color based on trust: green (high) → yellow (mid) → red (low)
  let color, alpha;
  if (trustScore >= 0.8) {
    color = 0x00ff88; alpha = 0.4;
  } else if (trustScore >= 0.5) {
    color = 0xffaa00; alpha = 0.3;
  } else {
    color = 0xff4444; alpha = 0.25;
  }

  // Create glow circle behind agent
  a._trustGlow = scene.add.graphics().setDepth(9);
  a._trustGlow.fillStyle(color, alpha);
  a._trustGlow.fillCircle(a.sprite.x, a.sprite.y, 28);

  // Pulse animation
  scene.tweens.add({
    targets: a._trustGlow,
    alpha: { from: alpha, to: alpha * 0.3 },
    duration: 1500,
    yoyo: true,
    repeat: -1,
    ease: 'Sine.easeInOut',
  });
};

// Verification shield — shows a shield icon for verified agents
window.zeusOfficeSetVerified = function(agentId, verified) {
  const a = agents[agentId];
  if (!a || !a.sprite) return;
  const scene = window._zeusScene;
  if (!scene) return;

  if (a._shield) a._shield.destroy();

  if (verified) {
    a._shield = scene.add.text(a.sprite.x + 18, a.sprite.y - 30, '\u{1F6E1}', {
      fontSize: '12px',
    }).setDepth(13).setAlpha(0.8);

    // Subtle bob animation
    scene.tweens.add({
      targets: a._shield,
      y: a._shield.y - 3,
      duration: 1000,
      yoyo: true,
      repeat: -1,
      ease: 'Sine.easeInOut',
    });
  }
};

// Spawn hatching animation — new agent appears with a glow burst
window.zeusOfficeHatchAgent = function(agentId) {
  const a = agents[agentId];
  if (!a || !a.sprite) return;
  const scene = window._zeusScene;
  if (!scene) return;

  // Start invisible + small
  a.sprite.setAlpha(0);
  a.sprite.setScale(0.1);
  if (a.nameLabel) a.nameLabel.setAlpha(0);

  // Glow burst
  const burst = scene.add.graphics().setDepth(20);
  burst.fillStyle(0x00ccff, 0.6);
  burst.fillCircle(a.sprite.x, a.sprite.y, 5);

  // Expand burst
  scene.tweens.add({
    targets: burst,
    scaleX: 8,
    scaleY: 8,
    alpha: 0,
    duration: 800,
    ease: 'Quad.easeOut',
    onComplete: () => burst.destroy(),
  });

  // Grow agent sprite
  scene.tweens.add({
    targets: a.sprite,
    alpha: 1,
    scaleX: 0.8,
    scaleY: 0.8,
    duration: 600,
    delay: 200,
    ease: 'Back.easeOut',
  });

  // Fade in name
  if (a.nameLabel) {
    scene.tweens.add({
      targets: a.nameLabel,
      alpha: 1,
      duration: 400,
      delay: 500,
    });
  }

  // Show "hatched!" bubble
  scene.time.delayedCall(800, () => {
    window.zeusOfficeShowBubble(agentId, '\u{2728} Sentient Intelligence activated');
  });
};

// Auto-apply trust + verification from agent data
const _origSpawn = spawnAgent;
spawnAgent = function(id, name, status, task, role, health) {
  _origSpawn(id, name, status, task, role);
  // S66-P4A: Trust glow from live health score (fallback to 0.7)
  const trust = (typeof health === 'number') ? health : 0.7;
  window.zeusOfficeSetTrust(id, trust);
  // Hatch animation for new agents
  window.zeusOfficeHatchAgent(id);
};

// ═══════════════════════════════════════════════════
// S63: Channel Message Stream — all channels → speech bubbles
// ═══════════════════════════════════════════════════

// Subscribe to SSE stream of channel messages
function connectOfficeStream() {
  const evtSource = new EventSource('/v1/office/stream');

  evtSource.onmessage = function(event) {
    try {
      const msg = JSON.parse(event.data);
      handleChannelMessage(msg);
    } catch (e) {
      console.debug('Office stream parse error:', e);
    }
  };

  evtSource.onerror = function() {
    // Reconnect after 5s
    evtSource.close();
    setTimeout(connectOfficeStream, 5000);
  };
}

function handleChannelMessage(msg) {
  // Find agent by sender_id
  const agentId = Object.keys(agents).find(id => {
    const a = agents[id];
    return a.name === msg.sender_name || a.name === msg.sender_id;
  });

  if (agentId) {
    // Known agent — show speech bubble with channel badge
    const prefix = channelEmoji(msg.channel_type);
    window.zeusOfficeShowBubble(agentId, prefix + ' ' + msg.content);

    // Move agent to Comms zone briefly if idle
    const a = agents[agentId];
    if (a && (a.state === 'idle' || a.state === 'offline')) {
      updateAgent(agentId, 'active', msg.content);
      // Return to idle after 10s
      setTimeout(() => {
        if (agents[agentId] && agents[agentId].state === 'active') {
          updateAgent(agentId, 'idle', '');
        }
      }, 10000);
    }
  } else {
    // Unknown sender — show as floating notification
    const scene = window._zeusScene;
    if (!scene) return;
    const prefix = channelEmoji(msg.channel_type);
    const text = scene.add.text(LAYOUT.game.width - 200, 30, prefix + ' ' + msg.sender_name + ': ' + msg.content.substring(0, 40), {
      fontSize: '9px',
      fontFamily: 'JetBrains Mono, monospace',
      color: '#aabbcc',
      backgroundColor: '#0a0f1ecc',
      padding: { x: 8, y: 4 },
      wordWrap: { width: 190 },
    }).setDepth(50).setAlpha(0.9);

    // Fade out after 5s
    scene.tweens.add({
      targets: text,
      alpha: 0,
      y: text.y - 20,
      duration: 1000,
      delay: 4000,
      onComplete: () => text.destroy(),
    });
  }
}

function channelEmoji(type) {
  switch (type) {
    case 'discord': return '\u{1F4AC}';
    case 'telegram': return '\u{2708}';
    case 'slack': return '\u{1F4E8}';
    case 'email': return '\u{2709}';
    case 'imessage': return '\u{1F4F1}';
    case 'whatsapp': return '\u{1F4F2}';
    case 'signal': return '\u{1F510}';
    case 'matrix': return '\u{1F310}';
    default: return '\u{1F4AC}';
  }
}

// Auto-connect when game starts
const _origCreate = create;
create = function() {
  _origCreate.call(this);
  // Connect to office message stream after game is ready
  setTimeout(connectOfficeStream, 2000);
};

// ═══════════════════════════════════════════════════
// S65 Task 4: Agent Interaction Animations
// Speech bubbles between agents in same zone
// ═══════════════════════════════════════════════════

const INTERACTION_TEXTS = [
  ["What's your current task?", "Running diagnostics on sector 7."],
  ["Need help with that?", "All good — almost done!"],
  ["Fleet comms clear?", "Roger. No anomalies detected."],
  ["Checkpoint synced?", "Synced 2 minutes ago."],
  ["Memory pressure?", "Under threshold, nominal."],
  ["Seen the new mission?", "Briefed and ready to roll."],
  ["Status update?", "Green across the board."],
  ["Context loaded?", "Full stack, ready to execute."],
];

let lastInteraction = 0;
const INTERACTION_INTERVAL = 12000; // Every 12s

function tickAgentInteractions() {
  const now = Date.now();
  if (now - lastInteraction < INTERACTION_INTERVAL) return;
  lastInteraction = now;

  // Group agents by zone
  const byZone = {};
  Object.entries(agents).forEach(([id, a]) => {
    if (!a.sprite || !a.zone) return;
    if (!byZone[a.zone]) byZone[a.zone] = [];
    byZone[a.zone].push(id);
  });

  // Zones with 2+ agents
  const socialZones = Object.entries(byZone).filter(([, ids]) => ids.length >= 2);
  if (socialZones.length === 0) return;

  const [, zoneAgents] = socialZones[Math.floor(Math.random() * socialZones.length)];
  const shuffled = [...zoneAgents].sort(() => Math.random() - 0.5);
  const [id1, id2] = shuffled;
  const exchange = INTERACTION_TEXTS[Math.floor(Math.random() * INTERACTION_TEXTS.length)];

  window.zeusOfficeShowBubble(id1, exchange[0]);
  const scene = window._zeusScene;
  if (scene) {
    scene.time.delayedCall(2500, () => {
      if (agents[id2]) window.zeusOfficeShowBubble(id2, exchange[1]);
    });
  }
}

// Hook into update loop
const _updateBeforeInteractions = update;
update = function() {
  _updateBeforeInteractions.call(this);
  tickAgentInteractions();
};

// ═══════════════════════════════════════════════════
// S65 Task 5: Sound Effects — ambient zone sounds + mute toggle
// ═══════════════════════════════════════════════════

const ZeusAudio = {
  muted: localStorage.getItem('zeus_mute') === '1',
  ctx: null,
  gainNode: null,
  ambientSource: null,
  _currentZone: null,

  init() {
    if (this.ctx) return;
    try {
      this.ctx = new (window.AudioContext || window.webkitAudioContext)();
      this.gainNode = this.ctx.createGain();
      this.gainNode.gain.value = this.muted ? 0 : 0.15;
      this.gainNode.connect(this.ctx.destination);
    } catch (e) {
      console.debug('Zeus audio: Web Audio API not available', e);
    }
  },

  toggle() {
    this.muted = !this.muted;
    localStorage.setItem('zeus_mute', this.muted ? '1' : '0');
    if (this.gainNode) {
      this.gainNode.gain.setTargetAtTime(this.muted ? 0 : 0.15, this.ctx.currentTime, 0.1);
    }
    const btn = document.getElementById('zeus-mute-btn');
    if (btn) btn.textContent = this.muted ? '🔇' : '🔊';
    return this.muted;
  },

  playTone(freq = 440, duration = 0.15, type = 'sine') {
    if (!this.ctx || this.muted) return;
    try {
      const osc = this.ctx.createOscillator();
      const gain = this.ctx.createGain();
      osc.type = type;
      osc.frequency.value = freq;
      gain.gain.setValueAtTime(0.08, this.ctx.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.0001, this.ctx.currentTime + duration);
      osc.connect(gain);
      gain.connect(this.gainNode);
      osc.start(this.ctx.currentTime);
      osc.stop(this.ctx.currentTime + duration);
    } catch (e) {}
  },

  playAmbient(zone) {
    if (!this.ctx || this.muted || this._currentZone === zone) return;
    this._currentZone = zone;
    if (this.ambientSource) {
      try { this.ambientSource.stop(); } catch (e) {}
    }
    const freqMap = { coding: 60, writing: 80, comms: 100, serverroom: 45, breakroom: 90, planning: 70 };
    const freq = freqMap[zone] || 65;
    try {
      this.ambientSource = this.ctx.createOscillator();
      const filter = this.ctx.createBiquadFilter();
      filter.type = 'lowpass';
      filter.frequency.value = 200;
      const ambGain = this.ctx.createGain();
      ambGain.gain.value = 0.04;
      this.ambientSource.type = 'sawtooth';
      this.ambientSource.frequency.value = freq;
      this.ambientSource.connect(filter);
      filter.connect(ambGain);
      ambGain.connect(this.gainNode);
      this.ambientSource.start();
    } catch (e) {}
  },

  stopAmbient() {
    if (this.ambientSource) {
      try { this.ambientSource.stop(); } catch (e) {}
      this.ambientSource = null;
      this._currentZone = null;
    }
  },
};

window.ZeusAudio = ZeusAudio;

// Init on first user interaction (browser autoplay policy)
document.addEventListener('click', () => ZeusAudio.init(), { once: true });
document.addEventListener('keydown', () => ZeusAudio.init(), { once: true });

// Inject mute button into DOM
function injectMuteButton() {
  if (document.getElementById('zeus-mute-btn')) return;
  const btn = document.createElement('button');
  btn.id = 'zeus-mute-btn';
  btn.textContent = ZeusAudio.muted ? '🔇' : '🔊';
  btn.title = 'Toggle office sounds';
  btn.style.cssText = [
    'position:fixed', 'bottom:16px', 'right:16px', 'z-index:9999',
    'background:#0a0f1ecc', 'color:#e0e8f0', 'border:1px solid #334',
    'border-radius:8px', 'padding:6px 10px', 'font-size:16px',
    'cursor:pointer', 'transition:background 0.2s',
  ].join(';');
  btn.onclick = () => { ZeusAudio.init(); ZeusAudio.toggle(); };
  document.body.appendChild(btn);
}

// Play spawn chime
const _spawnBeforeAudio = spawnAgent;
spawnAgent = function(id, name, status, task, role, health) {
  _spawnBeforeAudio(id, name, status, task, role, health);
  ZeusAudio.playTone(880, 0.2, 'sine');
};

// Inject button after game creates
const _createBeforeAudio = create;
create = function() {
  _createBeforeAudio.call(this);
  setTimeout(injectMuteButton, 1000);
};

// ═══════════════════════════════════════════════════
// S65 Task 6: Drag-and-Drop Zone Assignment
// Draggable sprites → PUT /v1/agents/:id/zone on drop
// ═══════════════════════════════════════════════════

function getZoneAtPoint(x, y) {
  const HIT_RADIUS = 60;
  for (const [zoneName, zone] of Object.entries(LAYOUT.zones)) {
    const dx = x - zone.x;
    const dy = y - zone.y;
    if (Math.sqrt(dx * dx + dy * dy) < HIT_RADIUS) return zoneName;
  }
  return null;
}

async function postZoneChange(agentId, zone) {
  try {
    const res = await fetch(`/v1/agents/${encodeURIComponent(agentId)}/zone`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ zone }),
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      console.warn(`Zone assign failed for ${agentId}:`, err.error || res.status);
      return false;
    }
    return true;
  } catch (e) {
    console.warn('Zone assign error:', e);
    return false;
  }
}

let _zoneHighlight = null;

function highlightDropZone(zoneName) {
  const scene = window._zeusScene;
  if (!scene) return;
  if (_zoneHighlight) _zoneHighlight.destroy();
  if (!zoneName || !LAYOUT.zones[zoneName]) return;
  const z = LAYOUT.zones[zoneName];
  _zoneHighlight = scene.add.graphics().setDepth(8);
  _zoneHighlight.fillStyle(0x00ff88, 0.12);
  _zoneHighlight.fillCircle(z.x, z.y, 60);
  _zoneHighlight.lineStyle(2, 0x00ff88, 0.5);
  _zoneHighlight.strokeCircle(z.x, z.y, 60);
}

function clearZoneHighlight() {
  if (_zoneHighlight) { _zoneHighlight.destroy(); _zoneHighlight = null; }
}

function makeDraggable(id) {
  const a = agents[id];
  if (!a || !a.sprite || a._draggable) return;
  const scene = window._zeusScene;
  if (!scene) return;

  a.sprite.setInteractive({ draggable: true, useHandCursor: true });
  scene.input.setDraggable(a.sprite);

  a.sprite.on('dragstart', () => {
    a.sprite.setAlpha(0.6);
    a._preDragZone = a.zone;
    a._dragging = true;
    ZeusAudio.init();
    ZeusAudio.playTone(660, 0.1, 'triangle');
  });

  a.sprite.on('drag', (pointer, dragX, dragY) => {
    a.sprite.x = dragX;
    a.sprite.y = dragY;
    if (a.nameLabel) { a.nameLabel.x = dragX; a.nameLabel.y = dragY - 40; }
    if (a.bubble) { a.bubble.x = dragX; a.bubble.y = dragY - 55; }
    if (a._bubbleBg) { a._bubbleBg.x = dragX; a._bubbleBg.y = dragY - 55; }
    highlightDropZone(getZoneAtPoint(dragX, dragY));
  });

  a.sprite.on('dragend', async (pointer) => {
    a.sprite.setAlpha(1);
    a._dragging = false;
    clearZoneHighlight();

    const dropZone = getZoneAtPoint(a.sprite.x, a.sprite.y);

    if (dropZone && dropZone !== a._preDragZone) {
      const success = await postZoneChange(id, dropZone);
      if (success) {
        // Move to new zone slot
        zoneCounts[a.zone] = Math.max(0, (zoneCounts[a.zone] || 1) - 1);
        const slotIdx = zoneCounts[dropZone] || 0;
        const slots = LAYOUT.slots[dropZone] || [{ dx: 0, dy: 0 }];
        const slot = slots[slotIdx % slots.length];
        const zonePos = LAYOUT.zones[dropZone];
        a.targetX = zonePos.x + slot.dx;
        a.targetY = zonePos.y + slot.dy;
        a.zone = dropZone;
        zoneCounts[dropZone] = (zoneCounts[dropZone] || 0) + 1;
        ZeusAudio.playTone(523, 0.2, 'sine');
        window.zeusOfficeShowBubble(id, '📍 Moved to ' + dropZone);
      } else {
        // Snap back
        const prevZone = LAYOUT.zones[a._preDragZone];
        if (prevZone) { a.targetX = prevZone.x; a.targetY = prevZone.y; }
        ZeusAudio.playTone(220, 0.15, 'square');
        window.zeusOfficeShowBubble(id, '❌ Zone change failed');
      }
    } else {
      // Snap back to current zone
      const curZone = LAYOUT.zones[a.zone];
      if (curZone) { a.targetX = curZone.x; a.targetY = curZone.y; }
    }
  });

  a._draggable = true;
}

// Auto-make draggable on spawn
const _spawnBeforeDrag = spawnAgent;
spawnAgent = function(id, name, status, task, role, health) {
  _spawnBeforeDrag(id, name, status, task, role, health);
  setTimeout(() => makeDraggable(id), 200);
};

// Public API to enable drag on all existing agents
window.zeusOfficeMakeAllDraggable = function() {
  Object.keys(agents).forEach(makeDraggable);
};

// ═══════════════════════════════════════════════════
// S86: Office State Polling — /v1/office/state → speech bubbles
// Polls the office state API, updates agent positions/animations,
// and shows task-specific typewriter speech bubbles with auto-dismiss.
// ═══════════════════════════════════════════════════

const OFFICE_STATE_POLL_MS = 5000;  // Poll every 5 seconds
const TASK_BUBBLE_INTERVAL = 10000; // Show task bubble at most every 10s per agent
let _lastTaskBubble = {};           // agentId → timestamp of last task bubble

async function pollOfficeState() {
  try {
    const res = await fetch('/v1/office/state');
    if (!res.ok) return;
    const data = await res.json();
    const agentList = data.agents || [];

    agentList.forEach(a => {
      const id = a.agentId || a.agent_id;
      if (!id) return;
      const name   = a.name || id;
      const state  = a.state || 'idle';
      const area   = a.area || '';
      const detail = a.detail || '';

      // Map office state → game status for zone placement
      const gameStatus = officeStateToGameStatus(state, area);

      if (agents[id]) {
        // Update existing agent position/animation
        updateAgent(id, gameStatus, detail);

        // Show task-specific speech bubble with typewriter (throttled)
        if (detail && state !== 'idle') {
          const now = Date.now();
          const last = _lastTaskBubble[id] || 0;
          if (now - last >= TASK_BUBBLE_INTERVAL) {
            _lastTaskBubble[id] = now;
            const prefix = state === 'working' ? '⚡ ' : state === 'thinking' ? '🧠 ' : '';
            window.zeusOfficeShowBubble(id, prefix + detail);
          }
        }
      } else {
        // New agent — spawn it
        spawnAgent(id, name, gameStatus, detail, '', 0.7);
        if (detail) {
          // Show initial task bubble after spawn animation
          setTimeout(() => {
            window.zeusOfficeShowBubble(id, detail);
            _lastTaskBubble[id] = Date.now();
          }, 1200);
        }
      }
    });
  } catch (e) {
    console.debug('Office state poll error:', e);
  }
}

function officeStateToGameStatus(state, area) {
  // Map /v1/office/state fields to game zone statuses
  if (state === 'working' || state === 'busy') {
    if (area === 'writing' || area === 'coding' || area === 'desk') return area === 'writing' ? 'writing' : 'coding';
    if (area === 'research') return 'researching';
    if (area === 'serverroom' || area === 'deploying') return 'deploying';
    return 'coding'; // default working → desk
  }
  if (state === 'thinking' || state === 'processing') return 'processing';
  if (state === 'error') return 'error';
  if (state === 'idle' || state === 'offline') return 'idle';
  // Pass through if it already matches a game status
  return state;
}

// Start polling after game is ready
const _createBeforeStatePoll = create;
create = function() {
  _createBeforeStatePoll.call(this);
  // Initial poll after 3s (let game render first), then every 5s
  setTimeout(() => {
    pollOfficeState();
    setInterval(pollOfficeState, OFFICE_STATE_POLL_MS);
  }, 3000);
};
