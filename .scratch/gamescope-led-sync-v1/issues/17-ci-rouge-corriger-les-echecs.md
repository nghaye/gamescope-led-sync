# 17: CI rouge sur les push de main — corriger les 4 échecs

**Reported by the owner (2026-11-23):** les GitHub Actions fail sur les
derniers push de main (runs sur `7ac12c3` et `6678ca5`). Quatre causes
racines, toutes diagnostiquées depuis les logs :

1. **JavaScript** (`js / Format`, `js / Lint + Test`): the justfile overrides
   of ticket 15 call a bare `biome`, which in CI resolves to the dev shell's
   nixpkgs Biome **2.x**, and it rejects the plugin's `biome.json` (schema
   1.9.4: `files.include` unknown, etc.). Fix: the plugin uses the Biome that
   `package-lock.json` pins (1.9.x) via `./node_modules/.bin/biome`, with
   `npm ci` as fallback when `node_modules` is missing (the js workflow's fmt
   job runs no install step); `pkgs.biome` is dropped from `shell.nix`.
2. **Rust** (`rust / Format` only; Lint+Test is green): the committed
   `src/reload.rs` / `src/setup.rs` are not rustfmt-clean under the pinned
   toolchain (pre-existing drift). Fix: `cargo fmt` the whole crate.
3. **Nix** (`nix / Lint + Test` = `nix flake check`): our `.gitignore` has a
   local `.scratch/*` + `!.scratch/...` exception, so it differs from the
   tools repository's `.gitignore` at the pinned rev (`.scratch/`). Fix:
   restore `.gitignore` to the exact pinned content; the ticket files already
   tracked stay tracked, new ones need `git add -f`.
4. **Markdown** (`markdown / Lint`): 8 pre-existing issues — MD049
   (`*active*`, `*runtime*` italics must be underscores) at SPEC.md:28 and
   :128, MD040 (fence without language) at SPEC.md:49, :347 and
   docs/agents/domain.md:17, :28. Fix: `_active_`, `_runtime_`, ```text
   fences.

**Status:** done (commit a494553 on main)

- [x] justfile: plugin recipes use `./node_modules/.bin/biome` (1.9.x pinned
      by package-lock.json) + `npm ci` fallback when node_modules is missing;
      shell.nix drops pkgs.biome (nixpkgs Biome is 2.x and rejects the 1.9
      biome.json)
- [x] `cargo fmt` whole crate (reload.rs/setup.rs drift); `cargo fmt --check`
      clean; 128 tests pass; clippy 0 warnings
- [x] `.gitignore` byte-identical to the tools repo at the pinned rev
      (`.scratch/`); tracked tickets stay tracked, new ones need `git add -f`
- [x] SPEC.md `_active_`/`_runtime_` (MD049) and ```text fences (MD040);
      docs/agents/domain.md fences too
- [x] committed (a494553) and pushed; new Actions runs watched
