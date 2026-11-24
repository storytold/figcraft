#!/usr/bin/env python3
"""Nightly workflow and policy contract regression tests (local static checks, not live API/crypto evidence)."""
import copy
import json
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
INSTALLED_POLICY = ROOT / ".github/nightly-signing-policy.json"

NIGHTLY_PLATFORMS = ("windows-x86_64", "linux-x86_64", "macos-aarch64")
# Each platform's one trust field in policy v3, and the public stable identity it must name.
NIGHTLY_TRUST = {
    "windows-x86_64": ("publisher_subject",
                       "CN=Learning Machines Inc, O=Learning Machines Inc, L=Atlanta, S=Georgia, C=US"),
    "linux-x86_64": ("fingerprint", "2B76741B414AAD6CD33E631213A005C0B9D7C3ED"),
    "macos-aarch64": ("apple_team_id", "DJ6XS33FX8"),
}

# Mirrors the storytold release-managers team (id 19837241) minus echelon-robot.
# Update by PR, in the caller and the policy, when the team changes.
RELEASE_MANAGERS = ["bflatastic", "dylanmtaylor", "echelon", "FredApps", "skylartaylor"]


def assert_nightly_serialization(source):
    """Fail closed unless this app's plain root concurrency block retains its contract."""
    lines = source.splitlines()
    roots = [index for index, line in enumerate(lines) if line.startswith("concurrency:")]
    if len(roots) != 1 or lines[roots[0]] != "concurrency:":
        raise ValueError("one plain root concurrency block is required")
    values = {}
    for line in lines[roots[0] + 1:]:
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if not line[0].isspace():
            break
        row = re.fullmatch(r"  (group|cancel-in-progress): ([^#]+?)(?: +#.*)?", line)
        if row is None or row[1] in values:
            raise ValueError("malformed or duplicate nightly concurrency entry")
        values[row[1]] = row[2].rstrip()
    if values != {"group": "figcraft-nightly-release", "cancel-in-progress": "false"}:
        raise ValueError("nightly serialization must use its stable group and no cancellation")


def job_conditions(jobs):
    """Each job-level `if:` as (block indicator or None, the whole expression YAML would read)."""
    lines = jobs.splitlines()
    found = []
    for index, line in enumerate(lines):
        head = re.fullmatch(r"    if:(?: (.*))?", line)
        if head is None:
            continue
        value = (head[1] or "").strip()
        continuation = []
        for follow in lines[index + 1:]:
            if follow.strip() and len(follow) - len(follow.lstrip(" ")) <= 4:
                break
            continuation.append(follow.strip())
        indicator = value if re.fullmatch(r"[>|][+-]?", value) else None
        parts = continuation if indicator else [value] + continuation
        found.append((indicator, " ".join(part for part in parts if part)))
    return found


def producer_platforms(source):
    """Read the one producer job's literal platform list."""
    rows = re.findall(r"(?m)^      platforms: (.*)$", source)
    if len(rows) != 1:
        raise ValueError("exactly one producer platform list is required")
    literal = re.fullmatch(r"'(\[[^']*\])'", rows[0])
    if literal is None:
        raise ValueError("the platform list must be one single-quoted literal")
    platforms = json.loads(literal[1])
    if not isinstance(platforms, list) or json.dumps(platforms, separators=(",", ":")) != literal[1]:
        raise ValueError("the platform list must be compact JSON")
    if len(set(platforms)) != len(platforms) or not set(platforms) <= set(NIGHTLY_PLATFORMS):
        raise ValueError("platforms must be distinct known nightly targets")
    if "windows-x86_64" not in platforms:
        raise ValueError("the Windows row stays in the producer")
    return platforms


def reviewed_manual_actors(caller_source, policy):
    """The one reviewed manual-actor list, identical in the producer caller and the policy."""
    rows = re.findall(r"(?m)^      manual-actors: '(\[[^']*\])'$", caller_source)
    if len(rows) != 1:
        raise ValueError("exactly one literal manual-actors list is required")
    caller = json.loads(rows[0])
    if caller != policy["manual_actors"]:
        raise ValueError("caller and policy must grant the same actors in the same order")
    if caller != RELEASE_MANAGERS:
        raise ValueError("manual actors must be exactly the reviewed release managers")
    if caller != sorted(caller, key=str.lower) or len({login.lower() for login in caller}) != len(caller):
        raise ValueError("manual actors must be distinct and sorted case-insensitively")
    if set(caller) & {login for login in policy["automatic_signer_actors"]}:
        raise ValueError("manual and automatic actors overlap")
    return caller


