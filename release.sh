#!/usr/bin/env bash
#
# Release moor. Master is protected, so changes arrive by pull request only
# and a release is two steps with a merge in between:
#
#   ./release.sh status              # where am I? what's the next command?
#   ./release.sh unreleased          # scaffold CHANGELOG's [Unreleased] section
#   ./release.sh unreleased --fixed -m "- **…**"   # …or file an entry straight in
#   ./release.sh prepare <version>   # open the release PR: version + CHANGELOG
#   ./release.sh publish <version>   # after that PR merges: gate, publish, tag
#
# <version> is a semver string (1.2.3) or a bump keyword — patch, minor or
# major — which the script resolves against the current version for you.
# Add --dry-run to any step to see what it would do without changing anything.
#
# The normal path, start to finish:
#
#   1. ./release.sh unreleased          (once, if there's no [Unreleased] yet)
#      …then write your changes under its Added/Changed/Fixed headings — or
#      file them straight in without opening the file:
#         ./release.sh unreleased --fixed -m "- **Fixed X.** …"
#         llm-that-wrote-the-note | ./release.sh unreleased --added -
#   2. ./release.sh prepare patch       opens the release PR. Review & merge it.
#   3. git pull --ff-only
#   4. ./release.sh publish 0.9.2       gates, publishes to crates.io, tags,
#                                       and cuts the GitHub release.
#
# Lost? `./release.sh status` reads the repo and tells you which of these you
# are at and the exact command to run next.
#
# --- what each step does ------------------------------------------------
#
# status:   read-only. Reports the current version, whether [Unreleased] has
#   content, whether a release branch/PR is open, whether the tag and the
#   crates.io publish exist — then prints the one command to run next.
#
# unreleased: read-only unless you confirm. If CHANGELOG.md has no
#   "## [Unreleased]" section it inserts an empty one (Added/Changed/Fixed)
#   at the top of the log. Given an entry (--added/--changed/--fixed with
#   -m "…" or piped on stdin via a trailing "-") it files that text verbatim
#   under the chosen heading — made for pasting a note an LLM drafted.
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
VERSION_ARG=""
DRY_RUN=false
CATEGORY=""          # Added / Changed / Fixed, set by --added/--changed/--fixed
MESSAGE=""           # entry text from --message
MESSAGE_SET=false    # distinguishes an unset --message from an empty one
READ_STDIN=false     # entry text comes from stdin (a trailing "-")

# The whole leading comment block is the help: print it all, not a fixed
# line range that silently drops anything added below it.
print_help() {
    awk 'NR == 1 { next }                       # shebang
         /^#!?/  { sub(/^# ?/, ""); print; next }
         { exit }' "$0"
}

set_category() {
    [ -z "$CATEGORY" ] || die "only one of --added/--changed/--fixed"
    CATEGORY="$1"
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run)                   DRY_RUN=true ;;
        -h|--help)                   print_help; exit 0 ;;
        --added)                     set_category "Added" ;;
        --changed)                   set_category "Changed" ;;
        --fixed)                     set_category "Fixed" ;;
        --message|-m)                shift; [ $# -gt 0 ] || die "--message needs an argument"
                                     MESSAGE="$1"; MESSAGE_SET=true ;;
        --message=*)                 MESSAGE="${1#--message=}"; MESSAGE_SET=true ;;
        -)                           READ_STDIN=true ;;
        -*)                          die "unknown flag: $1" ;;
        status|unreleased|prepare|publish)
                                     [ -n "$MODE" ] && die "mode given twice"
                                     MODE="$1" ;;
        *)                           [ -n "$VERSION_ARG" ] && die "version/bump given twice: $VERSION_ARG and $1"
                                     VERSION_ARG="$1" ;;
    esac
    shift
done

[ -n "$MODE" ] || die "usage: ./release.sh status|unreleased|prepare|publish [<version>] [--dry-run]"

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

# True when a CHANGELOG section has real content — at least one line that is
# not blank and not a bare "### " subheading. A freshly scaffolded
# [Unreleased] (empty Added/Changed/Fixed) is therefore *not* ready.
section_has_content() {
    changelog_section "$1" | grep -qvE '^(###? .*|\s*)$'
}

# Insert an empty "## [Unreleased]" block (Added/Changed/Fixed) above the
# first released "## [" heading. No-op if one already exists. awk prints the
# block line by line — a multiline -v string isn't portable (BSD awk rejects
# the embedded newlines).
scaffold_unreleased() {
    grep -q '^## \[Unreleased\]' CHANGELOG.md && return 0
    awk '
        !done && /^## \[/ {
            print "## [Unreleased]"; print ""
            print "### Added";   print ""
            print "### Changed"; print ""
            print "### Fixed";   print ""
            done = 1
        }
        { print }
    ' CHANGELOG.md > CHANGELOG.md.tmp && mv CHANGELOG.md.tmp CHANGELOG.md
    grep -q '^## \[Unreleased\]' CHANGELOG.md || die "failed to insert [Unreleased] into CHANGELOG.md"
}

