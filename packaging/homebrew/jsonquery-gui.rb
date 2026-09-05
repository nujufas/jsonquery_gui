class JsonqueryGui < Formula
  desc "Native desktop GUI for browsing and querying large JSON files"
  homepage "https://github.com/nujufas/jsonquery_gui"
  version "0.2.0"
  license "MIT"

  on_linux do
    url "https://github.com/nujufas/jsonquery_gui/releases/download/v0.2.0/jsonquery_gui-0.2.0-linux-x86_64.tar.gz"
    sha256 "d3e4a9954ea40d28eeab3435feb8d432f9f4f322693e9b32cf9b1ec39a969f53"
  end

  def install
    bin.install "jsonquery_gui"
  end

  test do
    assert_predicate bin/"jsonquery_gui", :exist?
  end
end
