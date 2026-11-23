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
ci: fmt-check lint test

# Run every check that works on macOS: all but the Rust lint and tests
[group('all')]
[macos]
ci: fmt-check lint-nix lint-toml lint-yaml lint-markdown test-nix

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

# ----------------------------------- plugin -----------------------------------
#
# The Decky plugin (plugin/) is this repository's only JavaScript and Python:
# its package.json, biome.json, and main.py live there, not at the root, so
# the base recipes' root markers would no-op. These overrides replace them and
# run the same tools in plugin/; the js and python CI workflows call the same
# recipes through `nix develop`, so CI checks exactly what `just ci` checks.
# The npm steps are skipped when plugin/node_modules is absent (a clean
# checkout): CI and the Linux `just ci` run install-js-deps first.

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
  biome format --write .

# Report plugin formatting drift, changing nothing
[group('js')]
[script]
fmt-check-js:
  [ -f plugin/biome.json ] || exit 0
  {{ enter }}
  cd plugin
  biome format .

# Lint the plugin: Biome, then its package's `check` script (tsc + biome lint)
[group('js')]
[script]
lint-js:
  [ -f plugin/biome.json ] || [ -f plugin/package.json ] || exit 0
  {{ enter }}
  cd plugin
  if [ -f biome.json ]; then biome lint --error-on-warnings .; fi
  if [ -d node_modules ]; then npm run --if-present check; fi

# Build the plugin (dist/ is gitignored, built at install time) and run its
# tests, if any
[group('js')]
[script]
test-js:
  [ -f plugin/package.json ] || exit 0
  {{ enter }}
  cd plugin
  if [ -d node_modules ]; then npm run --if-present test; npm run build; fi

# Syntax-check the plugin's Python backend
[group('python')]
[script]
test-python:
  [ -f plugin/main.py ] || exit 0
  {{ enter }}
  python3 -m py_compile plugin/main.py
