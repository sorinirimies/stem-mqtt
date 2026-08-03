# Minimal static file server for demo/web/, so the browser demo works via
# `docker compose up` without any local Node/Python setup.
FROM nginx:1.27-alpine
COPY demo/web/ /usr/share/nginx/html/
EXPOSE 80
