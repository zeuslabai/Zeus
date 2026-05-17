# How to Get Mattermost Credentials

Zeus connects to Mattermost using a **bot account** and the Mattermost WebSocket + REST API. You'll need access to a Mattermost server (self-hosted or Mattermost Cloud).

## Prerequisites

- A Mattermost server (self-hosted or [Mattermost Cloud](https://mattermost.com/sign-up/))
- System Admin access to create a bot account (or ask your admin)

## Step 1: Create a Bot Account

Bot accounts are the recommended way to integrate with Mattermost — they have their own identity and don't consume a user license on most plans.

1. Log into Mattermost as a **System Admin**
2. Go to **System Console → Integrations → Bot Accounts**
3. Ensure **Enable Bot Account Creation** is set to **true**
4. Navigate to your **Profile menu (top-left) → Integrations → Bot Accounts**
5. Click **Add Bot Account**
6. Fill in:
   - **Username** — e.g. `zeus`
   - **Display Name** — e.g. `Zeus AI`
   - **Description** — optional
   - **Role** — select **Member** (or System Admin if Zeus needs full access)
7. Click **Create Bot Account**
8. **Copy the Access Token immediately** — it's only shown once

> ⚠️ If you miss the token, you can regenerate it: go back to Bot Accounts → click the bot → **Create New Token**.

## Step 2: Get Your Server URL

This is the base URL of your Mattermost instance:
- Self-hosted: typically `https://mattermost.yourcompany.com`
- Mattermost Cloud: `https://yourworkspace.cloud.mattermost.com`

## Step 3: Get the Team and Channel (optional)

If you want Zeus to post to a specific channel by default:

1. Navigate to the channel in Mattermost
2. Click the **channel name** at the top → **Edit Channel** (or **View Info**)
3. The **Channel ID** is shown there (e.g. `abc123def456`)

Alternatively, you can use the channel name directly (e.g. `town-square`).

## Step 4: Invite the Bot to Channels

Bot accounts can't join channels automatically — a human must invite them:

1. Go to the channel you want Zeus to monitor
2. Click the **Members** icon → **Add Members**
3. Search for your bot's username and add it

## Step 5: Enter Credentials in Zeus

Run `zeus setup` or open the TUI and navigate to **Channels → Mattermost**:

- **Server URL** — your Mattermost instance URL (e.g. `https://mattermost.yourcompany.com`)
- **Token** — the bot access token from Step 1
- **Team** — your team name or ID (visible in the URL when browsing Mattermost: `mattermost.com/team-name/`)
- **Channel** — channel name (e.g. `town-square`) or channel ID

## Step 6: Test It

```bash
zeus gateway
# In Mattermost: @zeus hello
```

Zeus should reply in the channel.

## Notes

- The bot token is sensitive — treat it like a password
- Bot accounts require Mattermost Server v5.10 or later
- If your server uses a self-signed certificate, you may need to configure Zeus to trust it
- Mattermost uses WebSocket for real-time messaging — ensure port 443 (or 8065 for default HTTP) is accessible
- To use a personal access token instead of a bot: go to Profile → Security → Personal Access Tokens (must be enabled by admin)
