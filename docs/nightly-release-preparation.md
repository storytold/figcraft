# Windows, Linux and macOS release-ref nightly: supervised manual go-live

**Enabled for supervised manual runs, not yet qualified.** The producer (`nightly.yml`, main) and the
signer (`nightly-sign.yml`, on the dedicated `nightly-signer` branch) keep only their exact context guards and run when one of the reviewed
manual actors dispatches them. There is no schedule or automatic completion dispatcher yet. The installed
policy [`.github/nightly-signing-policy.json`](../.github/nightly-signing-policy.json) is the v3 policy
for FigCraft. `producer_workflow_id` is the registered `nightly.yml` workflow id, read with:

```bash
gh api repos/storytold/figcraft/actions/workflows/nightly.yml --jq .id
```

The callers pass no platform secrets. The unsigned reusable caller's maximum OIDC/attestation
permissions are for its isolated attester; its compiler narrows to contents and Actions read
only, with no OIDC, attestation write, release environment or platform signing secrets.
Shared tooling is pinned to `be00e5ccd18b68e950c8a08532f2230e7c3d0cba`, the craft-common-actions
main commit that merged [#371](https://github.com/storytold/craft-common-actions/pull/371) (the
candidate is admitted on the built commit's own Sonar analysis, not main's latest) after
[#370](https://github.com/storytold/craft-common-actions/pull/370) (the Windows and macOS nightly
registry rows for the 14 apps) and
[#369](https://github.com/storytold/craft-common-actions/pull/369) (shared helper checkouts kept out
of the app's clean-source check, and macOS on `macos-15`), following
[#364](https://github.com/storytold/craft-common-actions/pull/364) (the signer may run on the
policy's signer branch) and [#363](https://github.com/storytold/craft-common-actions/pull/363) (the
multi-app pipeline, which makes the signer's `app-name` input required and admits any reviewed
registry app), with #342 (`a4f6fd51`, policy v3 and the `platforms` input) and #339 (`605d5876`)
earlier. Both callers name it in `uses` and `tooling-revision`. A pin is reviewed source
availability, not provider, queue, native signing or crypto qualification.
**Signer branch.** The signer runs on `refs/heads/nightly-signer`, not on the stable `release`
branch, so nightly signer files never need a stable release sync. The policy's `release_branch`
names it, the signer's guard requires it and its caller passes `app-name`. The branch only
fast-forwards to reviewed main commits, through the nightly-signer settings script's `advance` (no
queued signer run, a trust-closure scan, then the full diff of `.github/` and `packaging/`), and the
existing `release` environment must admit `nightly-signer` alongside `release` before the first
dispatch (a repository settings change, not a workflow one). Its trust closure is all of `.github/`
and `packaging/`, including `packaging/env.sh`, which `import-cert.sh` sources with the Apple keys.
On that branch only the pinned `nightly-sign.yml` caller may reach an environment: no app workflow
binds one, nothing else calls `nightly-release.yml`, and the only caller of the shared stable
`release.yml` is `release.yml`, whose callee must carry the stable-only environment guard. The
contract tests check the in-repo part on every main PR; the settings scan also resolves the callee.
Stable releases still come only from `refs/heads/release`. History does not carry over a switch
between `release` and `nightly-signer`, so the switch happens before the first publication.

Existing stable workflows are unchanged.

**The shared pipeline is included.** This pin contains craft-common-actions #363, which runs the
shared nightly pipeline for any reviewed `nightly-apps.json` registry app: the signer passes
`app-name` and the shared scripts derive this app's names from that row, so no shared-workflow port
is still outstanding. The app's registry row is the admission boundary for what the producer may build, and
each platform needs its own reviewed target row there.

