class JsonqueryGui < Formula
  desc "Native desktop GUI for browsing and querying large JSON files"
  homepage "https://github.com/nujufas/jsonquery_gui"
  version "0.3.0"
  license "MIT"

  on_linux do
    url "https://github.com/nujufas/jsonquery_gui/releases/download/v0.3.0/jsonquery_gui-0.3.0-linux-x86_64.tar.gz"
    sha256 "def52d970fe34e7505e9f057d422f3d1eae366770daf7d2726200a3ebbdc047d"
  end

  def install
    bin.install "jsonquery_gui"
  end

  test do
    assert_predicate bin/"jsonquery_gui", :exist?
  end
end
