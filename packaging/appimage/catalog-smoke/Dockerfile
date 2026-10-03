# A bare Ubuntu 22.04 system shaped like the runner the AppImage catalog tests
# new entries on (https://github.com/AppImage/appimage.github.io, test.yml runs
# on ubuntu-22.04): Xvfb, software GL from Mesa, no Vulkan driver, gtk3 (which
# is why the base libxkbcommon0 is present) but no libxkbcommon-x11. See
# packaging/appimage/README.md.
#
# HOST picks what the system has of the xkbcommon family, which is what the
# AppImage's fallback libraries are for:
#   ci       libxkbcommon0 only: the catalog's runner (the app used to panic here)
#   desktop  the full family: an ordinary desktop, where the bundled copies must
#            stay unused
#   bare     none of it
#
# libfuse2 is deliberately NOT installed (the catalog installs it for its own
# tooling): the AppImage must not need it.
FROM ubuntu:22.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      xvfb icewm x11-utils imagemagick xdotool \
      libgl1-mesa-dri libgl1-mesa-dev mesa-utils libosmesa6 \
      libgtk-3-0 libgbm-dev fonts-wqy-microhei ca-certificates \
    && rm -rf /var/lib/apt/lists/*

ARG HOST=ci
RUN case "$HOST" in \
      ci) ;; \
      desktop) apt-get update \
        && apt-get install -y --no-install-recommends libxkbcommon-x11-0 \
        && rm -rf /var/lib/apt/lists/* ;; \
      bare) dpkg --force-depends --remove libxkbcommon0 && ldconfig ;; \
      *) echo "unknown HOST '$HOST'" >&2; exit 1 ;; \
    esac

COPY run.sh /usr/local/bin/catalog-smoke-run
