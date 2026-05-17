class Zeus < Formula
  desc "Autonomous AI assistant with 327 tools, 11 LLM providers, and 5 frontends"
  homepage "https://zeuslab.ai"
  url "https://github.com/zeuslabai/zeus/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "" # TODO: fill after release tag
  license "MIT"
  head "https://github.com/zeuslabai/zeus.git", branch: "main"

  depends_on "rust" => :build
  depends_on "pkg-config" => :build
  depends_on "cmake" => :build
  depends_on "openssl@3"
  depends_on "sqlite"

  def install
    system "cargo", "build", "--release", "--bin", "zeus"
    bin.install "target/release/zeus"

    # Shell completions
    generate_completions_from_executable(bin/"zeus", "completion")
  end

  def post_install
    # Create workspace directory
    (var/"zeus").mkpath
  end

  service do
    run [opt_bin/"zeus", "gateway", "--host", "0.0.0.0"]
    keep_alive true
    working_dir var/"zeus"
    log_path var/"log/zeus-gateway.log"
    error_log_path var/"log/zeus-gateway.log"
  end

  test do
    assert_match "zeus", shell_output("#{bin}/zeus --version")

    # Verify doctor runs without crash
    output = shell_output("#{bin}/zeus doctor 2>&1", 0)
    assert_match(/config|workspace/i, output)
  end
end
