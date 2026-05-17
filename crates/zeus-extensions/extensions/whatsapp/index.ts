/**
 * WhatsApp Cloud API Channel Extension
 *
 * Sends and receives messages via the Meta Graph API (WhatsApp Business).
 * Requires: access_token, phone_number_id in config.
 */

const GRAPH_API = "https://graph.facebook.com/v21.0";

interface WhatsAppConfig {
  access_token: string;
  phone_number_id: string;
  verify_token?: string;
  webhook_port?: number;
}

interface OutboundMessage {
  channel: string;
  target: string;
  text: string;
  media?: string[];
  reply_to?: string;
}

// Accumulated inbound messages (populated by webhook listener)
const inboundQueue: any[] = [];

const plugin = {
  id: "whatsapp",
  name: "WhatsApp Cloud API",

  capabilities: {
    send: true,
    receive: true,
    media: true,
    reply: true,
    thread: false,
    typing: false,
  },

  outbound: {
    async send(msg: OutboundMessage, config: WhatsAppConfig) {
      const url = `${GRAPH_API}/${config.phone_number_id}/messages`;

      const body: any = {
        messaging_product: "whatsapp",
        to: msg.target,
        type: "text",
        text: { body: msg.text },
      };

      // If media is provided, send as image/document
      if (msg.media && msg.media.length > 0) {
        body.type = "image";
        body.image = { link: msg.media[0] };
        delete body.text;
      }

      // Reply context
      if (msg.reply_to) {
        body.context = { message_id: msg.reply_to };
      }

      const resp = await fetch(url, {
        method: "POST",
        headers: {
          "Authorization": `Bearer ${config.access_token}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify(body),
      });

      if (!resp.ok) {
        const errText = await resp.text();
        throw new Error(`WhatsApp API ${resp.status}: ${errText}`);
      }

      const result = await resp.json();
      return {
        sent: true,
        message_id: result.messages?.[0]?.id ?? null,
      };
    },
  },

  inbound: {
    poll() {
      const events = [...inboundQueue];
      inboundQueue.length = 0;
      return events;
    },

    // Parse webhook payload from WhatsApp Cloud API
    processWebhook(payload: any) {
      const entries = payload?.entry ?? [];
      for (const entry of entries) {
        const changes = entry?.changes ?? [];
        for (const change of changes) {
          const messages = change?.value?.messages ?? [];
          for (const msg of messages) {
            inboundQueue.push({
              type: "message",
              channel: "whatsapp",
              from: msg.from,
              text: msg.text?.body ?? "",
              message_id: msg.id,
              media: msg.image ? [msg.image.id] : [],
            });
          }
        }
      }
    },
  },

  status: {
    async check(config: WhatsAppConfig) {
      // Verify token is valid by fetching phone number info
      const url = `${GRAPH_API}/${config.phone_number_id}`;
      try {
        const resp = await fetch(url, {
          headers: { "Authorization": `Bearer ${config.access_token}` },
        });
        if (resp.ok) {
          return { connected: true, issues: [] };
        }
        return { connected: false, error: `API returned ${resp.status}` };
      } catch (err: any) {
        return { connected: false, error: err.message };
      }
    },
  },

  register(api: any) {
    api.registerChannel({ plugin });
  },
};

export default plugin;
