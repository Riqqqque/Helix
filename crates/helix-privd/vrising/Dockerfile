FROM debian:bookworm-slim AS shutdown-build
RUN apt-get update && apt-get install -y --no-install-recommends gcc-mingw-w64-x86-64 && rm -rf /var/lib/apt/lists/*
COPY shutdown.c /build/shutdown.c
RUN x86_64-w64-mingw32-gcc -Wall -Wextra -Werror -Os -s /build/shutdown.c -o /build/shutdown.exe
COPY launcher.c /build/launcher.c
RUN x86_64-w64-mingw32-gcc -Wall -Wextra -Werror -Os -s -municode -mwindows /build/launcher.c -o /build/launcher.exe

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
COPY --from=shutdown-build /build/shutdown.exe /usr/local/libexec/helix-vrising-shutdown.exe
COPY --from=shutdown-build /build/launcher.exe /usr/local/libexec/helix-vrising-launch.exe
RUN chmod 0755 /usr/local/bin/helix-vrising
WORKDIR /data
ENTRYPOINT ["/usr/local/bin/helix-vrising"]
