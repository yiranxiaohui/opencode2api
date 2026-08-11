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
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 opencode2api \
    && useradd --system --uid 10001 --gid opencode2api --home-dir /app opencode2api \
    && mkdir -p /app/frontend /data \
    && chown -R opencode2api:opencode2api /app /data

WORKDIR /app
COPY --from=backend /build/opencode2api /usr/local/bin/opencode2api
COPY --from=frontend /build/frontend/dist ./frontend/dist

ENV OPENCODE2API_BIND=0.0.0.0:8787 \
    OPENCODE2API_DATA_DIR=/data \
    OPENCODE2API_WEB_DIST=/app/frontend/dist

USER opencode2api
VOLUME ["/data"]
EXPOSE 8787
ENTRYPOINT ["opencode2api"]