# Insert a block of entry text under "### <category>" inside [Unreleased],
# creating that subheading at the end of the section if it isn't there. The
# text is read from a file (not awk -v) so multi-line, pasted content with
# any characters survives verbatim. Assumes [Unreleased] already exists.
insert_entry() {
    local category="$1" textfile="$2"
    awk -v cat="### $category" -v tf="$textfile" '
        # Emit the entry file verbatim, preceded by a blank line for spacing.
        function emit(   line) {
            print ""
            while ((getline line < tf) > 0) print line
            close(tf)
        }
        # Track which top-level section we are in.
        /^## \[/ {
            # Leaving [Unreleased] without having seen the subheading: create
            # it here, at the tail of the section, then emit the entry.
            if (in_unrel && !placed) { print cat; emit(); placed = 1 }
            in_unrel = ($0 ~ /^## \[Unreleased\]/)
            print; next
        }
        # The target subheading inside [Unreleased]: print it, then the entry.
        in_unrel && !placed && index($0, cat) == 1 {
            print; emit(); placed = 1; next
        }
        { print }
        END {
            # [Unreleased] was the last section in the file and had no match.
            if (in_unrel && !placed) { print cat; emit() }
        }
    ' CHANGELOG.md > CHANGELOG.md.tmp && mv CHANGELOG.md.tmp CHANGELOG.md
}


# Apply a patch/minor/major bump to an X.Y.Z version. Pre-release and build
# suffixes are dropped — a bump always lands on a clean release number.
bump() {
    local part="$1" ver="${2%%-*}"
    local major="${ver%%.*}" rest="${ver#*.}"
    local minor="${rest%%.*}" patch="${rest#*.}"
    case "$part" in
        major) printf '%d.0.0\n'   "$((major + 1))" ;;
        minor) printf '%d.%d.0\n'  "$major" "$((minor + 1))" ;;
        patch) printf '%d.%d.%d\n' "$major" "$minor" "$((patch + 1))" ;;
    esac
}

# Resolve a prepare/publish argument into a concrete semver version: a
# bump keyword (patch/minor/major) is applied to the current crate version,
# anything else must already be a semver string. Done after the repo-root
# check so crate_version can be read.
resolve_version() {
    local arg="$1"
    [ -n "$arg" ] || die "$MODE needs a version or a bump keyword: ./release.sh $MODE patch|minor|major|<x.y.z>"
    case "$arg" in
        patch|minor|major) bump "$arg" "$(crate_version)" ;;
        *)  [[ "$arg" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] \
                || die "not a semver version or bump keyword (patch/minor/major): $arg"
            printf '%s\n' "$arg" ;;
    esac
}

$DRY_RUN && printf '%sdry run — nothing will be changed%s\n' "$C_YEL" "$C_OFF"

# ------------------------------------------------------------- shared checks

step "Preconditions"

[ -d .git ] && [ -f "$CRATE_DIR/Cargo.toml" ] && [ -f CHANGELOG.md ] \
    || die "run this from the root of the moor repository"
ok "in the moor repository"

# ================================================================= status
#
# Read-only. Works from any branch — it describes the release, it doesn't
# perform one. Reports each checkpoint of the two-phase flow and ends on the
# single command to run next.

if [ "$MODE" = "status" ]; then
    current=$(crate_version)
    step "Release status"
    note "current version:    $current  (cli/Cargo.toml)"

    # Is there unreleased work staged in the CHANGELOG?
    if grep -q '^## \[Unreleased\]' CHANGELOG.md; then
        if section_has_content '## [Unreleased]'; then
            ok "CHANGELOG:           [Unreleased] has content, ready to prepare"
            unreleased_ready=true
        else
            warn "CHANGELOG:           [Unreleased] is empty — add your changes under Added/Changed/Fixed"
            unreleased_ready=false
        fi
    else
        warn "CHANGELOG:           no [Unreleased] section — run: ./release.sh unreleased"
        unreleased_ready=false
    fi

    # The next version is a guess (patch), shown to make the next command copyable.
    next=$(bump patch "$current")

    # Is a release PR open for some version already?
    open_pr=""
    if command -v gh >/dev/null 2>&1; then
        open_pr=$(gh pr list --repo "$REPO" --state open --head "release/v$next" --json number --jq '.[0].number' 2>/dev/null || true)
    fi

    # Is the current version already published / tagged?
    tag_current="v$current"
    on_crates=false
    curl -fsS -o /dev/null -A "moor-release (github.com/$REPO)" \
        "https://crates.io/api/v1/crates/$CRATE/$current" 2>/dev/null && on_crates=true
    tagged_remote=false
    git ls-remote --exit-code --tags origin "refs/tags/$tag_current" >/dev/null 2>&1 && tagged_remote=true

    if $on_crates && $tagged_remote; then
        ok "published:           $CRATE $current is on crates.io and tagged $tag_current"
    else
        note "published:           $CRATE $current not yet on crates.io / not tagged"
    fi

    step "Next"
    if $tagged_remote && $on_crates && ! $unreleased_ready; then
        note "$current is fully released. Start the next one with:  ./release.sh unreleased"
    elif [ -n "$open_pr" ]; then
        note "A release PR is open (#$open_pr). Merge it, then:  git pull --ff-only && ./release.sh publish $next"
    elif $unreleased_ready; then
        note "Open the release PR:  ./release.sh prepare patch   (→ $next)   or minor/major/<x.y.z>"
    else
        note "List your changes under [Unreleased] first (./release.sh unreleased if it's missing)."
    fi
    printf '\n'
    exit 0
