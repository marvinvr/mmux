# mmux.org static site — nginx serving plain HTML/CSS/JS.
# Pinned to a recent stable alpine release.
FROM nginx:1.27-alpine

# Static assets (per DESIGN.md §8). DESIGN.md / README.md are NOT copied.
COPY index.html styles.css tui.js scenes.js /usr/share/nginx/html/
COPY fonts/ /usr/share/nginx/html/fonts/

# Server block: cache headers, gzip, strict CSP (see nginx.conf).
COPY nginx.conf /etc/nginx/conf.d/default.conf

EXPOSE 80

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD wget -q -O /dev/null http://127.0.0.1/ || exit 1
