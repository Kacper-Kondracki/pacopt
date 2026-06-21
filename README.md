# pacopt

pacopt is a small terminal tool for reviewing missing optional dependencies on pacman-based systems.

It shows installed packages, their optional dependencies, and a list of optional dependencies that are not installed.

## Screenshots

Package view:

![Package view](screenshots/package-view.webp)

Missing optional dependencies view:

![Missing optional dependencies view](screenshots/missing-optional-deps.webp)

## Requirements

- Arch Linux or another pacman-based system
- Rust toolchain

## Run

```sh
cargo run
```

When you quit, selected missing optional dependency names are printed to stdout.

## Controls

- `Tab`: switch view
- `Up` / `Down` or `k` / `j`: move selection
- `Space`: toggle a missing optional dependency
- `Shift+Up` / `Shift+Down`: select a range
- Click a checkbox: toggle it
- Drag from a checkbox: toggle multiple entries
- `/`: search
- `Esc` or `q`: quit
