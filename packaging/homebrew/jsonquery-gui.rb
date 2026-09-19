class JsonqueryGui < Formula
  desc "Native desktop GUI for browsing and querying large JSON files"
  homepage "https://github.com/nujufas/jsonquery_gui"
  version "0.4.0"
  license "MIT"

  on_linux do
    url "https://github.com/nujufas/jsonquery_gui/releases/download/v0.4.0/jsonquery_gui-0.4.0-linux-x86_64.tar.gz"
    sha256 "81b4d2919b426e760fd4005cac3aef6bc57b7723e57721b9a8279b0e22db366c"
  end

  def install
    bin.install "jsonquery_gui"
  end

  test do
    assert_predicate bin/"jsonquery_gui", :exist?
  end
end