def caller_pin(name):
    return re.findall(r"(?m)^      tooling-revision: ([0-9a-f]{40})$", (ROOT / ".github/workflows" / name).read_text())[0]


class TrustedSourcePreparation(unittest.TestCase):
    def test_one_producer_and_one_signer_job_carry_every_platform(self):
        for name, job in (("nightly.yml", "unsigned"), ("nightly-sign.yml", "release")):
            jobs = (ROOT / ".github/workflows" / name).read_text().split("\njobs:\n", 1)[1]
            with self.subTest(workflow=name):
                self.assertEqual(re.findall(r"(?m)^  ([^\s#][^:]*):", jobs), [job],
                                 "a platform is a list entry, not a separately activatable job")
        signer = (ROOT / ".github/workflows/nightly-sign.yml").read_text()
        self.assertNotRegex(signer, r"(?m)^\s*platforms?:", "the release policy, not the caller, admits platforms")

    def test_malformed_or_narrowed_platform_lists_are_rejected(self):
        source = (ROOT / ".github/workflows/nightly.yml").read_text()
        current = "platforms: '[\"windows-x86_64\",\"linux-x86_64\",\"macos-aarch64\"]'"
        self.assertIn(current, source)
        for new in ("platforms: '[\"linux-x86_64\"]'",
                    "platforms: '[\"linux-x86_64\",\"macos-aarch64\"]'",
                    "platforms: '[\"windows-x86_64\",\"linux-x86_64\",\"linux-x86_64\"]'",
                    "platforms: '[\"windows-x86_64\",\"macos-x86_64\"]'",
                    "platforms: '[\"windows-x86_64\",\"linux-arm64\"]'",
                    "platforms: '[\"windows-x86_64\", \"linux-x86_64\"]'",
                    "platforms: [\"windows-x86_64\",\"linux-x86_64\"]",
                    "platforms: ${{ vars.NIGHTLY_PLATFORMS }}",
                    "platforms: '{\"windows-x86_64\":1}'",
                    current + "\n      " + current):
            with self.subTest(change=new), self.assertRaises(ValueError):
                producer_platforms(source.replace(current, new))
        with self.assertRaises(ValueError):
            producer_platforms(source.replace("      " + current + "\n", ""))

    def test_continuation_lines_of_any_scalar_form_reach_the_guard_checks(self):
        for name, workflow in (("nightly.yml", "nightly.yml"), ("nightly-sign.yml", "nightly-sign.yml")):
            source = (ROOT / ".github/workflows" / workflow).read_text()
            expected = NightlyEnabledCallers.GUARDS[name]
            guard = re.search(r"(?m)^    if: >-\n(?:      [^\n]*\n)+", source)[0]
            for replacement in (guard + "     || true\n", "    if: " + expected + "\n      || true\n"):
                jobs = source.replace(guard, replacement).split("\njobs:\n", 1)[1]
                with self.subTest(workflow=name, replacement=replacement[-30:]):
                    (indicator, expression), = job_conditions(jobs)
                    self.assertFalse(indicator == ">-" and " ".join(expression.split()) == expected)

    def test_no_nightly_schedule_is_registered(self):
        for name, event in (("nightly.yml", "workflow_dispatch"),
                            ("nightly-sign.yml", "workflow_dispatch")):
            source = (ROOT / ".github/workflows" / name).read_text()
            events = source.split("\non:\n", 1)[1].split("\npermissions:", 1)[0]
            triggers = [line for line in events.splitlines() if line.startswith("  ")
                        and not line.startswith("   ") and line.strip() and not line.lstrip().startswith("#")]
            with self.subTest(workflow=name):
                self.assertEqual(triggers, [f"  {event}:"], "schedule/activation needs explicit review")

    def test_signer_serialization_contract_is_enforced(self):
        assert_nightly_serialization((ROOT / ".github/workflows/nightly-sign.yml").read_text())

    def test_weakened_or_malformed_serialization_is_rejected(self):
        source = (ROOT / ".github/workflows/nightly-sign.yml").read_text()
        for old, new in (
            ("cancel-in-progress: false", "cancel-in-progress: true"),
            ("cancel-in-progress: false", "cancel-in-progress: ${{ github.event_name == 'push' }}"),
            ("cancel-in-progress: false\n", ""),
            ("cancel-in-progress: false", "cancel-in-progress: false\n  cancel-in-progress: false"),
            ("group: figcraft-nightly-release", "group: ${{ github.run_id }}"),
            ("group: figcraft-nightly-release", "group: release-${{ github.ref }}"),
            ("concurrency:\n", "concurrency: ${{ github.run_id }}\n"),
            ("concurrency:\n", "  concurrency:\n"),
        ):
            with self.subTest(change=new), self.assertRaises(ValueError):
                assert_nightly_serialization(source.replace(old, new))

    def test_no_cache_actions_or_cache_setup_options_in_app_trusted_closure(self):
        for name in ("nightly.yml", "nightly-sign.yml"):
            source = (ROOT / ".github/workflows" / name).read_text()
            executable = "\n".join(line for line in source.splitlines()
                                   if not line.lstrip().startswith("#"))
            with self.subTest(workflow=name):
                self.assertNotRegex(executable, r"(?im)^\s*(?:-\s*)?uses:\s*[^\n]*cache")
                self.assertNotRegex(executable, r"(?im)^\s*[^\s:#]*cache[^\s:#]*:")
                self.assertNotRegex(executable, r"(?im)^\s*(?:environment|secrets):")
                self.assertNotIn("secrets.", executable)


