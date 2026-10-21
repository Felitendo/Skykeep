<!-- Thanks for contributing to Skykeep. A few lines here make review quick. -->

## What this changes

<!-- One or two sentences. Link the issue it closes: "Closes #123". -->

## Why

<!-- The problem it solves, or the behaviour it brings closer to Apple's own app. -->

## How it was tested

<!-- What you ran and what you saw. For anything that touches iCloud, say whether it was checked
     against a real (test) account and an iPhone, and how. -->

- [ ] `./check.sh` passes (or: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `npx tsc --noEmit`, and the relevant `.uiverify` suites)
- [ ] New or changed behaviour has a test that fails without this change
- [ ] Interface changes: checked in light and dark, and in WebKit (`ENGINE=webkit`)

## Screenshots

<!-- For interface changes: before and after, light and dark. Use test data only. -->

## Checklist

- [ ] No personal data (real names, emails, notes, account IDs) in code, tests, fixtures or screenshots
- [ ] No new network host: everything still goes through `apple_only()` (or `listed_only()` for subscriptions)
- [ ] User-facing text is short, positive and true, and never implies Apple affiliation
- [ ] Commit messages follow the style in [CONTRIBUTING.md](CONTRIBUTING.md#commit-style)
