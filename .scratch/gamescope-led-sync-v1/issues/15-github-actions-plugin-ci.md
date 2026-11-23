# 15: GitHub Actions validate the Decky plugin (js + python) too

**Reported by the owner (2026-11-23):** commit and push, and also introduce
GitHub Actions to validate changes on GitHub. The repository already had
workflows for Rust, Markdown, Nix, TOML, and YAML (`.github/workflows/`,
each holding only the triggers and calling the reusable workflows of
`larry0x/tools` at the commit pinned in flake.lock). The gap: the **Decky
plugin** (`plugin/src/*.tsx`, `plugin/main.py`) had no CI — ticket 14
changed both and nothing on GitHub validated them.

**Locked decision:** follow the existing pattern exactly —

- `.github/workflows/js.yml` and `.github/workflows/python.yml`, same
  header/triggers shape as the other five, `paths` filters on `plugin/**`
  (resp. `plugin/main.py`) plus the files that change what the recipes do
  (flake, shell.nix, justfile, base.just, .gitignore, the workflow itself),
  jobs calling `larry0x/tools/.github/workflows/js.yml` (resp. `python.yml`)
  at the pinned rev `702cf291e0a264e8c926feeeb3993785f6d925cc` — `nix flake
  check` fails on any other reference.
- The tools repo's js/python `just` recipes no-op without **root** marker
  files (`package.json`, `biome.json`, `pyproject.toml`), so the repo's
  `justfile` overrides them to run the same tools inside `plugin/`
  (`install-js-deps`, `fmt-js`, `fmt-check-js`, `lint-js`, `test-js`,
  `test-python`); `test-js` also runs `npm run build` (dist/ is gitignored,
  so CI validates the build succeeds), `test-python` runs
  `python3 -m py_compile plugin/main.py`.
- `shell.nix` adds `nodejs`, `biome`, `python3` so the nix dev shell provides
  what those recipes call (CI runs them via `nix develop`).

**Status:** done (commit <hash> on main)

- [x] `.github/workflows/js.yml`, `.github/workflows/python.yml` (pinned rev,
  paths filters, same shape as the existing five)
- [x] `justfile`: js/python recipe overrides for `plugin/`
- [x] `shell.nix`: nodejs + biome + python3 in the dev shell
- [x] Docs: SPEC.md §12, README.md development block
- [x] Local simulation of the recipes: biome format/lint clean, `npm run
  check` (tsc + biome), `npm run build`, `py_compile main.py`
- [x] `cargo test` still green; commit + push to origin main
