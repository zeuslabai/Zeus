#!/usr/bin/env node
// Zeus WhatsApp Bridge — Baileys WebSocket bridge for Zeus gateway
//
// Usage:
//   npm install @whiskeysockets/baileys ws
//   node scripts/whatsapp-bridge.js
//
// Zeus connects to ws://localhost:3000 and receives:
//   { type: "qr", data: "..." }       — QR code to scan
//   { type: "ready" }                  — device already linked
//   { type: "message", data: {...} }   — incoming message
//
// Zeus sends:
//   { type: "send", to: "...", text: "..." } — outgoing message

const { default: makeWASocket, useMultiFileAuthState, DisconnectReason } = require("@whiskeysockets/baileys");
const { WebSocketServer } = require("ws");
const path = require("path");

// Render QR in terminal using unicode blocks (no extra dependency)
function renderQR(text) {
  try {
    const qr = require("qrcode-terminal");
    qr.generate(text, { small: true });
  } catch {
    // Fallback: just print the raw string for Zeus to pick up
    console.log("[bridge] QR data (scan this or paste into a QR generator):");
    console.log(text);
  }
}

const PORT = parseInt(process.env.BRIDGE_PORT || "3000", 10);
const AUTH_DIR = process.env.AUTH_DIR || path.join(process.env.HOME || "~", ".zeus", "whatsapp-auth");

let wss;
let sock;

function broadcast(obj) {
  const msg = JSON.stringify(obj);
  if (wss) {
    for (const client of wss.clients) {
      if (client.readyState === 1) client.send(msg);
    }
  }
}

async function startWhatsApp() {
  const { state, saveCreds } = await useMultiFileAuthState(AUTH_DIR);

  sock = makeWASocket({
    auth: state,
    printQRInTerminal: false,
  });

  sock.ev.on("creds.update", saveCreds);

  sock.ev.on("connection.update", (update) => {
    const { connection, lastDisconnect, qr } = update;

    if (qr) {
      console.log("[bridge] QR code generated — broadcasting to Zeus");
      console.log("[bridge] Scan this QR with WhatsApp → Linked Devices → Link a Device:\n");
      renderQR(qr);
      broadcast({ type: "qr", data: qr });
    }

    if (connection === "open") {
      console.log("[bridge] WhatsApp connected — device linked");
      broadcast({ type: "ready" });
    }

    if (connection === "close") {
      const reason = lastDisconnect?.error?.output?.statusCode;
      if (reason === DisconnectReason.loggedOut) {
        console.log("[bridge] Logged out — delete auth and re-run to re-pair");
        broadcast({ type: "error", data: "Logged out. Delete ~/.zeus/whatsapp-auth and restart." });
      } else {
        console.log(`[bridge] Disconnected (reason: ${reason}) — reconnecting...`);
        setTimeout(startWhatsApp, 3000);
      }
    }
  });

  sock.ev.on("messages.upsert", ({ messages }) => {
    for (const msg of messages) {
      if (msg.key.fromMe) continue; // skip own messages
      const text = msg.message?.conversation
        || msg.message?.extendedTextMessage?.text
        || "";
      if (!text) continue;

      const from = msg.key.remoteJid || "";
      const sender = msg.pushName || from.split("@")[0];
      console.log(`[bridge] Message from ${sender}: ${text.substring(0, 80)}`);

      broadcast({
        type: "message",
        data: {
          from,
          sender,
          text,
          timestamp: msg.messageTimestamp,
          id: msg.key.id,
        },
      });
    }
  });

  return sock;
}

// WebSocket server for Zeus
wss = new WebSocketServer({ port: PORT });
console.log(`[bridge] WebSocket server listening on ws://localhost:${PORT}`);

wss.on("connection", (ws) => {
  console.log("[bridge] Zeus connected");

  ws.on("message", async (raw) => {
    try {
      const msg = JSON.parse(raw.toString());
      if (msg.type === "send" && msg.to && msg.text && sock) {
        await sock.sendMessage(msg.to, { text: msg.text });
        console.log(`[bridge] Sent to ${msg.to}: ${msg.text.substring(0, 80)}`);
      }
    } catch (e) {
      console.error("[bridge] Error handling message:", e.message);
    }
  });

  ws.on("close", () => console.log("[bridge] Zeus disconnected"));
});

// Start WhatsApp
startWhatsApp().catch((e) => {
  console.error("[bridge] Failed to start:", e.message);
  process.exit(1);
});
