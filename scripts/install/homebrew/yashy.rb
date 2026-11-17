class Yashy < Formula
  desc "A statically typed language compiled to native binaries through LLVM"
  homepage "https://github.com/Yash-K-Jagani/Yashy"
  version "1.0.0"
  # The release workflow fills in the URL and SHA-256 for each tag.
  url "https://github.com/Yash-K-Jagani/Yashy/releases/download/v#{version}/yashy-x86_64-apple-darwin.tar.gz"
  sha256 "REPLACE_WITH_RELEASE_SHA256"
  license "MIT"

  def install
    bin.install "yashyc"
    (share/"yashy/stdlib").install Dir["stdlib/yashy/*"]
  end

  test do
    assert_match "yashyc", shell_output("#{bin}/yashyc version")
    assert_equal 0, system("#{bin}/yashyc doctor")
  end
end
