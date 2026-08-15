FROM oven/bun:1.3.14 AS frontend
WORKDIR /build/frontend
COPY frontend/package.json frontend/bun.lock ./
RUN bun install --frozen-lockfile
COPY frontend/ ./
RUN bun run build

FROM rust:1-bookworm AS backend
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --locked --release \
    && cp target/release/opencode2api /build/opencode2api

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        chromium \
        chromium-sandbox \
        fonts-liberation \
        fonts-noto-cjk \
        fonts-noto-color-emoji \
        gosu \
        libssl3 \
        x11vnc \
        xvfb \
    && rm -rf /var/lib/apt/lists/* \
    && fc-list :lang=zh family | grep -q "Noto Sans CJK" \
    && mkdir -p /app/frontend /data

WORKDIR /app
COPY --from=backend /build/opencode2api /usr/local/bin/opencode2api
COPY --from=frontend /build/frontend/dist ./frontend/dist
COPY docker/chromium-wrapper.sh /usr/local/bin/opencode2api-chromium
RUN chmod 0755 /usr/local/bin/opencode2api-chromium

ENV OPENCODE2API_BIND=0.0.0.0:8787 \
    OPENCODE2API_CHROMIUM_BIN=/usr/local/bin/opencode2api-chromium \
    OPENCODE2API_DATA_DIR=/data \
    OPENCODE2API_WEB_DIST=/app/frontend/dist

VOLUME ["/data"]
EXPOSE 8787
ENTRYPOINT ["opencode2api"]
