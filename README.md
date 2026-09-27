# disk-wiz (`dw`)

A disk usage tool with two personalities in one binary:

- **Interactive TUI** (default on a terminal): a squarified treemap with live
  scan progress, keyboard-first navigation, zoom, depth control, filtering, a
  details sidebar, and "worth a look" heuristics.
- **Pipe-friendly CLI** (default when piped): `flat` (du-like), `tree`, `json`,
  and `ansi` treemap output, with ls-like flags for sorting, filtering, depth,
  and limits.

## Install

```sh
cargo install --path .
```

This builds the `dw` binary.

## TUI

```sh
dw                    # scan the current directory
dw ~/src --depth 3    # start at depth 3
dw --palette magma --color-mode size
```

Keys (all rebindable, see Configuration):

| Key | Action |
|---|---|
| `h j k l` / arrows | move the selection |
| `enter` / `o` | open (zoom into the selected directory) |
| `backspace` / `u` | up one level |
| `[` `]` | decrease / increase depth |
| `t` | cycle size / files / age |
| `H` | toggle hidden entries (rescans) |
| `a` | toggle apparent size (rescans) |
| `/` | filter by name (esc clears) |
| `space` | mark / unmark the selection |
| `d` | move marked entries to the trash (with confirmation) |
| `c` | clear marks |
| `r` | rescan |
| `0` | reset the view |
| `?` | help |
| `q` / `ctrl-c` | quit |

Mouse: click selects, double click opens, right click goes up, the wheel moves
the selection.

The sidebar shows the selection (size, share of the view, files, dirs, last
write, kind, category), the "worth a look" list (large or stale build output,
caches, `node_modules`, ...), and free/used/total disk space.

## CLI

```sh
dw -p --format flat -n 20      # top 20 entries by size
dw --format tree -d 2          # indented tree
dw --json | jq '.children'     # machine-readable
dw --format ansi --width 120   # treemap rendered to stdout
dw --min-size 1G --type cache  # big cache entries
dw --exclude '**/node_modules/**'
dw --threshold 50G /var        # exit code 4 when over budget (CI-friendly)
```

| Flag | Meaning |
|---|---|
| `-p, --print` | force non-interactive output |
| `-f, --format <auto\|tui\|flat\|tree\|json\|ansi>` | output format |
| `-d, --depth <N>` | maximum depth (tree defaults to 2) |
| `-n, --top <N>` | row limit (flat) or per-directory limit (tree) |
| `--sort <size\|files\|mtime\|name>`, `--reverse` | ordering |
| `--min-size`, `--max-size <SIZE>` | size filters (`10M`, `1.5GiB`) |
| `--type <CAT,...>` | category filter (`--list-categories`) |
| `--exclude <GLOB>` (repeatable), `--exclude-file <PATH>` | skip entries |
| `--hidden` / `--no-hidden` | hidden entries (default: included) |
| `--apparent-size` | logical bytes instead of allocated blocks |
| `--follow`, `--one-file-system`, `--count-links` | scan behavior |
| `--threads <N>` | scanner threads |
| `--color <auto\|always\|never>` | color output (`NO_COLOR` honored) |
| `--palette <NAME>`, `--color-mode <category\|size\|age\|depth>` | colors |
| `--width`, `--height` | ansi output size |
| `--json`, `--bytes`, `--quiet`, `--verbose` | output details |
| `--strict`, `--threshold <SIZE>` | exit codes 3 and 4 |
| `--list-palettes`, `--list-categories` | introspection |
| `--completions <SHELL>` | shell completions |

Exit codes: `0` ok, `1` error, `2` usage, `3` scan errors with `--strict`,
`4` total exceeded `--threshold`.

## Colors

- `category` (default): file-type categories (code, media, documents, cache,
  git, toolchains, agent scratch, synced) with muted colors tuned for terminals.
- `size`: a sequential ramp over size, with `scale = linear | log | rank` (log
  by default) and optional discrete `tiers`.
- `age`: a ramp over the newest mtime in each subtree.
- `depth`: a categorical cycle by nesting depth.

Palettes: `viridis` (default), `magma`, `inferno`, `plasma`, `cividis`,
`turbo`, `spectral`, plus categorical `okabe-ito`, `tableau10`, and `dim`.
`dark` and `light` theme variants, plus a `reverse` flag.

## Configuration

`~/.config/disk-wiz/config.toml` (Linux/macOS) or
`%APPDATA%\disk-wiz\config.toml` (Windows). Precedence: defaults < config file
< environment < CLI flags. See `docs/config.example.toml` for the full schema.

```toml
[scan]
hidden = true
exclude = ["**/.git/objects/**"]

[tui]
depth = 4
size_mode = "size"
sidebar = "auto"
mouse = true

[color]
mode = "category"
palette = "viridis"
scale = "log"
theme = "dark"

[color.categories]
code = { color = "#4C8BF5", globs = ["*.rs", "*.py"] }

[worth_a_look]
max_items = 7
rules = [
  { name = "build output", glob = "**/{target,dist,build}", older_than = "7d" },
]

[keys]
quit = ["q", "C-c"]
zoom_in = ["enter", "o"]
```

## Semantics

- Sizes match `du` conventions: allocated blocks by default on Unix, logical
  bytes on Windows (which has no allocated-block API in `std::fs`).
- Directory metadata is included in totals, like `du`.
- Hardlinks are counted once on Unix (`--count-links` overrides); Windows
  hardlink dedupe is not implemented in v1.
- Symlinks are counted as entries, never followed unless `--follow` is given
  (cycle-safe).
- Junctions and reparse points are never followed.
- Permission errors are counted, sampled, and never abort a scan.

## Development

```sh
cargo test                 # unit + integration tests (TUI via TestBackend)
cargo clippy --all-targets # lint
cargo bench                # scanner and layout benchmarks
cargo test dump_frame -- --ignored --nocapture   # render the TUI to stdout
```

Layout invariants (tiling, disjointness, determinism) are property-tested;
scanner totals are cross-checked against fixture sizes; the CLI is tested
end-to-end with `assert_cmd`.

## License

Dual licensed under MIT or Apache-2.0, at your option.
