class Zeus < Formula
  desc "Autonomous AI assistant — 31 crates, 212 tools, 9 channels, multi-agent orchestration"
  homepage "https://github.com/zeuslabai/Zeus"
  version "1.0.0"
  license "MIT OR Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/zeuslabai/Zeus/releases/download/v#{version}/zeus-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_AARCH64_DARWIN"
    end
    on_intel do
      url "https://github.com/zeuslabai/Zeus/releases/download/v#{version}/zeus-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_X86_64_DARWIN"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/zeuslabai/Zeus/releases/download/v#{version}/zeus-#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER_AARCH64_LINUX"
    end
    on_intel do
      url "https://github.com/zeuslabai/Zeus/releases/download/v#{version}/zeus-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER_X86_64_LINUX"
    end
  end

  def install
    bin.install "zeus"

    # Shell completions
    generate_completions_from_executable(bin/"zeus", "completion")
  end

  def post_install
    zeus_dir = Pathname.new(Dir.home)/".zeus"
    unless zeus_dir.exist?
      (zeus_dir/"workspace/memory").mkpath
      (zeus_dir/"workspace/daily").mkpath
      (zeus_dir/"sessions").mkpath
      ohai "Zeus workspace initialized at #{zeus_dir}"
    end
  end

  def caveats
    <<~EOS
      Zeus #{version} installed!

      Get started:
        zeus onboard          # Interactive setup (provider, model, channels)
        zeus                  # Launch terminal UI
        zeus gateway          # Start API server + all channels + cron

      Configuration:
        ~/.zeus/config.toml   # Single source of truth for all settings

      API server (once running):
        http://localhost:8080/health
        http://localhost:8080/v1/status
        http://localhost:8080/docs           # Interactive API docs

      Documentation:
        https://github.com/zeuslabai/Zeus/blob/main/README.md
        https://github.com/zeuslabai/Zeus/blob/main/docs/quickstart.md

      To run as a background service (launchd):
        zeus daemon install
        zeus daemon start
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/zeus --version")
    assert_match "ok", shell_output("#{bin}/zeus doctor --json 2>/dev/null || true")
  end
end
