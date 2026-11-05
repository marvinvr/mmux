# mmux.org static site — nginx serving plain HTML/CSS/JS.
# Pinned to a recent stable alpine release.
FROM nginx:1.29-alpine@sha256:5616878291a2eed594aee8db4dade5878cf7edcb475e59193904b198d9b830de

# Static assets (per DESIGN.md §8). DESIGN.md / README.md are NOT copied.
COPY index.html styles.css tui.js scenes.js /usr/share/nginx/html/
COPY fonts/ /usr/share/nginx/html/fonts/
COPY assets/ /usr/share/nginx/html/assets/

# The "mmux vs X" comparison pages: /vs/ + one vs/<slug>/index.html each, and
# their shared vs.css. vs/README.md and vs/_template/ stay out (.dockerignore).
COPY vs/ /usr/share/nginx/html/vs/

# Agent/SEO files served from the root with stable (un-fingerprinted) URLs:
# robots.txt for crawlers, llms.txt as an LLM-readable guide, banner.txt as
# the plain-text answer nginx serves to curl/wget on `/`.
COPY robots.txt llms.txt banner.txt /usr/share/nginx/html/

# sitemap.xml is generated, not committed: the home page plus every vs/<slug>/
# page that exists in this build (lastmod from each page's dateModified).
COPY scripts/gen-sitemap.sh /tmp/gen-sitemap.sh
RUN sh /tmp/gen-sitemap.sh /usr/share/nginx/html > /usr/share/nginx/html/sitemap.xml && \
    rm /tmp/gen-sitemap.sh

# The install script, served at a stable URL: `curl -fsSL https://mmux.org/install.sh | sh`.
COPY install.sh /usr/share/nginx/html/

# Fingerprint the assets: rename each to <name>.<contenthash>.<ext> and rewrite
# the quoted references in every HTML page to match. This is what makes the 1y
# `immutable` cache in nginx.conf safe — a changed file gets a new hash, hence a
# new URL, so browsers and Cloudflare fetch it fresh; unchanged files keep their
# hash and stay cached. No cache purging on deploy. References are matched by
# basename after a quote or a slash and before the closing quote, so `"styles.css"`,
# `"../../styles.css"` and `"../vs.css"` are all rewritten, while the plain-text
# mentions of these filenames in comments are not. That needs the basenames to be
# unique across the site, which they are.
RUN cd /usr/share/nginx/html && \
    for f in styles.css scenes.js tui.js vs/vs.css; do \
      h=$(sha256sum "$f" | cut -c1-8); \
      b=$(basename "$f"); \
      nb="${b%.*}.$h.${b##*.}"; \
      mv "$f" "$(dirname "$f")/$nb"; \
      find . -name '*.html' -exec sed -i "s#\\([\"/]\\)${b%.*}\\.${b##*.}\"#\\1$nb\"#g" {} +; \
    done

# Server block: cache headers, gzip, strict CSP (see nginx.conf).
COPY nginx.conf /etc/nginx/conf.d/default.conf

EXPOSE 80

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD wget -q -O /dev/null http://127.0.0.1/ || exit 1
