class JsonqueryGui < Formula
  desc "Native desktop GUI for browsing and querying large JSON files"
  homepage "https://github.com/nujufas/jsonquery_gui"
  version "0.3.1"
  license "MIT"

  on_linux do
    url "https://github.com/nujufas/jsonquery_gui/releases/download/v0.3.1/jsonquery_gui-0.3.1-linux-x86_64.tar.gz"
    sha256 "01f98e4c74edb3a5b8f894c44e65e8adcf8a111854a379a0d7cf29c6625b3ece"
  end

  def install
    bin.install "jsonquery_gui"
  end

  test do
    assert_predicate bin/"jsonquery_gui", :exist?
  end
end
