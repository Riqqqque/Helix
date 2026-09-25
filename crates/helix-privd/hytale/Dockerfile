FROM eclipse-temurin:25-jre
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    jq \
    unzip \
 && rm -rf /var/lib/apt/lists/*
COPY entrypoint.sh /usr/local/bin/helix-hytale
RUN chmod 0755 /usr/local/bin/helix-hytale
WORKDIR /data
ENTRYPOINT ["/usr/local/bin/helix-hytale"]
