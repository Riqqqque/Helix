# syntax=docker/dockerfile:1.7

FROM node:24-bookworm-slim@sha256:a9f5f7c91a432850b2a8a7797adf5eadb6c733ceed61167806cee7ea7fbc29df AS frontend-build
WORKDIR /build
COPY frontend/package.json frontend/package-lock.json ./frontend/
RUN --mount=type=cache,target=/root/.npm \
    cd frontend && npm ci --no-audit --no-fund
COPY frontend ./frontend
COPY scripts/precompress-assets.mjs ./scripts/precompress-assets.mjs
RUN cd frontend && npm run build

FROM rust:1.88-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0 AS rust-build
WORKDIR /build
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY crates ./crates
COPY examples ./examples
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --locked --release -p helixd -p helixctl -p helix-privd -p helix-terminal && \
    install -m 0755 target/release/helixd /tmp/helixd && \
    install -m 0755 target/release/helixctl /tmp/helixctl && \
    install -m 0755 target/release/helix-privd /tmp/helix-privd && \
    install -m 0755 target/release/helix-terminald /tmp/helix-terminald

FROM rust-build AS linux-test
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    rustup component add rustfmt clippy && \
    cargo fmt --all -- --check && \
    cargo clippy --locked --workspace --all-targets --all-features -- -D warnings && \
    cargo test --locked --workspace --all-targets --all-features -- --include-ignored

FROM scratch AS privd
COPY --from=rust-build /tmp/helix-privd /helix-privd

FROM scratch AS terminald
COPY --from=rust-build /tmp/helix-terminald /helix-terminald

FROM debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171 AS dashboard-layout
RUN install -d -m 0755 \
      /layout/app/bin \
      /layout/app/config \
      /layout/app/lib \
      /layout/app/web && \
    install -d -o 10001 -g 10001 -m 0700 \
      /layout/var/lib/helix \
      /layout/var/lib/helix-backups

COPY --from=rust-build /tmp/helixd /layout/app/bin/helixd
COPY --from=rust-build /tmp/helixctl /layout/app/bin/helixctl
COPY --from=rust-build /usr/lib/*/libgcc_s.so.1 /layout/app/lib/libgcc_s.so.1
COPY --from=frontend-build /build/frontend/dist /layout/app/web
RUN find /layout/app/web -type d -exec chmod 0755 {} + && \
    find /layout/app/web -type f -exec chmod 0444 {} +
COPY --chmod=0444 deploy/helix-container.toml /layout/app/config/helix.toml
COPY --chmod=0444 LICENSE /layout/app/LICENSE

FROM gcr.io/distroless/base-nossl-debian12:nonroot@sha256:be40c00dfabd86576d92666e87e406714d5618342de1a0c213ad232de255172e AS dashboard
ARG HELIX_SOURCE_REVISION=unknown
LABEL org.opencontainers.image.title="Helix dashboard" \
      org.opencontainers.image.description="Local-first Linux server control plane" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later" \
      org.opencontainers.image.source="https://github.com/Riqqqque/Helix" \
      org.opencontainers.image.revision="${HELIX_SOURCE_REVISION}"

COPY --from=dashboard-layout --chown=0:0 /layout/app /app
COPY --from=dashboard-layout --chown=10001:10001 --chmod=0700 /layout/var/lib/helix /var/lib/helix
COPY --from=dashboard-layout --chown=10001:10001 --chmod=0700 /layout/var/lib/helix-backups /var/lib/helix-backups

ENV LD_LIBRARY_PATH=/app/lib
USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["/app/bin/helixd"]
CMD ["--config", "/app/config/helix.toml"]

FROM nginxinc/nginx-unprivileged:stable-alpine@sha256:93722936b82ec8a1178d48448e619226680d2de3706a1640800e186cd5fa7fd3 AS gateway
ARG HELIX_SOURCE_REVISION=unknown
LABEL org.opencontainers.image.title="Helix LAN gateway" \
      org.opencontainers.image.description="Strict LAN-only reverse-proxy boundary for Helix" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later" \
      org.opencontainers.image.source="https://github.com/Riqqqque/Helix" \
      org.opencontainers.image.revision="${HELIX_SOURCE_REVISION}"
USER root
RUN apk add --no-cache --upgrade \
      libcrypto3=3.5.8-r0 \
      libssl3=3.5.8-r0 && \
    sed -i 's/^worker_processes  *auto;/worker_processes 1;/' /etc/nginx/nginx.conf && \
    install -d -o 101 -g 101 -m 0755 /etc/nginx/templates
COPY --chown=101:101 --chmod=0444 deploy/nginx/default.conf.template /etc/nginx/templates/default.conf.template
USER 101
EXPOSE 3000
