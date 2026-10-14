#!/bin/sh
# Print sitemap.xml for the site to stdout: the home page, plus /vs/ and one
# entry per comparison page once at least one vs/<slug>/index.html exists.
# Folders starting with "_" (the template) are skipped. A page's <lastmod> is
# the "dateModified" (YYYY-MM-DD) from its WebPage JSON-LD; without a valid
# one the entry simply has no <lastmod>.
#
# Runs in the Docker build (busybox sh/sed/grep), so stay POSIX:
#   sh scripts/gen-sitemap.sh [webroot] > sitemap.xml
# Locally, to preview:  sh scripts/gen-sitemap.sh
set -eu

root=${1:-$(dirname "$0")/..}
site=https://mmux.org

url() { # <loc> [lastmod]
  printf '  <url>\n    <loc>%s</loc>\n' "$1"
  [ -n "${2:-}" ] && printf '    <lastmod>%s</lastmod>\n' "$2"
  printf '  </url>\n'
}

printf '<?xml version="1.0" encoding="UTF-8"?>\n'
printf '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n'
url "$site/"

pages=$(find "$root/vs" -mindepth 2 -maxdepth 2 -name index.html 2>/dev/null | sort)
latest=
entries=
for f in $pages; do
  slug=$(basename "$(dirname "$f")")
  case $slug in _*) continue ;; esac
  d=$(sed -n 's/.*"dateModified"[[:space:]]*:[[:space:]]*"\([0-9]\{4\}-[0-9]\{2\}-[0-9]\{2\}\)".*/\1/p' "$f" | head -n 1)
  # /vs/ takes the newest page's date (YYYYMMDD compares as an integer).
  if [ -n "$d" ] && { [ -z "$latest" ] ||
    [ "$(echo "$d" | tr -d -)" -gt "$(echo "$latest" | tr -d -)" ]; }; then latest=$d; fi
  entries="$entries$slug $d
"
done

if [ -n "$entries" ]; then
  url "$site/vs/" "$latest"
  printf '%s' "$entries" | while read -r slug d; do
    url "$site/vs/$slug/" "$d"
  done
fi

printf '</urlset>\n'
