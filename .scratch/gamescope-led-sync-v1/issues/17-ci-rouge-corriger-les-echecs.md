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

**Status:** work in progress

- [ ] justfile: plugin recipes use `./node_modules/.bin/biome` (+ npm ci
      fallback); shell.nix drops pkgs.biome
- [ ] `cargo fmt` whole crate; `cargo fmt --check` clean; tests green
- [ ] `.gitignore` restored to the pinned tools content
- [ ] SPEC.md / docs/agents/domain.md lint fixes
- [ ] verify locally what can be (biome via node_modules, fmt, tests);
      commit + push; watch the new Actions runs go green
