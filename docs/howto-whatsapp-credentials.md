# How to Get WhatsApp Credentials

Zeus connects to WhatsApp via the **WhatsApp Cloud API** — Meta's official Business messaging API. You'll need a Meta developer account and a WhatsApp Business phone number.

## Prerequisites

- A Facebook/Meta account
- A phone number not already registered with WhatsApp (or use the free test number Meta provides)

## Step 1: Create a Meta Developer App

1. Go to [developers.facebook.com](https://developers.facebook.com) and log in
2. Click **My Apps → Create App**
3. Select **Business** as the app type
4. Fill in the app name and contact email, then click **Create App**

## Step 2: Add WhatsApp to Your App

1. On the app dashboard, find **WhatsApp** in the product list and click **Set up**
2. You'll be taken to the **WhatsApp API Setup** page

## Step 3: Get Your Phone Number ID

1. On the **API Setup** page, look for the **From** section under "Send and receive messages"
2. You'll see a phone number listed — click it to reveal the **Phone Number ID** below it
3. Copy the **Phone Number ID** (it's a long numeric string like `123456789012345`)

> ⚠️ This is the Phone Number ID — **not** the actual phone number. They look different.

## Step 4: Get Your Access Token

1. On the same **API Setup** page, scroll to **Temporary access token**
2. Click **Generate** to create a token
3. Copy the token — it starts with `EAA...`

> 📌 Temporary tokens expire after ~24 hours. For production, generate a **permanent token** via a System User in Meta Business Manager: Business Settings → Users → System Users → Add → Generate New Token.

## Step 5: Set Up a Webhook

Zeus needs to receive incoming WhatsApp messages via webhook.

1. On the **Configuration** tab, find **Webhooks**
2. Set the **Callback URL** to: `https://YOUR_GATEWAY_URL/v1/webhooks/whatsapp`
3. Set a **Verify Token** — any string you choose (e.g. `zeus-verify-123`)
4. Click **Verify and Save**
5. Subscribe to the **messages** webhook field

> 💡 Your gateway must be publicly reachable. For local testing, use [ngrok](https://ngrok.com): `ngrok http 3000`

## Step 6: Enter Credentials in Zeus

Run `zeus setup` or open the TUI and navigate to **Channels → WhatsApp**:

- **Access Token** — paste the `EAA...` token from Step 4
- **Phone Number ID** — paste the numeric ID from Step 3 (found at developers.facebook.com → WhatsApp → API Setup → under the sender phone number)
- **Bridge URL** — the URL where Zeus is running, e.g. `https://your-domain.com` or `http://localhost:3000` for local testing

## Step 7: Test It

Meta provides a free test number you can send messages to:

1. On the **API Setup** page, scroll to **Send and receive messages**
2. Add your personal WhatsApp number as a test recipient
3. Click **Send message** — you should receive a test message
4. Reply to it — Zeus should pick it up via the webhook

## Notes

- The access token is sensitive — treat it like a password
- WhatsApp Cloud API is free for the first 1,000 business-initiated conversations per month
- Meta reviews apps before allowing messages to non-test numbers — submit your app for review when ready
- For production, use a permanent System User token, not the temporary one
- Webhook verification happens once; after that, messages flow automatically
