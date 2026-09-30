#!/usr/bin/env bash
#
# Release moor, in two steps because master is protected (changes arrive by
# pull request only):
#
#   ./release.sh prepare <version>   # open the release PR: version + CHANGELOG
#   ./release.sh publish <version>   # after that PR merges: gate, publish, tag
#
# Add --dry-run to either to see what it would do without changing anything.
#
# prepare: from an up-to-date master, puts <version> into cli/Cargo.toml and
#   Cargo.lock, turns CHANGELOG.md's "## [Unreleased]" into "## [<version>] -
#   <today>", commits that on release/v<version>, pushes it and opens the PR.
#
# publish: on master, clean and identical to origin/master, with the version
#   and CHANGELOG section from the merged PR: runs the same gates CI does,
#   publishes to crates.io (with the token `cargo login` stored), tags
#   v<version>, pushes the tag, and creates the GitHub release from the
#   CHANGELOG section. It never pushes master. Each step checks whether it is
#   already done, so a release interrupted halfway is finished by running the
#   same command again.
#
# A gate, not a wizard: it stops at the first failed precondition and says
# which one. Release notes come from CHANGELOG.md, never from a prompt.

set -euo pipefail

readonly REPO="daneb/moor"
readonly CRATE="moor"
readonly TRUNK="master"
readonly CRATE_DIR="cli"

# ---------------------------------------------------------------- presentation

if [ -t 1 ]; then
    readonly C_RED=$'\033[0;31m' C_GRN=$'\033[0;32m' C_YEL=$'\033[0;33m'
    readonly C_DIM=$'\033[2m' C_BLD=$'\033[1m' C_OFF=$'\033[0m'
else
    readonly C_RED='' C_GRN='' C_YEL='' C_DIM='' C_BLD='' C_OFF=''
fi

step()  { printf '\n%s▶ %s%s\n' "$C_BLD" "$1" "$C_OFF"; }
ok()    { printf '  %s✓%s %s\n' "$C_GRN" "$C_OFF" "$1"; }
note()  { printf '  %s·%s %s\n' "$C_DIM" "$C_OFF" "$1"; }
warn()  { printf '  %s!%s %s\n' "$C_YEL" "$C_OFF" "$1"; }
die()   { printf '\n%s✗ %s%s\n\n' "$C_RED" "$1" "$C_OFF" >&2; exit 1; }

# ------------------------------------------------------------------- arguments

MODE=""
VERSION=""
DRY_RUN=false

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run)        DRY_RUN=true ;;
        -h|--help)        sed -n '3,25p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        -*)               die "unknown flag: $1" ;;
        prepare|publish)  [ -n "$MODE" ] && die "mode given twice"; MODE="$1" ;;
        *)                [ -n "$VERSION" ] && die "version given twice: $VERSION and $1"
                          VERSION="$1" ;;
    esac
    shift
done

[ -n "$MODE" ] && [ -n "$VERSION" ] \
    || die "usage: ./release.sh prepare|publish <version> [--dry-run]"

# Semver, because the tag is a public API and "v1.2" sorts badly forever.
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] \
    || die "not a semver version: $VERSION"

readonly MODE VERSION DRY_RUN
readonly TAG="v${VERSION}"
readonly BRANCH="release/${TAG}"

run() {
    if $DRY_RUN; then
        printf '  %swould run:%s %s\n' "$C_DIM" "$C_OFF" "$*"
    else
        "$@"
    fi
}

crate_version() {
    awk -F'"' '/^version = /{ print $2; exit }' "$CRATE_DIR/Cargo.toml"
}

# The CHANGELOG section for a heading, everything up to the next "## [".
changelog_section() {
    awk -v heading="$1" '
        index($0, heading) == 1 { grab = 1; next }
        grab && /^## \[/        { exit }
        grab                    { print }
    ' CHANGELOG.md | sed -e '/./,$!d'
}

printf '%smoor release %s — %s%s\n' "$C_BLD" "$TAG" "$MODE" "$C_OFF"
$DRY_RUN && printf '%sdry run — nothing will be changed%s\n' "$C_YEL" "$C_OFF"

