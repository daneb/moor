# Installing with Homebrew (maintainers)

```bash
brew install --cask daneb/tap/moor
```

The cask installs the `moor` binary and the sources `images/build.sh`
needs (it prints the path in its caveats). Docker is not installed by it.
`.github/workflows/release-binaries.yml` builds the macOS tarballs when a
release is published; `packaging/homebrew/` holds the cask and the scripts
that package it. One-time setup: create `daneb/homebrew-tap`, and add a
`HOMEBREW_TAP_TOKEN` secret (contents:write on that repo) so releases bump
the cask automatically. Without it, run
`packaging/homebrew/update-cask.sh <version> <tap>/Casks/moor.rb` by hand.
