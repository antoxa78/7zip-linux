# 7-Zip Linux

A Linux GUI for 7-Zip, inspired by the Windows File Manager.

![7-Zip Linux](data/7zip-linux.png)

## Features

- **Create archives** — 7z, zip, tar, tar.gz, tar.bz2, tar.xz, tar.zst
- **Extract archives** — 7z, rar, zip, tar, gz, bz2, xz, zst, lz4
- **Browse archives** — navigate archive contents like a regular directory
- **Password protection** — create and open encrypted archives
- **Drag & drop** — copy and move files between locations
- **File manager integration** — right-click scripts for Nautilus, Nemo, Thunar, Dolphin
- **Search with glob patterns** — `*.deb`, `*.tar.gz`, `doc*`, etc.
- **Compression presets** — Store, Fastest, Fast, Normal, Maximum, Ultra

## Installation

### Debian/Ubuntu

```bash
sudo dpkg -i release/7zip-linux_1.0.0_amd64.deb
sudo apt install -f  # install dependencies
```

### Dependencies

- `p7zip-full` — the 7z command-line tool
- `zenity` — dialogs for file manager scripts (optional)

## Build from source

```bash
cargo build --release
./target/release/7zip-linux
```

Requires Rust, GTK4, and libadwaita development packages.

## File Manager Integration

From the hamburger menu → **File Associations** → **Install File Manager Scripts**:

| File Manager | Location |
|---|---|
| Nautilus | `~/.local/share/nautilus/scripts/` |
| Nemo | `~/.local/share/nemo/scripts/` |
| Thunar | `~/.config/Thunar/uca.xml` |
| Dolphin | `~/.local/share/kservices5/servicemenus/` |

## License

GPL-3.0-or-later