class NightlyTrustAnchors(unittest.TestCase):
    """The public stable identities each platform's signed nightly must carry (all three platforms)."""

    def setUp(self):
        self.policy = json.loads(INSTALLED_POLICY.read_text())

    def test_each_platform_row_names_its_fixed_artifact_kind_and_stable_identity(self):
        self.assertEqual(set(self.policy["platforms"]), set(NIGHTLY_TRUST))
        for platform, (field, value) in NIGHTLY_TRUST.items():
            row = self.policy["platforms"][platform]
            with self.subTest(platform=platform):
                self.assertEqual(row["unsigned_artifact_name"], f"nightly-unsigned-{platform}",
                                 "the artifact kind is fixed by platform, never a caller pattern")
                self.assertEqual(row["trust"], {field: value}, "trust must be stable's public identity")

    def test_trust_values_have_the_shapes_the_shared_signer_accepts(self):
        rows = self.policy["platforms"]
        self.assertRegex(rows["macos-aarch64"]["trust"]["apple_team_id"], r"^[A-Z0-9]{10}$")
        self.assertRegex(rows["linux-x86_64"]["trust"]["fingerprint"], r"^[0-9A-F]{40}$")
        subject = rows["windows-x86_64"]["trust"]["publisher_subject"]
        self.assertTrue(0 < len(subject) <= 1024 and not set(subject) & {"\r", "\n", "\0"})

    def test_both_callers_use_one_shared_pin_in_uses_and_tooling_revision(self):
        pins = set()
        for name, callee in (("nightly.yml", "nightly-unsigned.yml"), ("nightly-sign.yml", "nightly-release.yml")):
            source = (ROOT / ".github/workflows" / name).read_text()
            uses = re.findall(r"(?m)^    uses: storytold/craft-common-actions/\.github/workflows/"
                              + re.escape(callee) + r"@(\S+)$", source)
            tooling = re.findall(r"(?m)^      tooling-revision: (\S+)$", source)
            with self.subTest(workflow=name):
                self.assertEqual(len(uses), 1)
                self.assertEqual(tooling, uses, "the defining workflow SHA and tooling input must match")
            pins.update(uses)
        self.assertEqual(len(pins), 1, "producer and signer must pin the same shared commit")
        self.assertRegex(pins.pop(), r"^[0-9a-f]{40}$", "pin a full immutable commit, never a branch or placeholder")