# ------------------------------------------------------------- shared checks

step "Preconditions"

[ -d .git ] && [ -f "$CRATE_DIR/Cargo.toml" ] && [ -f CHANGELOG.md ] \
    || die "run this from the root of the moor repository"

branch=$(git rev-parse --abbrev-ref HEAD)
[ "$branch" = "$TRUNK" ] || die "on branch '$branch', expected '$TRUNK'"
ok "on $TRUNK"

git update-index --refresh >/dev/null 2>&1 || true
git diff-index --quiet HEAD -- || die "working tree has uncommitted changes"
ok "working tree clean"

# The same commit GitHub has: a release built from a stale or diverged master
# is how keel's release tagged a commit that was already out of date.
git fetch -q origin "$TRUNK" || die "could not fetch origin/$TRUNK"
[ "$(git rev-parse HEAD)" = "$(git rev-parse "origin/$TRUNK")" ] \
    || die "$TRUNK is not the same commit as origin/$TRUNK — run 'git pull --ff-only' first"
ok "in step with origin/$TRUNK ($(git rev-parse --short HEAD))"

command -v gh >/dev/null 2>&1 || die "gh is needed (to open the PR and the GitHub release)"

# ================================================================= prepare

if [ "$MODE" = "prepare" ]; then
    step "What will change"

    current=$(crate_version)
    [ "$current" != "$VERSION" ] || die "$CRATE_DIR/Cargo.toml is already at $VERSION — run: ./release.sh publish $VERSION"
    note "$CRATE_DIR/Cargo.toml: $current → $VERSION"

    grep -q '^## \[Unreleased\]' CHANGELOG.md \
        || die "CHANGELOG.md has no '## [Unreleased]' section — list the changes there first"
    [ -n "$(changelog_section '## [Unreleased]')" ] \
        || die "CHANGELOG.md's [Unreleased] section is empty"
    today=$(date +%Y-%m-%d)
    note "CHANGELOG.md: [Unreleased] → [$VERSION] - $today"

    if git rev-parse --verify --quiet "refs/heads/$BRANCH" >/dev/null \
        || git ls-remote --exit-code --heads origin "$BRANCH" >/dev/null 2>&1; then
        die "branch $BRANCH already exists — is a release PR already open?"
    fi
    ok "branch $BRANCH is free"

    step "Release PR"
    if $DRY_RUN; then
        note "would create $BRANCH, commit the version and CHANGELOG, push, and open a PR"
    else
        git switch -q -c "$BRANCH"
        # BSD and GNU sed disagree about -i, so awk, once per file.
        awk -v v="$VERSION" '
            !done && /^version = / { print "version = \"" v "\""; done = 1; next }
            { print }
        ' "$CRATE_DIR/Cargo.toml" > "$CRATE_DIR/Cargo.toml.tmp" \
            && mv "$CRATE_DIR/Cargo.toml.tmp" "$CRATE_DIR/Cargo.toml"
        (cd "$CRATE_DIR" && cargo update --workspace --quiet)
        [ "$(crate_version)" = "$VERSION" ] || die "failed to write $VERSION into $CRATE_DIR/Cargo.toml"
        awk -v v="$VERSION" -v d="$today" '
            !done && /^## \[Unreleased\]/ { print "## [" v "] - " d; done = 1; next }
            { print }
        ' CHANGELOG.md > CHANGELOG.md.tmp && mv CHANGELOG.md.tmp CHANGELOG.md
        git add "$CRATE_DIR/Cargo.toml" "$CRATE_DIR/Cargo.lock" CHANGELOG.md
        git commit -q -m "Release ${TAG}: version and CHANGELOG"
        git push -q -u origin "$BRANCH"
        url=$(gh pr create --repo "$REPO" --base "$TRUNK" --head "$BRANCH" \
            --title "Release ${TAG}" \
            --body "Version ${VERSION} and its CHANGELOG section. After merging: \`./release.sh publish ${VERSION}\`.")
        git switch -q "$TRUNK"
        ok "$url"
    fi
    printf '\n%sRelease PR ready.%s Merge it, then:\n\n  git pull --ff-only && ./release.sh publish %s\n\n' \
        "$C_GRN" "$C_OFF" "$VERSION"
    exit 0
