#!/usr/bin/env sh
# Write the changelog section for a release from the conventional commits.
#
# Usage:
#   scripts/changelog.sh [--since <tag>] [--version <version>] [--date <YYYY-MM-DD>]
#
# Without --since, the section covers every commit since the most recent tag.
# Without --version, the tag is the version. The output is one Keep a Changelog
# section: a heading, the date, and the commits grouped by their
# conventional-commit type.
#
# The rule the script follows is the one a reader cares about: a commit that
# changes nothing a user can see is not in the changelog. `chore`, `ci`,
# `test` and `build` commits are build machinery, and a user who reads a
# changelog does not want them.

set -eu

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

repo=$(git rev-parse --show-toplevel 2>/dev/null || true)
[ -n "$repo" ] || die "not inside a git repository"
cd "$repo"

since=""
since_set=0
version=""
date=$(date -u +%Y-%m-%d)

while [ "$#" -gt 0 ]; do
    case "$1" in
        --since)
            [ "$#" -ge 2 ] || die "--since needs a value"
            since=$2
            since_set=1
            shift 2
            ;;
        --version)
            [ "$#" -ge 2 ] || die "--version needs a value"
            version=$2
            shift 2
            ;;
        --date)
            [ "$#" -ge 2 ] || die "--date needs a value"
            date=$2
            shift 2
            ;;
        -h|--help)
            printf '%s\n' "usage: scripts/changelog.sh [--since <tag>] [--version <version>] [--date <YYYY-MM-DD>]"
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
done

if [ "$since_set" -eq 0 ]; then
    since=$(git describe --tags --abbrev=0 2>/dev/null || true)
fi

if [ -z "$version" ]; then
    if [ -n "$since" ]; then
        version=${since#v}
    else
        version="0.0.0"
    fi
fi

# The range: everything after the last tag, up to HEAD.
if [ -n "$since" ]; then
    range="$since..HEAD"
else
    range="HEAD"
fi

# The log, one commit per line: the hash, the type, and the subject.
log=$(git log --format='%h%x09%s' "$range" 2>/dev/null || true)

printf '## %s - %s\n' "$version" "$date"

if [ -z "$log" ]; then
    printf '\n_No changes._\n'
    exit 0
fi

section() {
    # section <heading> <type>...
    heading=$1
    shift
    prefixes=$*
    entries=$(printf '%s\n' "$log" | awk -F'\t' -v prefixes="$prefixes" '
        BEGIN { n = split(prefixes, list, " ") }
        {
            for (i = 1; i <= n; i++) {
                p = list[i]
                if ($2 ~ "^" p "(\\([^)]*\\))?!?: ") {
                    line = $2
                    sub("^" p "(\\([^)]*\\))?!?: ", "", line)
                    printf "- %s ([%s](%s/commit/%s))\n", line, $1, ENVIRON["REPO"], $1
                    next
                }
            }
        }
    ')
    if [ -n "$entries" ]; then
        printf '\n### %s\n\n%s\n' "$heading" "$entries"
    fi
}

REPO=${YASHY_REPO_URL:-https://github.com/Yash-K-Jagani/Yashy}
export REPO

# A breaking change is called out first: it is the only kind of change that
# can make a working program stop working.
breaking=$(printf '%s\n' "$log" | awk -F'\t' '
    $2 ~ /^[a-z]+(\([^)]*\))?!: / {
        line = $2
        sub(/^[a-z]+(\([^)]*\))?!: /, "", line)
        printf "- %s ([%s](%s/commit/%s))\n", line, $1, ENVIRON["REPO"], $1
    }
')
if [ -n "$breaking" ]; then
    printf '\n### Breaking changes\n\n%s\n' "$breaking"
fi

section "Added" "feat"
section "Fixed" "fix"
section "Changed" "perf" "refactor" "docs" "build"