**First runs.** The first runs are manual dispatches in this order: `nightly.yml` on `main` (only a
listed manual actor passes the producer's plan step), then `nightly-sign.yml` on `nightly-signer` with that
producer run ID and attempt, by a listed manual actor.

**Manual actors.** `bflatastic`, `dylanmtaylor`, `echelon`, `FredApps` and `skylartaylor`, in the
producer's `manual-actors` and the policy's `manual_actors`. The list mirrors the `release-managers`
team (id 19837241), leaving out `echelon-robot`; update it by PR in both places when the team changes.

**Admission timing.** The candidate check reads the newest CI run on the candidate commit and
main's latest finished Sonar analysis, so when to dispatch matters:
- Pick a quiet `main`, wait for its CI and SonarCloud analysis to finish green, then dispatch the
  producer and, once it succeeds, the signer, before another `main` push lands a newer analysis.

**Platform rows.** The producer caller lists
`platforms: '["windows-x86_64","linux-x86_64","macos-aarch64"]'` on its single job: one producer
run builds every listed platform for one all-or-nothing release (one failed or skipped platform stops
that night's release), and the release-ref signer admits platforms only from the protected policy, so
there is no separate Linux or macOS caller job and the signer caller has no platform input.

## Installed policy values and their sources

| Field | Value | Source |
|---|---|---|
| `repository_id` | `1412112308` | `gh api repos/storytold/figcraft --jq .id` |
| `producer_workflow_id` | `429731005` | `gh api repos/storytold/figcraft/actions/workflows/nightly.yml` (path `.github/workflows/nightly.yml`, state `active`) |
| `producer_tooling_revision`, `signer_tooling_revision`, `producer_workflow_ref` | `be00e5cc…` (craft-common-actions exact-analysis admission, #371 after #370 and #369, following #364) | The callers' pin |
| `manual_actors` / `automatic_signer_actors` | the five release managers / `[]` | release-managers team (id 19837241) members; equals the producer caller's `manual-actors` |
| `max_age_seconds`, `max_artifact_bytes` | `86400`, `2147483648` | Go-live checklist |
| `platforms` | three rows | Storytold stable signing identities (`windows-x86_64`, `linux-x86_64`, `macos-aarch64`) |
| `candidate_policy.required_workflows` | CI (`ci.yml`, workflow `379864519`), `gate / gate (ubuntu-24.04)` | Workflow list API and latest `main` push CI run |
| `candidate_policy.sonar` | `storytold` / `storytold_figcraft`, app `12526` | `sonar-project.properties` |
| Sonar `issue_types` / `debt` | `VULNERABILITY` / `[]` | Refresh from SonarCloud before promoting policy to `release` |

The candidate check refuses whenever main's unresolved vulnerability set differs from `debt`, whether
an issue is new or fixed, and whenever a required condition fails or a listed CI job is renamed. Refresh
those values and `producer_workflow_id` in the same reviewed change that `nightly-signer` is then advanced to.

## Ownership and interface

- `.github/workflows/nightly.yml`: default-main registration of the unsigned producer, named
  `FigCraft unsigned nightly`, enabled for manual dispatch by `manual-actors`; no signing secrets
  may be passed or inherited.
- `.github/workflows/nightly-sign.yml`: top-level signer registration, enabled for manual dispatch
  on `nightly-signer`, which carries a reviewed main commit. It rejects wrong repository/event/actual
  branch, including same-name tags. Its constant whole-run concurrency (`figcraft-nightly-release`) is
  distinct from stable and cancellation is off. The caller itself
  passes no credentials; only the shared callee's sign jobs bind the `release` environment.
- Shared reusable entrypoints are `nightly-unsigned.yml` and `nightly-release.yml` in
  `storytold/craft-common-actions`. Both must resolve to a **real reviewed public immutable commit**;
  the defining workflow SHA and tooling input must match.
- `packaging/windows/sign.ps1 -Required` is the app-owned Windows adapter. Invoke it only from the
  immutable **release execution SHA** checkout, never the producer's main source or extracted archive.

## App-side entry points and the nightly profile

The producer's platform legs call these app-owned producers, each of which refuses signing material in
the compilation step:

| Platform | Producer | Signed by |
|---|---|---|
| Windows | `pwsh packaging/windows/package.ps1 -Arch x64 -Nightly -BuildOnly` (through `packaging/windows/nightly.ps1`) | `packaging/windows/sign.ps1 -Required` in the release-ref signer |
| Linux | `bash packaging/linux/nightly.sh --arch x86_64` | the shared signer's detached `.asc` (`.github/nightly-linux-signing-key.asc`) |
| macOS | `bash packaging/macos/nightly.sh --arch aarch64` | `packaging/macos/nightly-sign.sh`, checked by `packaging/macos/nightly-verify.sh` |

Inputs, all required: `FIGCRAFT_BUILD_CHANNEL=nightly-v1`, `FIGCRAFT_VERSION`
(`X.Y.Z-nightly.YYYYMMDD.<run>.<attempt>.g<sha12>`), `FIGCRAFT_BUILD_SHA` (40 hex) and
`FIGCRAFT_BUILD_DATE` (the run's UTC date), with an absolute `CARGO_TARGET_DIR` and `DIST`;
`CRAFT_FONTS_DIR`/`CRAFT_FONTS_REQUIRED=1` are optional. The build script refuses a nightly without
an exact version, so the feed identity and the binary cannot disagree. `packaging/nightly-common.sh`
runs `<> --version` after the build and expects `FigCraft <version> (nightly-v1)`; the Launcher's form
`--launch-profile nightly-v1 --profile-root <dir> --version` is answered before any window opens.

**The profile.** A nightly build refuses to start without `--launch-profile nightly-v1
--profile-root <absolute dir>` and keeps every piece of user state there: preferences and recent
files, recovery copies, installed plugins and the suite's font store. It never reads or writes
`~/.config/FigCraft` (macOS: `~/Library/Application Support/FigCraft`, Windows: `%APPDATA%\FigCraft`),
a portable copy's `config/`, or the shared `craft-fonts` store; the validator refuses a root that
overlaps any stable folder, a link or an alias. The Linux formats reach it through one launcher
(`packaging/linux/nightly-launch.sh`) that every package format installs as its entry point, with the
profile at `$XDG_DATA_HOME/figcraft-nightly/profile`.

## Local regression checks

`python3 packaging/nightly-release-tests.py` verifies the nightly workflow and policy contracts locally:
- Exact context guards on `nightly.yml` and `nightly-sign.yml` (reading every `if:` form and continuation line).
- No registered nightly schedule (`workflow_dispatch` only).
- Signer root concurrency serialization (`figcraft-nightly-release`, `cancel-in-progress: false`).
- No cache actions, cache setup options, `environment:`, or `secrets:` in the app callers.
- Matching `uses` and `tooling-revision` 40-hex commit pin across both callers and the installed policy.
- Exact `manual_actors` match between `nightly.yml` and `.github/nightly-signing-policy.json`, with empty `automatic_signer_actors`.
- Policy v3 schema, platform list/trust anchor shapes, required CI workflow check, and `sonar-project.properties` alignment.
- The `nightly-signer` trust-closure scan: no app workflow binds `environment:`, only
  `nightly-sign.yml` calls `nightly-release.yml` at a 40-hex SHA, and the only caller of the
  shared stable `release.yml` is `release.yml`.
