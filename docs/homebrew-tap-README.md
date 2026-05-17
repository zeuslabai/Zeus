# homebrew-zeus

Homebrew tap for [Zeus](https://github.com/zeuslabai/Zeus) — autonomous AI assistant.

## Install

```bash
brew tap zeuslabai/zeus
brew install zeus
```

## Usage

```bash
zeus onboard     # First-time setup wizard
zeus             # Launch terminal UI
zeus gateway     # Start API server + channels + cron
zeus doctor      # Run diagnostics
```

## Update

```bash
brew upgrade zeus
```

## Untap

```bash
brew uninstall zeus
brew untap zeuslabai/zeus
```

## Formula

The formula lives in [Formula/zeus.rb](https://github.com/zeuslabai/homebrew-zeus/blob/main/Formula/zeus.rb).

SHA256 checksums are updated automatically by the release workflow on each tagged release.
