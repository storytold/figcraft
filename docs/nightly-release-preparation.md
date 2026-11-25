# Windows, Linux and macOS release-ref nightly: supervised manual go-live

**Enabled for supervised manual runs, not yet qualified.** The producer (`nightly.yml`, main) and the
signer (`nightly-sign.yml`, release) keep only their exact context guards and run when one of the reviewed
manual actors dispatches them. There is no schedule or automatic completion dispatcher yet. The installed
policy [`.github/nightly-signing-policy.json`](../.github/nightly-signing-policy.json) is the v3 policy
for FigCraft. Once `nightly.yml` reaches `main`, populate `producer_workflow_id` before promoting the
policy to `release` by reading:

```bash
gh api repos/storytold/figcraft/actions/workflows/nightly.yml --jq .id
```

The callers pass no platform secrets. The unsigned reusable caller's maximum OIDC/attestation
permissions are for its isolated attester; its compiler narrows to contents and Actions read
only, with no OIDC, attestation write, release environment or platform signing secrets.
Shared tooling is pinned to `605d58766e8f536564da8fc81d3676ac86816dea`, the craft-common-actions
main commit that merged [#339](https://github.com/storytold/craft-common-actions/pull/339) (bounded
lineage compares and an issue reread at the end of candidate admission) on top of
[#342](https://github.com/storytold/craft-common-actions/pull/342) (policy v3, the `platforms` input
and the Linux and macOS legs). Both callers name it in `uses` and `tooling-revision`. A pin is reviewed
source availability, not provider, queue, native signing or crypto qualification.
Existing stable workflows are unchanged.

**The shared-workflow port is a prerequisite.** At this pin both reusable workflows admit only
PhotoCraft: `nightly-unsigned.yml` requires `inputs.app-name == 'photocraft'` and
`github.repository == 'storytold/photocraft'` on its build jobs, and `nightly-release.yml` requires the
same repository in `validate` and every signing job (its concurrency group is
`craft-release-ref-signer-photocraft`). A FigCraft dispatch therefore builds and signs nothing until
`storytold/craft-common-actions` admits `storytold/figcraft`. No dispatch can produce a release before
that port, and both callers' `uses`, `tooling-revision` and the policy pins must move to the same
reviewed revision together. The port is craft-common-actions #363 ("run the shared nightly pipeline
for any reviewed registry app"); until it merges these callers are inert.

**First runs.** The first runs are manual dispatches in this order: `nightly.yml` on `main` (only a
listed manual actor passes the producer's plan step), then `nightly-sign.yml` on `release` with that
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
| `producer_workflow_id` | `null` (pending `nightly.yml` on `main`) | `gh api repos/storytold/figcraft/actions/workflows/nightly.yml --jq .id` once merged to `main` |
| `producer_tooling_revision`, `signer_tooling_revision`, `producer_workflow_ref` | `605d5876…` (craft-common-actions #339) | The callers' pin |
| `manual_actors` / `automatic_signer_actors` | the five release managers / `[]` | release-managers team (id 19837241) members; equals the producer caller's `manual-actors` |
| `max_age_seconds`, `max_artifact_bytes` | `86400`, `2147483648` | Go-live checklist |
| `platforms` | three rows | Storytold stable signing identities (`windows-x86_64`, `linux-x86_64`, `macos-aarch64`) |
| `candidate_policy.required_workflows` | CI (`ci.yml`, workflow `379864519`), `gate / gate (ubuntu-24.04)` | Workflow list API and latest `main` push CI run |
| `candidate_policy.sonar` | `storytold` / `storytold_figcraft`, app `12526` | `sonar-project.properties` |
| Sonar `issue_types` / `debt` | `VULNERABILITY` / `[]` | Refresh from SonarCloud before promoting policy to `release` |

The candidate check refuses whenever main's unresolved vulnerability set differs from `debt`, whether
an issue is new or fixed, and whenever a required condition fails or a listed CI job is renamed. Refresh
those values and `producer_workflow_id` in the same reviewed change that promotes the policy to `release`.

## Ownership and interface

- `.github/workflows/nightly.yml`: default-main registration of the unsigned producer, named
  `FigCraft unsigned nightly`, enabled for manual dispatch by `manual-actors`; no signing secrets
  may be passed or inherited.
- `.github/workflows/nightly-sign.yml`: top-level release registration, enabled for manual dispatch,
  and identical on `main` and `release` after reviewed promotion. It rejects wrong repository/event/actual
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
