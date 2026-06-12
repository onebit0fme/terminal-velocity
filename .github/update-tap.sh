#!/bin/sh
# Regenerate the Homebrew formula in the tap repo for the current release.
# Called by .github/workflows/release.yml after publish. Env:
#   GH_TOKEN  PAT with contents:write on $TAP   SRC  this repo (owner/name)
#   TAP       tap repo (owner/homebrew-tap)     TAG  release tag (vX.Y.Z)
#   DRY_RUN=1 generate tv.rb but skip the commit (for local testing)
set -eu

ver="${TAG#v}"
base="https://github.com/$SRC/releases/download/$TAG"
sha() { curl -fsSL "$base/$1" | sha256sum | cut -d' ' -f1; }

MA=$(sha tv-aarch64-apple-darwin.tar.gz)
MI=$(sha tv-x86_64-apple-darwin.tar.gz)
LA=$(sha tv-aarch64-unknown-linux-musl.tar.gz)
LI=$(sha tv-x86_64-unknown-linux-musl.tar.gz)

cat > tv.rb <<EOF
class Tv < Formula
  desc "Is your build's speed real throughput, or just thrashing? A git-status for build-flow health"
  homepage "https://github.com/$SRC"
  version "$ver"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "$base/tv-aarch64-apple-darwin.tar.gz"
      sha256 "$MA"
    end
    on_intel do
      url "$base/tv-x86_64-apple-darwin.tar.gz"
      sha256 "$MI"
    end
  end

  on_linux do
    on_arm do
      url "$base/tv-aarch64-unknown-linux-musl.tar.gz"
      sha256 "$LA"
    end
    on_intel do
      url "$base/tv-x86_64-unknown-linux-musl.tar.gz"
      sha256 "$LI"
    end
  end

  def install
    bin.install "tv"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/tv --version")
  end
end
EOF

if [ "${DRY_RUN:-}" = "1" ]; then
  echo "(dry run) generated tv.rb for $ver"
  exit 0
fi

existing=$(gh api "repos/$TAP/contents/Formula/tv.rb" --jq .sha 2>/dev/null || echo "")
gh api "repos/$TAP/contents/Formula/tv.rb" -X PUT \
  -f message="tv $ver" \
  -f content="$(base64 -w0 tv.rb)" \
  ${existing:+-f sha="$existing"} \
  -f branch=main \
  --jq '.commit.html_url'
