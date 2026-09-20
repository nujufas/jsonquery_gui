class JsonqueryGui < Formula
  desc "Native desktop GUI for browsing and querying large JSON files"
  homepage "https://github.com/nujufas/jsonquery_gui"
  version "0.4.1"
  license "MIT"

  on_linux do
    url "https://github.com/nujufas/jsonquery_gui/releases/download/v0.4.1/jsonquery_gui-0.4.1-linux-x86_64.tar.gz"
    sha256 "1e1369cf84de9112ae31e78cb806d09b436954de4bc3975741fcc5cee149fa56"
  end

  def install
    bin.install "jsonquery_gui"
  end

  test do
    assert_predicate bin/"jsonquery_gui", :exist?
  end
end