class NightlyEnabledCallers(unittest.TestCase):
    """Producer and signer run only in their exact context, for the reviewed manual actors."""

    GUARDS = {
        "nightly.yml": "github.repository == 'storytold/figcraft' && github.ref == 'refs/heads/main' && "
                       "github.ref_type == 'branch' && (github.event_name == 'schedule' || "
                       "github.event_name == 'workflow_dispatch')",
        "nightly-sign.yml": "github.repository == 'storytold/figcraft' && github.event_name == 'workflow_dispatch' && "
                            "github.ref == 'refs/heads/release' && github.ref_type == 'branch'",
    }

    def test_enabled_jobs_keep_exact_context_guards(self):
        for name, expected in self.GUARDS.items():
            jobs = (ROOT / ".github/workflows" / name).read_text().split("\njobs:\n", 1)[1]
            guards = job_conditions(jobs)
            with self.subTest(workflow=name):
                self.assertEqual(len(guards), 1)
                indicator, expression = guards[0]
                self.assertEqual(indicator, ">-")
                self.assertEqual(" ".join(expression.split()), expected, "guards may only change with review")

    def test_manual_actors_are_the_release_managers_in_caller_and_policy(self):
        source = (ROOT / ".github/workflows/nightly.yml").read_text()
        policy = json.loads(INSTALLED_POLICY.read_text())
        self.assertEqual(reviewed_manual_actors(source, policy), RELEASE_MANAGERS)
        self.assertEqual(policy["automatic_signer_actors"], [], "no automatic signer without a schedule")

    def test_an_actor_outside_the_release_managers_is_refused(self):
        source = (ROOT / ".github/workflows/nightly.yml").read_text()
        policy = json.loads(INSTALLED_POLICY.read_text())
        caller = json.dumps(RELEASE_MANAGERS, separators=(",", ":"))
        widened = sorted(RELEASE_MANAGERS + ["echelon-robot"], key=str.lower)
        for change in ("caller", "policy", "both", "order", "case"):
            mutated_source, mutated_policy = source, copy.deepcopy(policy)
            if change in ("caller", "both"):
                mutated_source = source.replace(caller, json.dumps(widened, separators=(",", ":")))
            if change in ("policy", "both"):
                mutated_policy["manual_actors"] = widened
            if change == "order":
                mutated_policy["manual_actors"] = list(reversed(RELEASE_MANAGERS))
            if change == "case":
                mutated_policy["manual_actors"] = [login.lower() for login in RELEASE_MANAGERS]
            with self.subTest(change=change), self.assertRaises(ValueError):
                reviewed_manual_actors(mutated_source, mutated_policy)


