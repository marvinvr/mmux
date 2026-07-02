# mmux.org static site — nginx serving plain HTML/CSS/JS.
# Pinned to a recent stable alpine release.
FROM nginx:1.27-alpine

# Static assets (per DESIGN.md §8). DESIGN.md / README.md are NOT copied.
COPY index.html styles.css tui.js scenes.js /usr/share/nginx/html/
COPY fonts/ /usr/share/nginx/html/fonts/
COPY assets/ /usr/share/nginx/html/assets/

# Agent/SEO files served from the root with stable (un-fingerprinted) URLs:
# robots.txt + sitemap.xml for crawlers, llms.txt as an LLM-readable guide,
# banner.txt as the plain-text answer nginx serves to curl/wget on `/`.
COPY robots.txt sitemap.xml llms.txt banner.txt /usr/share/nginx/html/

# Fingerprint the assets: rename each to <name>.<contenthash>.<ext> and rewrite
# the quoted references in index.html to match. This is what makes the 1y
# `immutable` cache in nginx.conf safe — a changed file gets a new hash, hence a
# new URL, so browsers and Cloudflare fetch it fresh; unchanged files keep their
# hash and stay cached. No cache purging on deploy. The quotes in the sed match
# the real `src="…"`/`href="…"` references, never the plain-text mentions of
# these filenames in index.html's comments.
RUN cd /usr/share/nginx/html && \
    for f in styles.css scenes.js tui.js; do \
      h=$(sha256sum "$f" | cut -c1-8); \
      new="${f%.*}.$h.${f##*.}"; \
      mv "$f" "$new"; \
      sed -i "s|\"$f\"|\"$new\"|g" index.html; \
    done

# Server block: cache headers, gzip, strict CSP (see nginx.conf).
COPY nginx.conf /etc/nginx/conf.d/default.conf

EXPOSE 80

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD wget -q -O /dev/null http://127.0.0.1/ || exit 1
