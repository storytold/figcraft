#!/usr/bin/env python3
"""Nightly workflow and policy contract regression tests (local static checks, not live API/crypto evidence)."""
import copy
import json
from pathlib import Path
import re
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[1]
INSTALLED_POLICY = ROOT / ".github/nightly-signing-policy.json"

DISPATCHER = ROOT / ".github/workflows/nightly-dispatch.yml"
SIGNER_BRANCH = "nightly-signer"


def inline_scripts(path):
    """Read literal run blocks, not a substitute for YAML/actionlint validation."""
    return [textwrap.dedent(block) for block in re.findall(
        r"^        run: \|\n((?:          .*\n|\n)+)", path.read_text(), re.MULTILINE)]

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
                            "github.ref == 'refs/heads/nightly-signer' && github.ref_type == 'branch'",
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
        self.assertEqual(self.policy["release_branch"], SIGNER_BRANCH, "the signer runs on its dedicated branch")
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

    def test_sonar_policy_keeps_the_shared_candidate_contract(self):
        """The Sonar shape craft-common-actions' candidate contract() admits, refused here first.

        An empty issue scope, an accepted condition outside the reviewed issue types, a metric in both
        lists, a missing live gate condition or a widened security bound is refused at admission, so
        this suite must catch it before a nightly run can. `debt` stays empty, so any unresolved
        vulnerability on main is still refused; accepting the new-code rating only records the
        reviewed A threshold instead of demanding a clean rating the live gate does not.
        """
        sonar = self.policy["candidate_policy"]["sonar"]
        accepted = {"bugs": "BUG", "vulnerabilities": "VULNERABILITY", "code_smells": "CODE_SMELL",
                    "security_rating": "VULNERABILITY", "new_security_rating": "VULNERABILITY"}
        gate = {"new_reliability_rating", "new_security_rating", "new_maintainability_rating",
                "new_coverage", "new_duplicated_lines_density", "new_security_hotspots_reviewed"}
        self.assertTrue(sonar["issue_types"], "an empty issue scope is refused as unreviewed")
        self.assertEqual(len(sonar["issue_types"]), len(set(sonar["issue_types"])))
        metrics = {}
        for field in ("accepted_conditions", "required_conditions"):
            for condition in sonar[field]:
                with self.subTest(field=field, metric=condition.get("metric")):
                    expected = {"metric", "comparator", "error_threshold"}
                    if field == "accepted_conditions":
                        expected.add("maximum")
                    self.assertEqual(set(condition), expected)
                    self.assertNotIn(condition["metric"], metrics, "one condition per gate metric")
                    metrics[condition["metric"]] = condition
                    self.assertIn(condition["comparator"], ("GT", "LT"))
                    self.assertRegex(condition["error_threshold"], r"^(?:0|[1-9][0-9]*)(?:\.[0-9]+)?$")
                    if field != "accepted_conditions":
                        continue
                    self.assertIn(condition["metric"], accepted, "an accepted metric outside the reviewed map")
                    self.assertIn(accepted[condition["metric"]], sonar["issue_types"],
                                  "an accepted failure must lie inside the reviewed issue scope")
                    self.assertEqual(condition["comparator"], "GT")
                    self.assertRegex(condition["maximum"], r"^(?:0|[1-9][0-9]*)(?:\.[0-9]+)?$")
                    maximum = float(condition["maximum"])
                    if condition["metric"] in ("security_rating", "new_security_rating"):
                        self.assertEqual(int(maximum), maximum, "the A..E bound is a whole rating")
                        self.assertTrue(1 <= maximum <= 5, "the security bound stays A..E")
                        self.assertEqual(condition["error_threshold"], "1", "the reviewed A threshold stays")
                    else:
                        self.assertLessEqual(maximum, len(sonar["debt"]), "accepted failures exceed the reviewed debt")
        self.assertEqual({accepted[c["metric"]] for c in sonar["accepted_conditions"]}, set(sonar["issue_types"]),
                         "accepted conditions and reviewed issue types must cover the same scope")
        self.assertEqual(set(metrics), gate, "every live gate condition stays gated exactly once")


