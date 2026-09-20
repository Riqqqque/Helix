FROM debian:bookworm-slim
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    xz-utils \
 && rm -rf /var/lib/apt/lists/*
COPY entrypoint.sh /usr/local/bin/helix-factorio
RUN chmod 0755 /usr/local/bin/helix-factorio
WORKDIR /data
ENTRYPOINT ["/usr/local/bin/helix-factorio"]