fi

# ================================================================= publish

current=$(crate_version)
[ "$current" = "$VERSION" ] \
    || die "$CRATE_DIR/Cargo.toml is at $current, not $VERSION — run './release.sh prepare $VERSION' and merge its PR first"
ok "$CRATE_DIR/Cargo.toml at $VERSION"

# A version already tagged must be published from exactly that commit.
# Otherwise a master that has moved on, still carrying the old version
# number, would go out under it.
tagged=$(git ls-remote origin "refs/tags/${TAG}^{}" "refs/tags/${TAG}" | awk 'NR == 1 { print $1 }')
if [ -n "$tagged" ] && [ "$tagged" != "$(git rev-parse HEAD)" ]; then
    die "$TAG is tagged on ${tagged:0:7}, but $TRUNK is at $(git rev-parse --short HEAD) — this version is already released from another commit"
fi

NOTES=$(mktemp)
trap 'rm -f "$NOTES"' EXIT
changelog_section "## [$VERSION]" > "$NOTES"
[ -s "$NOTES" ] || die "CHANGELOG.md has no '## [$VERSION]' section"
ok "$(wc -l < "$NOTES" | tr -d ' ') lines of release notes from CHANGELOG.md"

# ------------------------------------------------------------------- gates

step "Gates (the same ones CI runs)"
if $DRY_RUN; then
    note "would run: cargo fmt --check, cargo clippy -D warnings, cargo test"
else
    (cd "$CRATE_DIR" && cargo fmt --check) || die "cargo fmt --check failed"
    ok "formatted"
    (cd "$CRATE_DIR" && cargo clippy --all-targets --quiet -- -D warnings) \
        || die "clippy found warnings"
    ok "no clippy warnings"
    test_out=$(cd "$CRATE_DIR" && cargo test --quiet 2>&1) \
        || { echo "$test_out" | tail -30; die "tests failed"; }
    passed=$(echo "$test_out" | awk '/^test result: ok/ { n += $4 } END { print n+0 }')
    [ "$passed" -gt 0 ] || die "no tests ran — that is a failure, not a pass"
    ok "$passed tests passed"
fi

# --------------------------------------------------------------- crates.io

step "crates.io"
published() {
    curl -fsS -o /dev/null -A "moor-release (github.com/$REPO)" \
        "https://crates.io/api/v1/crates/$CRATE/$VERSION" 2>/dev/null
}
if published; then
    note "$CRATE $VERSION is already on crates.io"
else
    (cd "$CRATE_DIR" && cargo publish --dry-run --quiet) || die "cargo publish --dry-run failed"
    ok "packages cleanly"
    # cargo uses the token `cargo login` stored; this script never sees it.
    (cd "$CRATE_DIR" && run cargo publish --quiet) || die "cargo publish failed"
    $DRY_RUN || ok "published $CRATE $VERSION"
fi

# --------------------------------------------------------------------- tag

step "Tag"
if git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
    note "$TAG is already on origin"
else
    if ! git rev-parse --verify --quiet "refs/tags/$TAG" >/dev/null; then
        run git tag -a "$TAG" -F "$NOTES"
    fi
    # Only the tag: master is protected, and it already has everything.
    run git push -q origin "$TAG"
    $DRY_RUN || ok "$TAG pushed"
fi

# ---------------------------------------------------------- GitHub release

step "GitHub release"
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    note "release $TAG already exists"
else
    run gh release create "$TAG" --repo "$REPO" --title "$TAG" --notes-file "$NOTES" --verify-tag
    $DRY_RUN || ok "https://github.com/${REPO}/releases/tag/${TAG}"
fi

printf '\n%s%s released.%s\n\n' "$C_GRN" "$TAG" "$C_OFF"