SHARED = "storytold/craft-common-actions/.github/workflows/"
# App workflows reviewed to call the shared stable release.yml. That callee must carry the
# stable-only environment guard (dry runs and non-release refs bind no environment); the
# nightly-signer settings scan checks the resolved callee before creating or advancing the branch.
REVIEWED_STABLE_RELEASE_CALLERS = {"release.yml"}


def app_workflows():
    return {path.name: path.read_text() for path in sorted((ROOT / ".github/workflows").glob("*.y*ml"))}


WORKFLOW_KEY = re.compile(r'''("(?:[^"\\]|\\.)*"|'(?:[^']|'')*'|[^\s'"#{}\[\],:&*!|>%@`?-][^:#,{}]*?|-[^\s:#][^:#,{}]*?)\s*:(?:\s+(.*))?$''')


def _workflow_key(text):
    if text.startswith('"'):
        return None if "\\" in text else text[1:-1]
    if text.startswith("'"):
        return text[1:-1].replace("''", "'")
    return text


def _workflow_value(text):
    """A one-line scalar value without its comment and quotes; None when escaped or unterminated."""
    text = text.strip()
    if text.startswith('"'):
        quoted = re.match(r'"((?:[^"\\]|\\.)*)"\s*(?:#.*)?$', text)
        return None if quoted is None or "\\" in quoted.group(1) else quoted.group(1)
    if text.startswith("'"):
        quoted = re.match(r"'((?:[^']|'')*)'\s*(?:#.*)?$", text)
        return None if quoted is None else quoted.group(1).replace("''", "'")
    return re.split(r"\s+#", text, maxsplit=1)[0].strip()


def _strip_comment(text):
    """(text without its comment, unterminated) using a quote-aware scan.

    As in YAML, a quote opens a quoted scalar only where a scalar starts (line start or after
    `:`, `-`, `,`, `{` or `[`), and a `#` starts a comment only outside quotes, at the start or
    after whitespace.
    """
    quote, index = None, 0
    while index < len(text):
        char = text[index]
        if quote == '"':
            if char == "\\":
                index += 2
                continue
            if char == '"':
                quote = None
        elif quote == "'":
            if char == "'":
                if text[index + 1:index + 2] == "'":
                    index += 2
                    continue
                quote = None
        elif char in "\"'" and text[:index].rstrip()[-1:] in ("", ":", "-", ",", "{", "["):
            quote = char
        elif char == "#" and (index == 0 or text[index - 1] in " \t"):
            return text[:index].rstrip(), False
        index += 1
    return text.rstrip(), quote is not None


FLOW_QUOTED = re.compile(r"\"(?:[^\"\\]|\\.)*\"|'(?:[^']|'')*'")
FLOW_FORBIDDEN_KEYS = {"environment", "uses"}


