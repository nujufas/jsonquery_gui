# Throwaway image for smoke-testing the arm64 snap's binary on an amd64 host
# without registering binfmt handlers on the host: run-gui.sh chroots into
# /sysroot (Ubuntu 24.04 arm64 libs, standing in for core24 + the gnome content
# snap) and runs the binary through the static qemu-aarch64. See
# packaging/snap/README.md.
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive

RUN sed -i 's/^Types: deb$/Types: deb\nArchitectures: amd64/' /etc/apt/sources.list.d/ubuntu.sources \
 && printf 'Types: deb\nURIs: http://ports.ubuntu.com/ubuntu-ports\nSuites: noble noble-updates\nComponents: main universe\nArchitectures: arm64\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n' \
      > /etc/apt/sources.list.d/ports.sources \
 && dpkg --add-architecture arm64 \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
      qemu-user-static xvfb x11-utils xauth ca-certificates procps imagemagick

# Download (not install) the arm64 runtime closure, then unpack it into /sysroot.
RUN mkdir -p /dl/partial /sysroot \
 && apt-get install -y --no-install-recommends --download-only -o Dir::Cache::archives=/dl \
      libc6:arm64 libgcc-s1:arm64 libstdc++6:arm64 \
      libx11-6:arm64 libxcb1:arm64 libxcursor1:arm64 libxi6:arm64 \
      libxrandr2:arm64 libxinerama1:arm64 libxkbcommon0:arm64 libxkbcommon-x11-0:arm64 \
      libwayland-client0:arm64 libwayland-cursor0:arm64 libwayland-egl1:arm64 \
      libegl1:arm64 libgl1:arm64 libglx0:arm64 libgles2:arm64 libvulkan1:arm64 \
      libegl-mesa0:arm64 libglx-mesa0:arm64 libgl1-mesa-dri:arm64 mesa-vulkan-drivers:arm64 \
      xkb-data \
 && for d in /dl/*_arm64.deb /dl/*_all.deb; do dpkg -x "$d" /sysroot; done \
 && du -sh /sysroot

# noble's debs already use the merged-/usr layout; make /lib resolve too so the
# ELF interpreter /lib/ld-linux-aarch64.so.1 is found inside the chroot.
RUN if [ -d /sysroot/lib ] && [ ! -L /sysroot/lib ]; then \
        cp -a /sysroot/lib/. /sysroot/usr/lib/ && rm -rf /sysroot/lib; \
    fi; \
    if [ ! -e /sysroot/lib ]; then ln -s usr/lib /sysroot/lib; fi; \
    ls -l /sysroot/lib/ld-linux-aarch64.so.1 /sysroot/usr/lib/ld-linux-aarch64.so.1 2>&1; \
    test -e /sysroot/lib/ld-linux-aarch64.so.1

# xkb-data is already installed in this image (Xvfb depends on it), so the
# --download-only step above skips it; fetch it explicitly for the sysroot.
RUN cd /tmp && apt-get download xkb-data \
 && dpkg -x xkb-data_*_all.deb /sysroot \
 && test -d /sysroot/usr/share/X11/xkb
