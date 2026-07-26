# Configure-tui

A terminal UI that configures an [Arkime](https://arkime.com) installation,
replacing the old bash `Configure` script. The shipped binary is `Configure-tui`
(it lives next to `Configure` in `/opt/arkime/bin`).

Run it without installing anything via Docker to generate compose files:

```sh
docker run --rm -it -v "$PWD:/work" ghcr.io/arkime/configure-tui
``` It supports **native** deployments
(writes `config.ini` etc., manages systemd/rc.d services) and **Docker**
deployments (generates `docker-compose.yml` + `ARKIME__*` env, writes no
`.ini`), and lets you toggle any combination of components
(capture/viewer/wise/parliament/cont3xt).

Built as a single **static binary per architecture** with no runtime
dependencies, so it can be produced once and pulled into the rpm/deb packages.

## Wizard flow

1. **Start** — one of four modes (everything branches here):
   - Docker — create a new `docker-compose`
   - Docker — load an existing `docker-compose`
   - Run on machine — create new `.ini` files
   - Run on machine — load existing `.ini` files

   On macOS only the two docker modes appear (native needs Linux/FreeBSD).
2. **Load file** (load modes only) — path to the compose file / etc dir. The
   wizard prefills components and values from it.
3. **Components** — multi-select toggles; later prompts are the union of what the
   selected components need.
4. **Interfaces** (capture only; checkbox list of detected NICs + advanced
   free-type), **OpenSearch/Elasticsearch** (URL/user/password; in docker you can
   opt into a single-node Elasticsearch we configure — you pick the host data
   dir, which becomes a compose volume, and heap is auto-sized so there's no
   memory question), **Encryption password**, **Plugins** (capture only; checkbox list,
   auto-enables `wise.so` when the wise component is on), **Viewer plugins**
   (viewer only; checkbox list incl. `wise.js`), **WISE URL** (only when
   `wise.so` is enabled without deploying the wise component — points capture at
   an external WISE), **GeoIP** (MaxMind account + license → writes `GeoIP.conf`;
   native also runs the download), **Database & admin** (native only; optionally
   `db.pl init` and create an admin user), **Docker mounts** (docker only;
   suggested host bind mounts, toggleable).
5. **Review** → **Apply** → live progress log with next-steps for anything left
   to do by hand.

Navigation: **Enter** / **→** advance, **Esc** / **←** go back (or quit on the
first screen). On typing screens `←/→` move the field cursor instead, so use
`Enter`/`Esc` there.

### In-memory documents & the `^E` editor

Every output file (`docker-compose.yml` + `arkime.env`, or `config.ini` /
`wise.ini` / `cont3xt.ini`) is kept as an in-memory text buffer. Wizard changes
merge the understood fields into it while **preserving everything we don't
understand** (unknown compose services/keys, unknown `ARKIME__*` vars, unknown
ini keys). **Nothing is written to disk until the final apply** (load modes
overwrite the loaded file — after backing it up to a timestamped `.bak` — and
new modes never clobber an existing one).

Press **`^E`** (Ctrl-E) at any time — or `e` on a non-typing screen — to open a
full-file editor with a **tab per file** (`Tab`/`Shift-Tab` cycles). Edit the
fully-substituted text freely; on exit (`Esc`) the understood fields are parsed
back into the wizard. Editor and wizard are two views of the same data,
last-write-wins. **`^D`** shows a **diff** of the current file vs its original —
handy from the Review screen to see exactly what will change before writing.

Every docker flow has a **Service prefix** step. The tool detects the
service-name prefix (`arkime-viewer` vs `viewer` vs `arkime6-viewer`) and
round-trips it. You can **choose** which prefix set to manage, **add** a new one
(`a` — works on new files too, so one compose can hold several deployments), or
**delete** one (`d` — strips those services). Prefixes you aren't managing are
left untouched.

Docker mode suggests these host mounts (all on by default, individually
toggleable), attached to the capture/viewer services:

```
/arkime/etc:/opt/arkime/etc
/arkime/pcap:/opt/arkime/raw
/arkime/maxmind:/var/lib/GeoIP
./GeoIP.conf:/etc/GeoIP.conf
```

## Develop

```sh
cargo test          # unit + view (TestBackend) tests
cargo clippy --all-targets -- -D warnings
cargo run -- --help
```

The tool refuses to run on macOS (same as the bash version); run it in a Linux
VM/container to drive the UI. **Root is not required to start** — only the
**native** apply needs it (it writes system config and manages services). Docker
mode only writes `docker-compose.yml` + `arkime.env` to the current directory
and needs no privileges; the Review screen warns if you pick native without root.

Runtime overrides:

- `--install-dir <PATH>` (default `/opt/arkime`)
- `--name <NAME>` (default `arkime`)

Compile-time defaults can be baked via the `BUILD_ARKIME_INSTALL_DIR` /
`BUILD_ARKIME_NAME` env vars at build time (see `src/domain/build_config.rs`).

## Builds & releases

Fully static musl binaries on Linux (`+crt-static`), a native binary on macOS:

```sh
cargo build --release --target x86_64-unknown-linux-musl
file target/x86_64-unknown-linux-musl/release/Configure-tui   # "statically linked"
```

On a `v*` tag, `.github/workflows/release.yml` builds on native runners
— **linux-amd64** (`ubuntu-24.04`), **linux-arm64** (`ubuntu-24.04-arm`), and
**macos-arm64** (`macos-14`) — and attaches `Configure-tui-<name>` + `.sha256`
to the GitHub release (version comes from the tag). It also builds and pushes
multi-arch container images to `ghcr.io/arkime/configure-tui`. All actions are
pinned by commit SHA.

## Packaging integration (Arkime rpm/deb)

The Arkime build downloads the correct-arch `Configure-tui-<name>` (pinned by tag
+ checksum), drops it into `bin/`, and `make install` copies it to
`/opt/arkime/bin/Configure-tui`, next to the existing `Configure`. The
`.ini.sample` files are read from `/opt/arkime/etc` at runtime (the binary also
embeds fallback copies under `templates/`).

## Layout

- `src/domain/` — deployment, components, platform, answers, build config
- `src/steps.rs` — wizard state machine (`required_steps`/`next`/`prev`)
- `src/config/` — native `.ini` templating (`substitute`, `templates`)
- `src/docset.rs` — in-memory documents + round-trip parse/merge for ini, env,
  and compose (opensearch/elasticsearch backends, prefixes, GeoIP.conf)
- `src/actions/` — `native` (systemd/rc.d, dirs, limits.d, backend install,
  GeoIP, DB init/user), `system` (the `SystemOps` side-effect boundary)
- `src/app.rs`, `src/ui.rs` — Elm-style model/update + rendering
- `templates/` — fallback `.ini.sample` copies (kept in sync with Arkime's)