def workflow_entries(source):
    """Every mapping entry in a workflow, plus the YAML forms this reader refuses.

    A narrow reader, not a YAML parser (none ships with the runner's Python). Each entry is
    (line, key, value, multiline): plain, single- and double-quoted keys and one-line values in
    block and simple one-line flow mappings. Block scalar bodies are skipped, and an entry whose
    value is a block scalar or continues on the next lines is marked multiline. It refuses whatever
    could hide a key from it: escaped double-quoted keys, explicit `?` keys, merge keys, anchors,
    aliases, tags, tab indentation and nested or multi-line flow.
    """
    lines = source.splitlines()
    entries, refused, block = [], [], None

    def continues(index, indent):
        for follow in lines[index + 1:]:
            stripped = follow.strip()
            if not stripped or stripped.startswith("#"):
                continue
            deeper = len(follow) - len(follow.lstrip(" ")) > indent
            return deeper and not stripped.startswith("- ") and WORKFLOW_KEY.match(stripped) is None
        return False

    for index, raw in enumerate(lines):
        number = index + 1
        stripped = raw.strip()
        indent = len(raw) - len(raw.lstrip(" "))
        if block is not None:
            if not stripped or indent > block:
                continue
            block = None
        if not stripped or stripped.startswith("#"):
            continue
        if "\t" in raw[:indent + 1]:
            refused.append(f"line {number}: tab indentation"); continue
        body, unterminated = _strip_comment(stripped)
        if unterminated:
            refused.append(f"line {number}: unterminated or multi-line quoted scalar"); continue
        if not body:
            continue
        while body.startswith("- "):
            body = body[2:].lstrip()
        if body == "?" or body.startswith("? "):
            refused.append(f"line {number}: explicit key"); continue
        match = WORKFLOW_KEY.match(body)
        value = body
        if match is not None:
            key, value = _workflow_key(match.group(1)), match.group(2) or ""
            if key is None:
                refused.append(f"line {number}: escaped double-quoted key"); continue
            if key == "<<":
                refused.append(f"line {number}: merge key"); continue
        plain = re.sub(r"\$\{\{.*?\}\}", "", value).strip()
        bare = re.sub(r"\"(?:[^\"\\]|\\.)*\"|'(?:[^']|'')*'", "''", plain)
        if re.match(r"[&*!]", plain):
            refused.append(f"line {number}: anchor, alias or tag")
        is_block = re.fullmatch(r"[|>][0-9+-]*", plain) is not None
        if match is not None:
            entries.append((number, key, _workflow_value(value),
                            is_block or ((value == "" or not is_block) and continues(index, indent))))
        if "{" in bare and bare != "{}":
            masked = re.fullmatch(r"\{([^{}\[\]]*)\}", FLOW_QUOTED.sub("''", value.strip()))
            flow = re.fullmatch(r"\{([^{}\[\]]*)\}", value.strip())
            if any(set(scalar[1:-1]) & set("#{}[],") for scalar in FLOW_QUOTED.findall(value)):
                refused.append(f"line {number}: flow mapping quoted scalar with # or flow punctuation")
            elif masked is None or flow is None:
                refused.append(f"line {number}: nested or multi-line flow mapping")
            else:
                # The masked text only proves the shape: a quoted key ("uses", 'environment') keeps
                # its identity, so the entries are read from the original mapping text, never ''.
                for item in flow.group(1).split(","):
                    entry = WORKFLOW_KEY.match(item.strip())
                    key = _workflow_key(entry.group(1)) if entry else None
                    if key is None:
                        refused.append(f"line {number}: unreadable flow mapping entry")
                    elif key in FLOW_FORBIDDEN_KEYS:
                        refused.append(f"line {number}: {key} in a flow mapping (must be block style)")
                    else:
                        entries.append((number, key, _workflow_value(entry.group(2) or ""), False))
        if match is not None and is_block:
            block = indent
    return entries, refused


def workflow_keys(source):
    """(line, key) for every mapping entry, plus the refused YAML forms."""
    entries, refused = workflow_entries(source)
    return [(number, key) for number, key, _, _ in entries], refused


SHARED_CALLEE = re.compile(r"storytold/craft-common-actions/\.github/workflows/([\w.-]+\.ya?ml)@(\S+)", re.IGNORECASE)
INSPECTED_KEYS = {"environment", "uses"}


def trust_closure_problems(workflows):
    """The nightly-signer trust-closure scan over app workflow sources; empty when the tree is safe.

    Every key it inspects comes from workflow_entries(), never a regex over the raw text.
    """
    problems, stable_callers, signer_calls = [], set(), 0
    for name, source in workflows.items():
        entries, refused = workflow_entries(source)
        problems.extend(name + " uses YAML the scan cannot read (" + reason + ")" for reason in refused)
        for number, key, value, multiline in entries:
            if key not in INSPECTED_KEYS:
                continue
            if key == "environment":
                problems.append(f"{name} binds an environment directly (line {number})")
                continue
            if value is None or multiline:
                problems.append(f"{name} has a uses value the scan cannot read (line {number})")
                continue
            callee = SHARED_CALLEE.fullmatch(value)
            if callee is None:
                if re.search(r"craft-common-actions", value, re.IGNORECASE) and re.search(r"workflows/(nightly-)?release\.ya?ml", value, re.IGNORECASE):
                    problems.append(f"{name} has an unrecognised shared release caller (line {number})")
                continue
            workflow, ref = callee.group(1), callee.group(2)
            if workflow.lower() in ("nightly-release.yml", "nightly-release.yaml"):
                if name == "nightly-sign.yml":
                    signer_calls += 1
                if name != "nightly-sign.yml" or not re.fullmatch(r"[0-9a-f]{40}", ref):
                    problems.append(name + " calls nightly-release.yml@" + ref)
            if workflow.lower() in ("release.yml", "release.yaml"):
                stable_callers.add(name)
                if ref != "main" and not re.fullmatch(r"[0-9a-f]{40}", ref):
                    problems.append(name + " calls release.yml@" + ref + "; use @main or a reviewed full SHA")
    if stable_callers != REVIEWED_STABLE_RELEASE_CALLERS:
        problems.append("shared release.yml callers are " + ", ".join(sorted(stable_callers)))
    if signer_calls != 1:
        problems.append("nightly-sign.yml must call nightly-release.yml exactly once")
    return problems


