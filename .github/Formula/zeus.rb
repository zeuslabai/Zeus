class Zeus < Formula
  desc "Autonomous AI agent framework — 212 tools, 11 LLM providers, fleet coordination"
  homepage "https://github.com/zeuslabai/Zeus"
  version "1.0.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/zeuslabai/Zeus/releases/download/v1.0.0/zeus-darwin-arm64.tar.gz"
      sha256 "PLACEHOLDER_SHA256_ARM64"
    else
      url "https://github.com/zeuslabai/Zeus/releases/download/v1.0.0/zeus-darwin-amd64.tar.gz"
      sha256 "PLACEHOLDER_SHA256_AMD64"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/zeuslabai/Zeus/releases/download/v1.0.0/zeus-linux-arm64.tar.gz"
      sha256 "PLACEHOLDER_SHA256_LINUX_ARM64"
    else
      url "https://github.com/zeuslabai/Zeus/releases/download/v1.0.0/zeus-linux-amd64.tar.gz"
      sha256 "PLACEHOLDER_SHA256_LINUX_AMD64"
    end
  end

  def install
    bin.install "zeus"
  end

  test do
    system "#{bin}/zeus", "--version"
  end
end