fi

# ================================================================= unreleased
#
# Prepare the CHANGELOG's [Unreleased] section. With no entry text it just
# scaffolds an empty section (Added/Changed/Fixed) when one is missing. Given
# text — inline via --message, or piped/typed on stdin with a trailing "-" —
# it files that text verbatim under --added/--changed/--fixed, creating both
# the section and the subheading as needed. A local edit only: no git, no
# branch, no network, so it runs from any branch. Built for pasting an entry
# an LLM drafted straight in, rather than hand-editing the file.
#
#   ./release.sh unreleased                         # scaffold empty section
#   ./release.sh unreleased --fixed -m "- **X.** …"  # file one entry, inline
#   some-llm | ./release.sh unreleased --added -     # …or piped on stdin

if [ "$MODE" = "unreleased" ]; then
    step "CHANGELOG [Unreleased]"

    # Resolve the entry text, if any. --message wins; otherwise an explicit
    # trailing "-", or a category with piped (non-tty) stdin, supplies it.
    # Reading stdin only when asked keeps the bare `unreleased` scaffold
    # working at an interactive prompt.
    entry=""
    have_entry=false
    if $MESSAGE_SET; then
        entry="$MESSAGE"; have_entry=true
    elif $READ_STDIN; then
        entry="$(cat)"; have_entry=true
    elif [ -n "$CATEGORY" ] && [ ! -t 0 ]; then
        entry="$(cat)"; have_entry=true
    fi

    # Category and text go together — one without the other is a mistake.
    if $have_entry; then
        [ -n "$CATEGORY" ] \
            || die "which section? pass --added, --changed or --fixed with the entry text"
        [ -n "${entry//[$' \t\n']/}" ] \
            || die "the entry text is empty — nothing to add under ### $CATEGORY"
    elif [ -n "$CATEGORY" ]; then
        die "--$(printf '%s' "$CATEGORY" | tr '[:upper:]' '[:lower:]') needs entry text: add -m \"…\" or pipe it in with a trailing -"
    fi

    existed=false
    grep -q '^## \[Unreleased\]' CHANGELOG.md && existed=true

    if $DRY_RUN; then
        $existed || note "would insert an empty [Unreleased] section (Added / Changed / Fixed)"
        $have_entry && note "would file the entry under ### $CATEGORY"
        note "dry run — CHANGELOG.md not changed"
        printf '\n'
        exit 0
    fi

    scaffold_unreleased
    $existed || ok "added [Unreleased] to CHANGELOG.md"

    if $have_entry; then
        tmp=$(mktemp)
        # shellcheck disable=SC2064  # expand tmp now, not at trap time
        trap "rm -f '$tmp'" EXIT
        printf '%s\n' "$entry" > "$tmp"
        insert_entry "$CATEGORY" "$tmp"
        section_has_content '## [Unreleased]' \
            || die "entry did not land under ### $CATEGORY — CHANGELOG.md left unchanged on disk? check it"
        ok "filed the entry under ### $CATEGORY"
        printf '\n%sReview it, then:%s  ./release.sh prepare patch\n\n' "$C_GRN" "$C_OFF"
    else
        $existed && ok "[Unreleased] already exists — add your changes under its headings"
        $existed && ! section_has_content '## [Unreleased]' && warn "it's currently empty"
        printf '\n%sList changes under Added / Changed / Fixed (or pass --fixed -m \"…\"), then:%s  ./release.sh prepare patch\n\n' \
            "$C_GRN" "$C_OFF"
    fi
    exit 0
fi

# ---- from here down: prepare and publish, which act on the real release ----

# Resolve the version now that crate_version is readable; set the derived refs.
VERSION=$(resolve_version "$VERSION_ARG")
readonly VERSION
readonly TAG="v${VERSION}"
readonly BRANCH="release/${TAG}"
note "target version:     $VERSION  ($TAG)"

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
        || die "CHANGELOG.md has no '## [Unreleased]' section — run: ./release.sh unreleased, then list the changes"
    section_has_content '## [Unreleased]' \
        || die "CHANGELOG.md's [Unreleased] section is empty — add your changes under Added/Changed/Fixed first"
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
