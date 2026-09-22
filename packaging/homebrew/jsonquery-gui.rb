class JsonqueryGui < Formula
  desc "Native desktop GUI for browsing and querying large JSON files"
  homepage "https://github.com/nujufas/jsonquery_gui"
  version "0.4.2"
  license "MIT"

  on_linux do
    url "https://github.com/nujufas/jsonquery_gui/releases/download/v0.4.2/jsonquery_gui-0.4.2-linux-x86_64.tar.gz"
    sha256 "45720620045640862262e7a6e0dbbd043ee0f84c069275d7e5d681b45ce2a717"
  end

  def install
    bin.install "jsonquery_gui"
  end

  test do
    assert_predicate bin/"jsonquery_gui", :exist?
  end
end
