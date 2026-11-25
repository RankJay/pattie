#!/usr/bin/env bash
# Push each ruleset in this folder to GitHub: update the one with the same name, or
# create it. GitHub doesn't read these files itself; this script is the only way they
# take effect. Needs `gh` signed in as a repository admin.
#
#   .github/rulesets/apply.sh            # apply every *.json here
#   .github/rulesets/apply.sh --dry-run  # show what would be created or updated
set -euo pipefail

dir="$(cd "$(dirname "$0")" && pwd)"
repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"
dry_run=0
[[ "${1:-}" == "--dry-run" ]] && dry_run=1

for file in "$dir"/*.json; do
  # The top-level "name" is the only key indented two spaces with that spelling.
  name="$(sed -n 's/^  "name": "\(.*\)",$/\1/p' "$file")"
  if [[ -z "$name" ]]; then
    echo "skip $file: no top-level \"name\"" >&2
    continue
  fi
  id="$(gh api "repos/$repo/rulesets" --paginate \
    --jq ".[] | select(.name == \"$name\" and .source_type == \"Repository\") | .id")"

  if [[ -n "$id" ]]; then
    echo "update ruleset '$name' (id $id) from $(basename "$file")"
    ((dry_run)) || gh api --method PUT "repos/$repo/rulesets/$id" --input "$file" --silent
  else
    echo "create ruleset '$name' from $(basename "$file")"
    ((dry_run)) || gh api --method POST "repos/$repo/rulesets" --input "$file" --silent
  fi
done
