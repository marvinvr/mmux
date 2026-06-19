# This formula is generated automatically — do not edit by hand.
# CI rewrites it on every vX.Y.Z tag (see .github/workflows/release.yml).
class Mmux < Formula
  desc "Persistent, per-directory terminal multiplexer for AI agents and dev processes"
  homepage "https://github.com/marvinvr/mmux"
  url "URL_PLACEHOLDER"
  sha256 "SHA256_PLACEHOLDER"
  license "MIT"
  head "https://github.com/marvinvr/mmux.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "mmux #{version}", shell_output("#{bin}/mmux --version")
  end
end
