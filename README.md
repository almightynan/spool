<!--
SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# spool

Manage network printers and scanners on Linux. Finds printers and scanners on the network as they appear and disappear, or by IP address, host name or `ipp://` URI

## Install

Download `spool-0.1.0-x86_64.flatpak` from the
[releases page](https://github.com/almightynan/spool/releases) and run:

```sh
flatpak install --user spool-0.1.0-x86_64.flatpak
flatpak run io.github.almightynan.Spool
```

This needs Flatpak and the Flathub remote, which most distributions have set
up already (see <https://flathub.org/setup>). The GNOME runtime downloads
automatically. The bundle is for x86_64 PCs.

## Build

### Flatpak

```sh
flatpak install --user flathub org.flatpak.Builder org.gnome.Sdk//51 org.gnome.Platform//51 \
    org.freedesktop.Sdk.Extension.rust-stable//26.08
flatpak run org.flatpak.Builder --user --install --force-clean build-dir build-aux/io.github.almightynan.Spool.json
flatpak run io.github.almightynan.Spool
```

After changing dependencies, regenerate the offline crate list:

```sh
build-aux/cargo-sources.py Cargo.lock > build-aux/cargo-sources.json
```

### Native

Needs Rust 1.85 or newer and the development files for GTK 4.14+ and
libadwaita 1.6+ (Debian 13: `libgtk-4-dev libadwaita-1-dev`; Fedora:
`gtk4-devel libadwaita-devel`).

```sh
cargo build --release
./target/release/spool
```

## Tests

```sh
cargo test --workspace
```


## Permissions for CUPS administration

Adding and removing queues and setting the default printer need CUPS
administrator rights. the app authenticates with peer credentials over the
CUPS socket, so members of the CUPS system group (usually `lpadmin`) need no
password. Others see a message saying so.

## License

GPL-3.0-or-later.
