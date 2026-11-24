# The routine recipes come from the tools repository, in base.just: format,
# lint, test, and bump, for every language. This file adds the recipes of
# this repository alone.
import 'base.just'

# The crate links libpipewire, which exists only on Linux, so on macOS the
# Rust lint and tests fail in the build script of libspa-sys. A `ci` that
# failed there would block every commit on the Mac used for editing, so the
# macOS variant runs only the checks that work there: every format check, the
# Nix, TOML, YAML, and Markdown lints, and the Nix test. The Linux variant,
# and the CI workflows on Ubuntu, run the full set. Both replace the `ci` of
# base.just.
#
# Run every check: formatting, linters, and tests
[group('all')]
[linux]
ci: check-versions fmt-check lint test

# Run every check that works on macOS: all but the Rust lint and tests
[group('all')]
[macos]
ci: check-versions fmt-check lint-nix lint-toml lint-yaml lint-markdown test-nix

# ---------------------------------- versioning ----------------------------------
#
# One version number for the whole deliverable (SPEC §11, D14): Cargo.toml is
# the source of truth, and plugin/plugin.json + plugin/package.json carry the
# same number, so the Decky plugin can compare its own version with the
# installed binary's `--version` output and detect a mismatch. `bump-version`
# writes all three at once; `check-versions` fails when they drift apart, and
# both `ci` variants run it.

# Fail when Cargo.toml, plugin/plugin.json and plugin/package.json disagree
[group('all')]
[script]
check-versions:
  [ -f Cargo.toml ] && [ -f plugin/plugin.json ] || exit 0
  v_cargo="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
  v_plugin="$(python3 -c 'import json; print(json.load(open("plugin/plugin.json"))["version"])')"
  v_pkg="$(python3 -c 'import json; print(json.load(open("plugin/package.json"))["version"])')"
  if [ "$v_cargo" != "$v_plugin" ] || [ "$v_cargo" != "$v_pkg" ]; then
    echo "version mismatch: Cargo.toml=$v_cargo plugin/plugin.json=$v_plugin plugin/package.json=$v_pkg" >&2
    echo "fix the files by hand, or set all three at once: just bump-version $v_cargo" >&2
    exit 1
  fi
  echo "version $v_cargo (Cargo.toml = plugin/plugin.json = plugin/package.json)"

# Set the shared version X.Y.Z in Cargo.toml, plugin/plugin.json and
# plugin/package.json, refreshing Cargo.lock and package-lock.json
[group('all')]
[script]
bump-version VERSION:
  [ -f Cargo.toml ] && [ -f plugin/plugin.json ] || exit 0
  {{ enter }}
  if ! [[ "{{ VERSION }}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "VERSION must look like X.Y.Z, got '{{ VERSION }}'" >&2
    exit 2
  fi
  sed -i 's/^version = ".*"/version = "{{ VERSION }}"/' Cargo.toml
  sed -i 's/"version": ".*"/"version": "{{ VERSION }}"/' plugin/plugin.json
  (cd plugin && npm version --no-git-tag-version "{{ VERSION }}")
  cargo metadata --format-version 1 > /dev/null   # refresh Cargo.lock's package version
  just check-versions

# Build the release binary, into target/release (not into $PATH)
[group('rust')]
build:
  {{ dev }} cargo build --release --locked

# `run` hands its extra arguments to the binary. just joins them with spaces
# and never quotes them, so no such argument may contain one.
#
# Run the binary, forwarding extra arguments to it
[group('rust')]
run *ARGS:
  {{ dev }} cargo run -- {{ ARGS }}

# Install the pre-commit hook that base.just's `ci` comment assumes: a fresh
# clone has no hook, so without this nothing runs `just ci` before a commit.
[group('all')]
install-hooks:
  mkdir -p .git/hooks && printf '#!/bin/sh\nexec just ci\n' > .git/hooks/pre-commit && chmod +x .git/hooks/pre-commit && echo "installed .git/hooks/pre-commit (runs just ci on every commit)"

# -------------------------------- distribution ----------------------------------
#
# The distributable artifact is the Decky plugin zip with the embedded payload
# (binary + systemd unit + example config + license): importing it into Decky
# Loader gives the plugin everything it needs to install the service and the
# binary on the Steam Deck, from the Steam UI, no sudo (SPEC §11).

# Build the release binary + the plugin frontend, pack dist/…-decky-<version>.zip (+ .sha256)
[group('all')]
[script]
package-plugin:
  [ -f scripts/package-plugin.sh ] || exit 0
  {{ enter }}
  bash scripts/package-plugin.sh

# ----------------------------------- plugin -----------------------------------
#
# The Decky plugin (plugin/) is this repository's only JavaScript and Python:
# its package.json, biome.json, and main.py live there, not at the root, so
# the base recipes' root markers would no-op. These overrides replace them and
# run the same tools in plugin/; the js and python CI workflows call the same
# recipes through `nix develop`, so CI checks exactly what `just ci` checks.
#
# Biome comes from the plugin's own node_modules (`./node_modules/.bin/biome`,
# the 1.9.x that package-lock.json pins), never from the nix dev shell: the
# nixpkgs Biome is 2.x and rejects the 1.9 config schema. The js workflow's fmt
# job runs no install step, so each recipe runs `npm ci` first when
# node_modules is missing.

# Install the plugin's JavaScript dependencies per package-lock.json
[group('js')]
[script]
install-js-deps:
  [ -f plugin/package.json ] || exit 0
  {{ enter }}
  cd plugin
  npm ci

# Format the plugin's JavaScript/TypeScript sources
[group('js')]
[script]
fmt-js:
  [ -f plugin/biome.json ] || exit 0
  {{ enter }}
  cd plugin
  [ -x node_modules/.bin/biome ] || npm ci
  ./node_modules/.bin/biome format --write .

# Report plugin formatting drift, changing nothing
[group('js')]
[script]
fmt-check-js:
  [ -f plugin/biome.json ] || exit 0
  {{ enter }}
  cd plugin
  [ -x node_modules/.bin/biome ] || npm ci
  ./node_modules/.bin/biome format .

# Lint the plugin: Biome, then its package's `check` script (tsc + biome lint)
[group('js')]
[script]
lint-js:
  [ -f plugin/biome.json ] || [ -f plugin/package.json ] || exit 0
  {{ enter }}
  cd plugin
  [ -d node_modules ] || npm ci
  ./node_modules/.bin/biome lint --error-on-warnings .
  npm run --if-present check

# Build the plugin (dist/ is gitignored, built at install time) and run its
# tests, if any
[group('js')]
[script]
test-js:
  [ -f plugin/package.json ] || exit 0
  {{ enter }}
  cd plugin
  [ -d node_modules ] || npm ci
  npm run --if-present test
  npm run build

# Syntax-check the plugin's Python backend
[group('python')]
[script]
test-python:
  [ -f plugin/main.py ] || exit 0
  {{ enter }}
  python3 -m py_compile plugin/main.py
