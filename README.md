# ImpHash - Duplicate Image Finder

Finds near-duplicate images using perceptual hashing (dHash). Built with GTK4 and libadwaita.

## Features

- **Perceptual hashing** — detects near-duplicates even with different compression, resizing, or minor edits
- **Rotation-invariant matching** — optionally detects rotated copies (90°, 180°, 270°)
- **Reference directory support** — mark a directory as reference to protect originals
- **Select By** — smart selection: biggest/smallest by size, resolution, or path length
- **Batch actions** — move or trash selected files
- **Preview window** — side-by-side comparison of duplicate groups
- **Persistent cache** — hash database speeds up subsequent scans
- **Export/Import** — save and load duplicate group results as JSON

## Building

### Dependencies

- Rust 1.70+
- GTK4, libadwaita (development packages)

### Build

```bash
cargo build --release
```

### Run

```bash
cargo run --release
```

## Usage

1. Click **Add Directory** to select folders to scan
2. Optionally mark one as **Reference** (files there won't be selected for batch actions)
3. Click **Scan**
4. Review duplicate groups, select files, and Move or Trash

## License

MIT
