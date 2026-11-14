# Builds BlueZ 5.72's btvirt. Usage:
#   docker build -t bluebus-btvirt tools/btvirt
#   docker run -d --rm --name bluebus-vhci --privileged bluebus-btvirt -l2   # adds two virtual controllers to the host BlueZ
#   cargo run --example virtual_e2e -- hci1 hci2
#   docker stop bluebus-vhci
FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      build-essential pkg-config libglib2.0-dev libdbus-1-dev libudev-dev libical-dev libreadline-dev \
      python3-docutils wget ca-certificates xz-utils && rm -rf /var/lib/apt/lists/*
RUN wget -q https://www.kernel.org/pub/linux/bluetooth/bluez-5.72.tar.xz && tar xf bluez-5.72.tar.xz \
 && cd bluez-5.72 && ./configure --enable-testing --disable-systemd --disable-cups --disable-obex --disable-manpages --disable-udev \
 && make -j"$(nproc)" && cp emulator/btvirt /usr/local/bin/ && cd / && rm -rf bluez-5.72*
ENTRYPOINT ["/usr/local/bin/btvirt"]
