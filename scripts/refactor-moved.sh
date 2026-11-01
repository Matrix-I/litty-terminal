#!/usr/bin/env bash
# Residual of a refactor commit: lines removed and added that do NOT cancel out
# after trimming indentation and visibility keywords. A pure move prints nothing.
# Usage: scripts/refactor-moved.sh [<base>] [<head>]   (default: HEAD~1 HEAD)
set -eo pipefail
base=${1:-HEAD~1}; head=${2:-HEAD}
norm() {
    git diff -U0 --no-color "$base" "$head" -- src |
        grep -E '^[-+]' | grep -vE '^(\+\+\+|---)' |
        sed -E 's/^([-+])[[:space:]]+/\1/; s/\b(pub\((crate|super|in [a-z_:]+)\)|pub) //g' |
        grep -vE '^[-+](use |mod |//)' | grep -vE '^[-+]\}?$'
}
{ norm | grep '^-' | sed 's/^-//' | sort | sed 's/^/R /'
  norm | grep '^+' | sed 's/^+//' | sort | sed 's/^/A /'; } |
    awk '{k=substr($0,3); if ($1=="R") r[k]++; else a[k]++}
         END {for (k in r) if (r[k]>a[k]) printf "only-removed x%d: %s\n", r[k]-a[k], k
              for (k in a) if (a[k]>r[k]) printf "only-added   x%d: %s\n", a[k]-r[k], k}' | sort
