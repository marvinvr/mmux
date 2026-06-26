#!/usr/bin/env bash
#
# Purge the Cloudflare cache for mmux.org.
#
# mmux.org is a static nginx site (see web/) served behind Cloudflare. After a
# deploy, the edge may still hand out stale HTML/CSS/JS until its cache expires;
# this forces a full purge so the new build goes live immediately.
#
# Usage:
#   CLOUDFLARE_API_KEY=... scripts/clear-cloudflare-cache.sh
#
# Env:
#   CLOUDFLARE_API_KEY   (required)  A Cloudflare API *token* with the
#                                    "Zone › Cache Purge › Edit" permission on
#                                    the mmux.org zone. Used as a Bearer token.
#   CLOUDFLARE_ZONE      (optional)  Domain to purge. Default: mmux.org
#   CLOUDFLARE_ZONE_ID   (optional)  Zone ID. If unset, it is resolved from the
#                                    domain name via the API.
#
# The token is never stored in the repo — pass it via the environment.

set -euo pipefail

API="https://api.cloudflare.com/client/v4"
ZONE="${CLOUDFLARE_ZONE:-mmux.org}"

if [[ -z "${CLOUDFLARE_API_KEY:-}" ]]; then
	echo "Error: CLOUDFLARE_API_KEY environment variable is not set" >&2
	exit 1
fi

auth=(-H "Authorization: Bearer ${CLOUDFLARE_API_KEY}" -H "Content-Type: application/json")

# Resolve the zone ID from the domain name unless one was supplied.
zone_id="${CLOUDFLARE_ZONE_ID:-}"
if [[ -z "$zone_id" ]]; then
	resp="$(curl -fsS "${auth[@]}" "${API}/zones?name=${ZONE}")"
	zone_id="$(printf '%s' "$resp" | sed -n 's/.*"result":\[{"id":"\([0-9a-f]*\)".*/\1/p')"
	if [[ -z "$zone_id" ]]; then
		echo "Error: could not find a Cloudflare zone named '${ZONE}' for this token." >&2
		echo "       The token must have access to the '${ZONE}' zone (Cache Purge › Edit)." >&2
		echo "       Cloudflare said: ${resp}" >&2
		exit 1
	fi
fi

echo "Clearing Cloudflare cache for ${ZONE} (zone ${zone_id})..."

resp="$(curl -fsS -X POST "${auth[@]}" \
	"${API}/zones/${zone_id}/purge_cache" \
	--data '{"purge_everything":true}')"

if printf '%s' "$resp" | grep -q '"success":true'; then
	echo "✓ Cache cleared successfully!"
else
	echo "Error clearing cache: ${resp}" >&2
	exit 1
fi
