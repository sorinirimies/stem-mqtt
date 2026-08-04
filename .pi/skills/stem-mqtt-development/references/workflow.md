# Workflow reference — stem-mqtt

## justfile-first

Always use `justfile` recipes instead of raw cargo/git commands — it
already covers nearly every workflow. Run `just --list` if unsure a recipe
exists before writing a manual command.

Key recipes:

- `just build` / `test` / `check-all` / `check-release`
- `just bump <version>` / `just release <version>` — bump + commit + tag +
  push, triggers the Release workflow
- `just release-retrigger <version>` — re-dispatch the Release workflow
  for an existing tag
- `just push` / `push-all` / `pull-all` — GitHub `origin` + Gitea `gitea`
  remotes (see below)
- `just push-tags`
- `just publish` — crates.io
- `just version`
- `just vhs-all` / `vhs-tape <name>` / `vhs-list` — VHS demo GIFs (see
  below)
- `just package-kotlin-jvm` / `package-kotlin-android`
- `just bindings-kotlin` / `bindings-swift` / `bindings-python` /
  `bindings-ruby`

## Releasing safely

Before tagging/publishing a real release:

1. Confirm the target version matches what's already in
   `Cargo.toml`/`package.json` (bump if not).
2. Confirm `CRATES_IO_TOKEN`/`NPM_TOKEN` etc. secrets are actually
   configured (`gh secret list`) — publishing is irreversible (crates.io =
   yank only, npm unpublish restricted after 72h).
3. Get explicit user confirmation before pushing a version tag.
4. After pushing, verify: `gh run list --branch main --limit 5` shows the
   latest CI run green, and check the Release workflow run status.

## Dual git remotes

This repo has **two** remotes: `origin` (GitHub, primary) and `gitea`
(self-hosted mirror). Default to `just push` (GitHub only) unless asked to
sync both (`just push-all`) or Gitea specifically.

## VHS demo GIFs

- Tapes live in `examples/vhs/*.tape`, rendered GIFs in
  `examples/vhs/generated/` — these **are** committed via git-lfs (`*.gif`
  tracked in `.gitattributes`) and embedded in `README.md`'s `## Preview`
  section, not gitignored.
- When a tape backgrounds a process (e.g. the broker) in the same
  terminal:
  - Redirect its stdout/stderr to `/dev/null` — its tracing logs otherwise
    interleave with the foreground command's output in the recording.
  - Run `set +m` first to disable job-control notifications — otherwise
    `[1] <pid>` and `[1]+ Exit/Done ...` lines land in the recording.
  - Use `Hide` / `Type "clear"` / `Enter` / `Show` to scrub any leftover
    typed-command text from the visible recording before the real demo
    starts.
- Verify a re-rendered tape by extracting multiple frames
  (`ffmpeg -i generated.gif frame_%03d.png`) and inspecting them — don't
  just trust the render succeeded silently.

## Broker CI flakiness (a real bug that was fixed here)

Never background a broker with `cargo run ... &` followed by a fixed
`sleep N` before a client connects — first-compile time varies and a short
sleep races it, causing "Connection refused". Pre-build the binary
separately, then poll the port until it's actually listening. See
`.github/workflows/ci.yml`'s `test-node` job for the fixed version.

## justfile template provenance

This repo's `justfile` follows the same task-runner template/conventions as
the author's other Rust workspace projects (same recipe names/structure
for build/test/release/changelog/multi-remote push). If you maintain a
sibling project with the same template, keep the shared recipes
(release/bump/changelog/git-remote ones) in sync and only diverge on crate
names and language-specific packaging recipes.
