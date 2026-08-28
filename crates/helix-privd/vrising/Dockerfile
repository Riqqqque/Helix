FROM debian:bookworm-slim
ENV DEBIAN_FRONTEND=noninteractive
RUN dpkg --add-architecture i386 \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      cabextract \
      xvfb \
      xauth \
      lib32gcc-s1 \
      wine \
      wine64 \
 && rm -rf /var/lib/apt/lists/* \
 && mkdir -p /opt/steamcmd \
 && curl -fsSL -o /tmp/steamcmd_linux.tar.gz https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz \
 && tar -xzf /tmp/steamcmd_linux.tar.gz -C /opt/steamcmd \
 && rm /tmp/steamcmd_linux.tar.gz \
 && chmod -R a+rX /opt/steamcmd
COPY entrypoint.sh /usr/local/bin/helix-vrising
RUN chmod 0755 /usr/local/bin/helix-vrising
WORKDIR /data
ENTRYPOINT ["/usr/local/bin/helix-vrising"]
