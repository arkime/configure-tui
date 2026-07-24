# arkime-setup

A terminal UI that configures an [Arkime](https://arkime.com) installation,
replacing the old bash `Configure` script. It supports **native** deployments
(writes `config.ini` etc., manages systemd/rc.d services) and **Docker**
deployments (generates `docker-compose.yml` + `ARKIME__*` env, writes no
`.ini`), and lets you toggle any combination of components
(capture/viewer/wise/parliament/cont3xt).

Built as a single **static binary per architecture** with no runtime
dependencies, so it can be produced once and pulled into the rpm/deb packages.

## Wizard flow

1. **Deployment** — Native vs Docker (everything branches on this).
2. **Components** — multi-select toggles; later prompts are the union of what the
   selected components need.
3. **Interfaces** (capture only), **OpenSearch/Elasticsearch** (URL/user/password
   + optional local demo), **Encryption password**, **GeoIP** (native + capture).
4. **Review** → **Apply** → live progress log.

## Develop

```sh
cargo test          # unit + view (TestBackend) tests
cargo clippy --all-targets -- -D warnings
cargo run -- --help
```

The tool refuses to run on macOS (same as the bash version) and requires root;
pass `--no-root-check` for a local look on a dev box (note: the macOS refusal
still applies — run it in a Linux VM/container to drive the UI).

Runtime overrides:

- `--install-dir <PATH>` (default `/opt/arkime`)
- `--name <NAME>` (default `arkime`)

Compile-time defaults can be baked via the `BUILD_ARKIME_INSTALL_DIR` /
`BUILD_ARKIME_NAME` env vars at build time (see `src/domain/build_config.rs`).

## Static / cross builds

Fully static musl binaries, built with [`cross`](https://github.com/cross-rs/cross):

```sh
cross build --release --target x86_64-unknown-linux-musl
cross build --release --target aarch64-unknown-linux-musl
file target/x86_64-unknown-linux-musl/release/arkime-setup   # "statically linked"
```

CI (`.github/workflows/release.yml`) builds both arches on tag push and attaches
`arkime-setup-<arch>` + `.sha256` to the GitHub release.

## Packaging integration (Arkime rpm/deb)

The Arkime build downloads the correct-arch `arkime-setup-<arch>` (pinned by tag
+ checksum), drops it into `bin/`, and `make install` copies it to
`/opt/arkime/bin/`. It replaces the old `bin/Configure`; install it under that
name for a drop-in swap. The `.ini.sample` files are read from
`/opt/arkime/etc` at runtime (the binary also embeds fallback copies under
`templates/`).

## Layout

- `src/domain/` — deployment, components, platform, answers, build config
- `src/steps.rs` — wizard state machine (`required_steps`/`next`/`prev`)
- `src/config/` — native `.ini` templating (`substitute`, `templates`)
- `src/actions/` — `native` (systemd/rc.d, dirs, limits.d, demo-ES, GeoIP),
  `docker` (compose + env), `system` (the `SystemOps` side-effect boundary)
- `src/app.rs`, `src/ui.rs` — Elm-style model/update + rendering
- `templates/` — fallback `.ini.sample` copies (kept in sync with Arkime's)
