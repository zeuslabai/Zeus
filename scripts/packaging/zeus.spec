Name:           zeus
Version:        %{?zeus_version}%{!?zeus_version:0.1.0}
Release:        1%{?dist}
Summary:        Autonomous AI assistant with 327 tools and 11 LLM providers
License:        MIT OR Apache-2.0
URL:            https://zeuslab.ai
Source0:        https://github.com/zeuslabai/Zeus/archive/v%{version}.tar.gz

# Pre-built binary is provided — no BuildRequires for Rust toolchain
AutoReqProv:    no

Requires:       glibc >= 2.31
Requires:       openssl-libs >= 3.0
Requires:       sqlite-libs
Recommends:     curl
Recommends:     ca-certificates

%description
Zeus is a local-first AI assistant featuring a cognitive engine (Nous),
multi-channel chat (Telegram, Discord, Slack, Email, iMessage, WhatsApp,
Signal, Matrix), 5 frontends (TUI, Web, macOS, iOS, visionOS), browser
automation, voice calls, and security sandboxing.

327 tools across 22 categories, 11 LLM providers (Anthropic, OpenAI,
Google, Groq, Ollama, and more), and full offline capability.

Run 'zeus' for the TUI, 'zeus gateway' for the API server,
or 'zeus setup' for guided first-run configuration.

%install
# Binary
install -D -m 755 %{_sourcedir}/zeus %{buildroot}%{_prefix}/local/bin/zeus

# Systemd service
install -D -m 644 %{_sourcedir}/zeus-gateway.service %{buildroot}%{_unitdir}/zeus-gateway.service

# Shell completions (pre-generated)
install -d %{buildroot}%{_sysconfdir}/bash_completion.d
install -d %{buildroot}%{_datadir}/zsh/site-functions
install -d %{buildroot}%{_datadir}/fish/vendor_completions.d

if [ -f %{_sourcedir}/completions/zeus.bash ]; then
    install -m 644 %{_sourcedir}/completions/zeus.bash %{buildroot}%{_sysconfdir}/bash_completion.d/zeus
fi
if [ -f %{_sourcedir}/completions/_zeus ]; then
    install -m 644 %{_sourcedir}/completions/_zeus %{buildroot}%{_datadir}/zsh/site-functions/_zeus
fi
if [ -f %{_sourcedir}/completions/zeus.fish ]; then
    install -m 644 %{_sourcedir}/completions/zeus.fish %{buildroot}%{_datadir}/fish/vendor_completions.d/zeus.fish
fi

# License
install -D -m 644 %{_sourcedir}/copyright %{buildroot}%{_datadir}/doc/zeus/copyright

%pre
# Create zeus system user if it doesn't exist (for gateway service)
getent group zeus >/dev/null 2>&1 || groupadd --system zeus
getent passwd zeus >/dev/null 2>&1 || useradd --system --gid zeus --create-home --home-dir /home/zeus zeus

%post
# Create zeus data directory
mkdir -p /home/zeus/.zeus
chown -R zeus:zeus /home/zeus/.zeus

# Reload systemd
%systemd_post zeus-gateway.service

echo ""
echo "Zeus installed successfully!"
echo ""
echo "  Quick start:    zeus"
echo "  Setup wizard:   zeus setup"
echo "  API gateway:    zeus gateway"
echo "  Start service:  systemctl enable --now zeus-gateway"
echo ""

%preun
%systemd_preun zeus-gateway.service

%postun
%systemd_postun_with_restart zeus-gateway.service

%files
%{_prefix}/local/bin/zeus
%{_unitdir}/zeus-gateway.service
%config(noreplace) %{_sysconfdir}/bash_completion.d/zeus
%{_datadir}/zsh/site-functions/_zeus
%{_datadir}/fish/vendor_completions.d/zeus.fish
%{_datadir}/doc/zeus/copyright
