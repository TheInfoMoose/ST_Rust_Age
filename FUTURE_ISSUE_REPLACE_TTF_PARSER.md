# Future Project / Issue: Replace `ttf-parser` Dependency

## Context & Motivation
During a recent security and dependency audit (`cargo-audit`), the crate `ttf-parser` was flagged as **unmaintained** (Advisory ID: `RUSTSEC-2026-0192`). 

Currently, `ttf-parser` is brought into our dependency tree transitively via the UI framework's windowing backend:
`slint` -> `i-slint-backend-winit` -> `winit` -> `ab_glyph` -> `owned_ttf_parser` -> `ttf-parser`.

While it does not pose an active security vulnerability and is highly stable, relying on unmaintained crates is against our security best practices. However, because it is deeply embedded in major Rust ecosystem UI crates (`winit` and `slint`), replacing it immediately is an excessive resource expenditure that would require forking and rewriting parts of those libraries.

## Objective
The goal of this project is to eventually eliminate our reliance on `ttf-parser` by tracking upstream movements and migrating when feasible, or contributing to an upstream replacement.

## Proposed Action Items (To be addressed at a later date)
1. **Monitor Upstream:** Keep track of the `winit` and `ab_glyph` repositories for any discussions, PRs, or forks aiming to replace `ttf-parser` with a maintained alternative (e.g., `skrifa` or a community-maintained fork of `ttf-parser`).
2. **Slint Backend Alternatives:** Investigate if Slint can be configured to use a different windowing/text-rendering backend that does not rely on `ab_glyph`/`ttf-parser` on certain platforms.
3. **Community Contribution:** If upstream projects are stalled, consider opening an issue/bounty or contributing a PR to `ab_glyph` to transition to a maintained parser.
4. **Remove CI Exception:** Once the dependency is swapped out in a future version of `slint`/`winit`, remove the `--ignore RUSTSEC-2026-0192` flag from our `.github/workflows/ci.yml`.

## Current Mitigation
For now, the crate is ignored in the CI workflow since it strictly handles font parsing and has no known exploits. 