class NightlyInstalledPolicy(unittest.TestCase):
    """The installed v3 policy matches the callers, the repository and its documented sources."""

    EXPECTED_KEYS = {
        "schema", "repository", "repository_id", "producer_workflow_id",
        "producer_workflow_path", "producer_tooling_revision", "producer_workflow_ref",
        "signer_workflow_path", "signer_tooling_revision", "release_branch", "manual_actors",
        "automatic_signer_actors", "max_age_seconds", "candidate_policy",
        "max_artifact_bytes", "platforms",
    }

    def setUp(self):
        self.assertFalse(INSTALLED_POLICY.is_symlink())
        self.policy = json.loads(INSTALLED_POLICY.read_text())

    def test_exact_v3_fields_and_repository_identity(self):
        self.assertEqual(set(self.policy), self.EXPECTED_KEYS)
        self.assertEqual(self.policy["schema"], "craft-nightly-release-policy/v3")
        self.assertEqual(self.policy["repository"], "storytold/figcraft")
        self.assertEqual(self.policy["release_branch"], "release")
        self.assertIs(type(self.policy["repository_id"]), int)
        self.assertGreater(self.policy["repository_id"], 0)
        # producer_workflow_id is None until nightly.yml is registered on main.
        producer_id = self.policy["producer_workflow_id"]
        if producer_id is not None:
            self.assertIs(type(producer_id), int)
            self.assertGreater(producer_id, 0)
            self.assertNotEqual(producer_id,
                                self.policy["candidate_policy"]["required_workflows"][0]["workflow_id"],
                                "the producer is not its own quality workflow")

    def test_paths_and_tooling_match_the_callers(self):
        pin = caller_pin("nightly.yml")
        self.assertEqual(caller_pin("nightly-sign.yml"), pin)
        self.assertEqual(self.policy["producer_workflow_path"], ".github/workflows/nightly.yml")
        self.assertEqual(self.policy["signer_workflow_path"], ".github/workflows/nightly-sign.yml")
        self.assertEqual(self.policy["producer_tooling_revision"], pin)
        self.assertEqual(self.policy["signer_tooling_revision"], pin)
        self.assertEqual(self.policy["producer_workflow_ref"],
                         "storytold/craft-common-actions/.github/workflows/nightly-unsigned.yml@" + pin)

    def test_platforms_equal_the_producer(self):
        built = producer_platforms((ROOT / ".github/workflows/nightly.yml").read_text())
        self.assertEqual(built, list(NIGHTLY_PLATFORMS), "one release carries Windows, Linux and macOS")
        self.assertEqual(set(self.policy["platforms"]), set(built))

    def test_bounds(self):
        self.assertIs(type(self.policy["max_age_seconds"]), int)
        self.assertTrue(0 < self.policy["max_age_seconds"] <= 604800)
        self.assertIs(type(self.policy["max_artifact_bytes"]), int)
        self.assertTrue(0 < self.policy["max_artifact_bytes"] <= 16 * 1024 ** 3)

    def test_candidate_workflows_run_on_every_main_push(self):
        candidate = self.policy["candidate_policy"]
        self.assertEqual(set(candidate), {"schema", "epoch", "repository", "branch", "required_workflows", "sonar"})
        self.assertEqual(candidate["schema"], "craft-nightly-candidate-policy/v3")
        self.assertEqual((candidate["repository"], candidate["branch"]), ("storytold/figcraft", "main"))
        self.assertTrue(isinstance(candidate["epoch"], str) and 0 < len(candidate["epoch"]) <= 128)
        rows = candidate["required_workflows"]
        self.assertTrue(rows)
        for row in rows:
            with self.subTest(workflow=row["workflow_path"]):
                self.assertEqual(set(row), {"workflow_id", "workflow_path", "jobs"})
                self.assertIs(type(row["workflow_id"]), int)
                self.assertNotEqual(row["workflow_path"], self.policy["producer_workflow_path"])
                source = (ROOT / row["workflow_path"]).read_text()
                push = re.search(r"(?m)^  push:\n((?:    .*\n)+)", source)
                self.assertIsNotNone(push, "a required workflow must run on main pushes")
                self.assertIn("    branches: [main]\n", push[1])
                self.assertNotRegex(push[1], r"paths", "a path filter would leave candidates without a run")
                names = [job["name"] for job in row["jobs"]]
                self.assertEqual(len(names), len(set(names)))
                for job in row["jobs"]:
                    self.assertEqual(set(job), {"name", "app_id", "applicability"})
                    self.assertEqual((job["app_id"], job["applicability"]), (15368, "required"))

    def test_sonar_identity_and_reviewed_scope(self):
        sonar = self.policy["candidate_policy"]["sonar"]
        properties = dict(line.split("=", 1) for line in (ROOT / "sonar-project.properties").read_text().splitlines()
                          if line and not line.startswith("#"))
        self.assertEqual(sonar["organization"], properties["sonar.organization"])
        self.assertEqual(sonar["project"], properties["sonar.projectKey"])
        self.assertEqual((sonar["app_id"], sonar["check_name"]), (12526, "SonarCloud Code Analysis"))
        self.assertEqual(sonar["details_url"], "https://sonarcloud.io/dashboard?id=" + sonar["project"] + "&branch=main")
        accepted = {"bugs": "BUG", "vulnerabilities": "VULNERABILITY", "code_smells": "CODE_SMELL",
                    "security_rating": "VULNERABILITY", "new_security_rating": "VULNERABILITY"}
        self.assertEqual({accepted[c["metric"]] for c in sonar["accepted_conditions"]}, set(sonar["issue_types"]))
        metrics = [c["metric"] for c in sonar["accepted_conditions"] + sonar["required_conditions"]]
        self.assertEqual(len(metrics), len(set(metrics)))
        keys = [item["key"] for item in sonar["debt"]]
        self.assertEqual(len(keys), len(set(keys)))
        for item in sonar["debt"]:
            with self.subTest(issue=item["key"]):
                self.assertEqual(set(item), {"key", "rule", "component", "type", "severity", "hash"})
                self.assertTrue(item["component"].startswith(sonar["project"] + ":"))
                self.assertIn(item["type"], sonar["issue_types"])
                self.assertRegex(item["hash"], r"^[0-9a-f]{32}$")


if __name__ == "__main__":
    unittest.main(verbosity=2)