class NightlySignerBranchTrustClosure(unittest.TestCase):
    """On refs/heads/nightly-signer, only the pinned signer caller may reach an environment."""

    def test_trust_closure_scan_passes_on_this_tree(self):
        self.assertEqual(trust_closure_problems(app_workflows()), [])

    def test_trust_closure_scan_refuses_each_route_into_an_environment(self):
        tree = app_workflows()
        bad = {
            "a workflow binds an environment": {**tree, "extra.yml": "jobs:\n  x:\n    runs-on: ubuntu-latest\n    environment: release\n"},
            "a double-quoted environment key": {**tree, "extra.yml": 'jobs:\n  x:\n    runs-on: ubuntu-latest\n    "environment": release\n'},
            "a single-quoted environment key": {**tree, "extra.yml": "jobs:\n  x:\n    runs-on: ubuntu-latest\n    'environment' : release\n"},
            "an environment mapping": {**tree, "extra.yml": "jobs:\n  x:\n    runs-on: ubuntu-latest\n    environment:\n      name: release\n"},
            "an environment in a flow mapping": {**tree, "extra.yml": "jobs:\n  x: {runs-on: ubuntu-latest, environment: release}\n"},
            "an escaped key": {**tree, "extra.yml": 'jobs:\n  x:\n    runs-on: ubuntu-latest\n    "environ\\u006dent": release\n'},
            "a merge key": {**tree, "extra.yml": "base: &b\n  runs-on: ubuntu-latest\njobs:\n  x:\n    <<: *b\n"},
            "an explicit key": {**tree, "extra.yml": "jobs:\n  x:\n    ? environment\n    : release\n"},
            "a nested flow mapping": {**tree, "extra.yml": "jobs:\n  x: {steps: [{run: a}]}\n"},
            "a double-quoted uses key copying the signer": {**tree, "extra.yml": 'jobs:\n  x:\n    "uses": ' + SHARED + "nightly-release.yml@" + "a" * 40 + "\n"},
            "a single-quoted uses key": {**tree, "extra.yml": "jobs:\n  x:\n    'uses': " + SHARED + "nightly-release.yml@" + "a" * 40 + "\n"},
            "a quoted uses value": {**tree, "extra.yml": 'jobs:\n  x:\n    uses: "' + SHARED + "nightly-release.yml@" + "a" * 40 + '"\n'},
            "uses in a flow mapping": {**tree, "extra.yml": "jobs:\n  x: {uses: " + SHARED + "nightly-release.yml@" + "a" * 40 + "}\n"},
            "a differently cased owner": {**tree, "extra.yml": "jobs:\n  x:\n    uses: Storytold/Craft-Common-Actions/.github/workflows/nightly-release.yml@" + "a" * 40 + "\n"},
            "a folded uses value": {**tree, "extra.yml": "jobs:\n  x:\n    uses: >-\n      " + SHARED + "nightly-release.yml@" + "a" * 40 + "\n"},
            "a continued uses value": {**tree, "extra.yml": "jobs:\n  x:\n    uses: " + SHARED + "\n      nightly-release.yml@" + "a" * 40 + "\n"},
            "a quoted second signer call": {**tree, "nightly-sign.yml": tree["nightly-sign.yml"] + "  again:\n    \"uses\": " + SHARED + "nightly-release.yml@" + "a" * 40 + "\n"},
            "a hash inside a quoted flow scalar": {**tree, "extra.yml": 'jobs:\n  x: {name: "a} # b", runs-on: ubuntu-latest, environment: release, steps: []}\n'},
            "a single-quoted hash in a flow scalar": {**tree, "extra.yml": "jobs:\n  x: {name: 'a # b', runs-on: ubuntu-latest}\n"},
            "a comma inside a quoted flow scalar": {**tree, "extra.yml": 'jobs:\n  x: {name: "a, environment: release", runs-on: ubuntu-latest}\n'},
            "a bracket inside a quoted flow scalar": {**tree, "extra.yml": 'jobs:\n  x: {name: "[a]", runs-on: ubuntu-latest}\n'},
            "an unterminated quote": {**tree, "extra.yml": 'jobs:\n  x:\n    name: "a\n      b"\n    runs-on: ubuntu-latest\n'},
            "environment in any flow mapping": {**tree, "extra.yml": "jobs:\n  x:\n    runs-on: ubuntu-latest\n    with: {environment: release}\n"},
            "a double-quoted environment key in a flow mapping": {**tree, "extra.yml": 'jobs:\n  x:\n    runs-on: ubuntu-latest\n    with: {"environment": release}\n'},
            "a single-quoted environment key in a flow mapping": {**tree, "extra.yml": "jobs:\n  x:\n    runs-on: ubuntu-latest\n    with: {'environment': release}\n"},
            "a double-quoted uses key in a flow mapping": {**tree, "extra.yml": 'jobs:\n  x: {"uses": ' + SHARED + "nightly-release.yml@" + "a" * 40 + "}\n"},
            "uses in any flow mapping": {**tree, "extra.yml": "jobs:\n  x:\n    steps:\n      - {uses: actions/checkout@v4}\n"},
            "another caller of the signer": {**tree, "extra.yml": "jobs:\n  x:\n    uses: " + SHARED + "nightly-release.yml@" + "a" * 40 + "\n"},
            "a mutable signer pin": {**tree, "nightly-sign.yml": re.sub(r"nightly-release\.yml@[0-9a-f]{40}", "nightly-release.yml@main", tree["nightly-sign.yml"])},
            "a new stable release caller": {**tree, "extra.yml": "jobs:\n  x:\n    uses: " + SHARED + "release.yml@main\n"},
            "a stable caller on a branch or tag ref": {**tree, "release.yml": tree["release.yml"].replace(SHARED + "release.yml@main", SHARED + "release.yml@v1")},
        }
        for label, workflows in bad.items():
            with self.subTest(case=label):
                self.assertTrue(trust_closure_problems(workflows))

    def test_quoted_keys_in_a_flow_mapping_are_read_not_masked(self):
        entries, refused = workflow_entries('jobs:\n  x: {runs-on: ubuntu-latest, "name": build}\n')
        self.assertEqual(refused, [])
        self.assertIn((2, "name", "build", False), entries, "a quoted key keeps its identity")
        self.assertIn((2, "runs-on", "ubuntu-latest", False), entries)
        _, refused = workflow_entries("jobs:\n  x: {runs-on: ubuntu-latest, 'environment': release}\n")
        self.assertEqual(refused, ["line 2: environment in a flow mapping (must be block style)"])

    def test_signer_dispatch_guard_and_policy_name_the_same_branch(self):
        sign = (ROOT / ".github/workflows/nightly-sign.yml").read_text()
        (_, guard), = job_conditions(sign.split("\njobs:\n", 1)[1])
        self.assertIn("github.ref == 'refs/heads/" + SIGNER_BRANCH + "'", guard)
        self.assertNotIn("refs/heads/release", sign, "the signer no longer runs on the stable branch")
        self.assertEqual(re.findall(r"(?m)^      app-name: (\S+)$", sign), ["figcraft"], "the shared signer needs the reviewed registry app")
        # This app ships no completion dispatcher yet: that workflow forwards scheduled producers
        # only, and no schedule is approved. Adding it is part of a reviewed cadence change.
        self.assertFalse(DISPATCHER.exists(), "a completion dispatcher needs its own reviewed activation")
        self.assertEqual(json.loads(INSTALLED_POLICY.read_text())["release_branch"], SIGNER_BRANCH)
        self.assertFalse((ROOT / "docs/nightly-signing-policy.example.json").exists(),
                         "a policy example must then name the signer branch")
if __name__ == "__main__":
    unittest.main(verbosity=2)
