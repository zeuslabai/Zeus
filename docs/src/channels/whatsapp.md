# WhatsApp

Zeus connects to WhatsApp using the **WhatsApp Cloud API** via reqwest. This is Meta's official API for WhatsApp Business, providing programmatic access to send and receive messages.

## Configuration

Add the following to your `~/.zeus/config.toml`:

```toml
[channels.whatsapp]
access_token = "your_access_token"
phone_number_id = "your_phone_number_id"
```

| Field | Description |
|-------|-------------|
| `access_token` | WhatsApp Cloud API access token |
| `phone_number_id` | WhatsApp Business phone number ID |

## Setup

### 1. Create a Meta Developer Account

1. Go to [https://developers.facebook.com](https://developers.facebook.com) and create an account or log in.
2. Create a new app and select **Business** as the app type.

### 2. Set Up WhatsApp Business

1. In your app dashboard, add the **WhatsApp** product.
2. Follow the setup wizard to configure a WhatsApp Business account.
3. You will receive a test phone number and a temporary access token.

### 3. Get a Permanent Access Token

The temporary token from the setup wizard expires after 24 hours. For production use:

1. Go to **Business Settings > System Users**.
2. Create a system user and assign it to your WhatsApp Business Account.
3. Generate a permanent token for the system user with the `whatsapp_business_messaging` permission.

### 4. Get Your Phone Number ID

1. In the WhatsApp developer dashboard, navigate to **WhatsApp > Getting Started**.
2. Your phone number ID is displayed alongside the test phone number.
3. For production, register a real phone number and use its ID.

### 5. Configure Webhooks (for receiving messages)

1. In the WhatsApp developer dashboard, configure a webhook URL pointing to your Zeus gateway.
2. Subscribe to the `messages` webhook field.
3. The Zeus gateway handles incoming webhook events on its webhook endpoint.

### 6. Configure Zeus

Add the access token and phone number ID to your `config.toml` as shown above.

## Features

- Send text messages to WhatsApp users
- Receive incoming messages via webhooks
- Uses Meta's official Cloud API

## Limitations

- **WhatsApp Business API required** -- This is not a personal WhatsApp connection. It requires a WhatsApp Business account.
- **24-hour messaging window** -- WhatsApp enforces a 24-hour window for responding to user-initiated messages. Outside this window, only approved message templates can be sent.
- **Webhook URL required for receiving** -- Inbound messages require a publicly accessible URL for webhook delivery.
- **Phone number verification** -- Production phone numbers must go through Meta's verification process.
