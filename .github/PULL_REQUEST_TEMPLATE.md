<!-- Thanks for contributing. Keep the checklist; it mirrors CI gates. -->

## What & why

<!-- One paragraph: what changes and the user-visible/operator-visible reason. -->

## Data-safety impact

<!-- Does this touch state writes, index migrations, permissions, or the
     kernel↔plugin protocol? If yes, describe the failure mode you handled. -->

## Checklist

- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is clean
- [ ] `npm run typecheck` / `npm run lint` / `npm test` pass
- [ ] `npm run build` succeeds (plugins + shell bundle)
- [ ] Docs updated (`docs/`, ADR if an architecture decision was made)
- [ ] `CHANGELOG.md` entry added under Unreleased (if user-visible)
- [ ] No credentials, tokens, or machine-specific paths committed
