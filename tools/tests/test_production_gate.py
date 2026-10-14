"""Acceptance gates reject missing perception, stale artifacts, mux truncation and out-of-range scopes."""
import contextlib
import copy
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("production_gate", Path(__file__).parents[1] / "production_gate.py")
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


class CaptionGateTests(unittest.TestCase):
    def test_external_assets_must_be_vendored(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            scene = root / "film.scene.xml"
            scene.write_text('<scene><assets><image src="../external.png"/></assets></scene>')
            with self.assertRaisesRegex(gate.GateError, "outside project"):
                gate.vendored_scene(scene, root)
            scene.write_text('<scene><assets><image src="https://example.org/image.png"/></assets></scene>')
            with self.assertRaisesRegex(gate.GateError, "Vendor remote"):
                gate.vendored_scene(scene, root)
            scene.write_text('<scene><assets><image src="assets/image.png"/></assets></scene>')
            gate.vendored_scene(scene, root)

    def test_captions_must_fit_the_final_timeline_and_reading_area(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "film.vtt"
            path.write_text("WEBVTT\n\n00:00.000 --> 00:02.000\nA readable line.\n")
            self.assertEqual(gate.captions(path, 2)["cues"], 1)
            with self.assertRaisesRegex(gate.GateError, "timeline"):
                gate.captions(path, 1)
            path.write_text("WEBVTT\n\n00:00.000 --> 00:02.000\n" + "x" * 43 + "\n")
            with self.assertRaisesRegex(gate.GateError, "42-character"):
                gate.captions(path, 2)
            path.write_text("WEBVTT\n\n00:00.000 --> 00:02.000\nOne\nTwo\nThree\n")
            with self.assertRaisesRegex(gate.GateError, "42-character"):
                gate.captions(path, 2)


class PremiumGateTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        for folder in ("qa", "out", "work", "assets"):
            (self.root / folder).mkdir()
        for name in ("film.scene.xml", "out/film.mp4", "out/film.vtt", "assets/image.png",
                     "work/encode.json", "work/engine", "work/review.txt"):
            (self.root / name).write_text(name)
        self.review = gate.artifact(self.root / "work/review.txt")
        checks = {name: {"exit": 0} for name in ("validate", "resolve", "sr-audiocheck", "sr-margin",
                   "sr-sibilance", "decoded_audio", "scopes", "black_freeze", "captions", "audio_tail")}
        self.tech = {"version": 1, "status": "TECHNICAL PASS", "project": str(self.root), "duration": 100,
                     "inputs": gate.inputs(self.root), "checks": checks,
                     "sidecars": [gate.artifact(self.root / "out/film.vtt")]}
        self.tech["encoded_utc"] = "2026-10-02T12:00:00+00:00"
        for key, name in (("scene", "film.scene.xml"), ("video", "out/film.mp4"),
                          ("engine", "work/engine"), ("encode_report", "work/encode.json")):
            self.tech[key] = gate.artifact(self.root / name)
        gate.write_json(self.root / "qa/TECHNICAL.json", self.tech)
        gate.template(type("Args", (), {"project": self.root}))
        self.manifest = json.loads((self.root / "qa/PREMIUM.json").read_text())

    def complete(self):
        m = self.manifest
        m["producer_session"], m["reviewer_session"] = "producer", "reviewer"
        m["sidecars"] = [gate.artifact(self.root / "out/film.vtt")]
        for name in gate.REPORTS:
            p = self.root / "qa" / name
            p.write_text("Actual review evidence")
            m["reports"][name] = gate.artifact(p)
        for role, row in m["roles"].items():
            row["session"] = "reviewer" if role == "pals-reviewer" else "producer" if role == "pals-film-producer" else role
        for row in m["categories"]:
            row.update(score=9, inspection=["direct-playback", "direct-listening"],
                       evidence=[self.review], rationale="Reviewed final cut", reviewed_by="reviewer", reference="ref")
        m["coordinator_acceptance"].update(session="pals-coordinator", evidence=[self.review])
        m["art_direction"].update(session="pals-art-director", signature_still=self.review,
            approved_utc="2026-10-02T11:00:00+00:00", reference_board=[self.review],
            visible_advance="Specific improvement over reference", evidence=[self.review])
        m["editorial_review"].update(reviewer="reviewer", method="sentence-and-silence-frame-review",
                                     failures=0, evidence=[self.review])
        m["professional_references"] = [{"id": "ref", "title": "Selected professional benchmark",
            "source": "Reference source and timestamp", "comparison": "Documented comparison",
            "reviewer": "reviewer", "evidence": [self.review]}]
        m["sampling"] = [{"start": 0, "end": 100, "reviewer": "reviewer", "reason": "Complete film review",
                          "tags": ["hook", "quarter", "middle", "three-quarter", "ending", "transitions",
                                   "complex", "modified", "black-freeze-review", "caption-sync"],
                          "evidence": [self.review]}]
        for key in ("full_playback", "regression"):
            m[key].update(reviewer="reviewer", method="direct-audiovisual-playback", evidence=[self.review])

    def check(self):
        gate.write_json(self.root / "qa/PREMIUM.json", self.manifest)
        return gate.premium(self.root)

    def test_template_never_certifies_quality(self):
        with self.assertRaises(gate.GateError):
            self.check()

    def test_complete_independent_review_passes(self):
        self.complete()
        self.assertEqual(self.check()["overall"], "9.00")

    def test_proxy_scores_cannot_replace_perceptual_evidence(self):
        self.complete()
        self.manifest["categories"][5]["inspection"] = ["ASR", "loudness"]
        with self.assertRaisesRegex(gate.GateError, "direct-listening"):
            self.check()

    def test_changed_caption_invalidates_review(self):
        self.complete()
        (self.root / "out/film.vtt").write_text("Changed captions")
        with self.assertRaisesRegex(gate.GateError, "Artifact changed"):
            self.check()

    def test_changed_asset_invalidates_technical_review(self):
        self.complete()
        (self.root / "assets/image.png").write_text("Changed pixels")
        with self.assertRaisesRegex(gate.GateError, "Sources/assets changed"):
            self.check()

    def test_changed_encode_or_engine_invalidates_review(self):
        for name in ("out/film.mp4", "work/engine", "work/encode.json"):
            with self.subTest(name=name):
                self.complete()
                p = self.root / name
                old = p.read_text()
                p.write_text("Changed")
                with self.assertRaisesRegex(gate.GateError, "Artifact changed"):
                    self.check()
                p.write_text(old)

    def test_high_average_cannot_hide_weak_category(self):
        self.complete()
        for row in self.manifest["categories"]:
            row["score"] = 10
        self.manifest["categories"][0]["score"] = 8.49
        with self.assertRaisesRegex(gate.GateError, "Score below"):
            self.check()

    def test_visual_and_cinematography_require_nine(self):
        self.complete()
        self.manifest["categories"][0]["score"] = 8.9
        with self.assertRaisesRegex(gate.GateError, "Visual quality"):
            self.check()
        self.complete()
        self.manifest["categories"][2]["score"] = 8.9
        with self.assertRaisesRegex(gate.GateError, "Cinematography"):
            self.check()

    def test_signature_approval_after_encode_and_editorial_fail_block_release(self):
        self.complete()
        self.manifest["art_direction"]["approved_utc"] = "2026-10-02T13:00:00+00:00"
        with self.assertRaisesRegex(gate.GateError, "before encoding"):
            self.check()
        self.complete()
        self.manifest["editorial_review"]["failures"] = 1
        with self.assertRaisesRegex(gate.GateError, "editorial review"):
            self.check()

    def test_overall_never_rounds_up_to_threshold(self):
        self.complete()
        self.manifest["categories"][1]["score"] = 8.99
        with self.assertRaisesRegex(gate.GateError, "Overall is below"):
            self.check()

    def test_nan_and_boolean_scores_are_rejected(self):
        self.complete()
        for value in (True, "NaN", "Infinity"):
            self.manifest["categories"][0]["score"] = value
            with self.assertRaises((gate.GateError, ValueError)):
                self.check()

    def test_producer_cannot_accept_own_review(self):
        self.complete()
        self.manifest["reviewer_session"] = "producer"
        with self.assertRaisesRegex(gate.GateError, "independent"):
            self.check()

    def test_professional_benchmark_and_coordinator_acceptance_are_required(self):
        self.complete()
        self.manifest["professional_references"] = []
        with self.assertRaisesRegex(gate.GateError, "professional reference"):
            self.check()
        self.complete()
        self.manifest["coordinator_acceptance"]["session"] = ""
        with self.assertRaisesRegex(gate.GateError, "coordinator acceptance"):
            self.check()

    def test_missing_middle_sample_blocks_release(self):
        self.complete()
        self.manifest["sampling"][0]["end"] = 20
        with self.assertRaisesRegex(gate.GateError, "quarter"):
            self.check()

    def test_open_issue_or_unverified_fix_blocks_release(self):
        self.complete()
        self.manifest["issues"] = [{"id": "V1", "owner": "producer", "root_cause": "camera",
            "classification": "Blocking", "status": "FIXED", "accepted_by": "reviewer"}]
        with self.assertRaisesRegex(gate.GateError, "Unresolved issue"):
            self.check()

    def test_technical_result_without_scopes_is_incomplete(self):
        self.complete()
        tech = json.loads((self.root / "qa/TECHNICAL.json").read_text())
        del tech["checks"]["scopes"]
        gate.write_json(self.root / "qa/TECHNICAL.json", tech)
        self.manifest["technical"] = gate.artifact(self.root / "qa/TECHNICAL.json")
        with self.assertRaisesRegex(gate.GateError, "Incomplete technical checks"):
            self.check()

    def test_new_source_file_invalidates_review(self):
        self.complete()
        (self.root / "assets/new.png").write_text("New image")
        with self.assertRaisesRegex(gate.GateError, "Sources/assets changed"):
            self.check()


def scopes_report(**levels):
    """A `scene-render scopes --format json` report of a clean limited-range video, with `levels` replaced."""
    channels = {"r": 0.0, "g": 0.0, "b": 0.0, "y": 0.0}
    report = {
        "schema": "scene-render-scopes/1", "file": "film.mp4",
        "source": {"kind": "video", "width": 1920, "height": 1080, "matrix": "BT.709", "range": "limited"},
        "frames": {"analysed": 300, "step": 1},
        "levels": {"samples": 622080000, "clipping": {"black": dict(channels), "white": dict(channels)},
                   "float_range": None, "legal": None, "illegal": 0.0,
                   "saturation": {"chroma_limit": 0.44675, "above_limit": 0.0, "out_of_gamut": 0.0,
                                  "max_chroma": 0.3}},
        "worst": {"black_clipped": {"frame": 0, "value": 0.0}, "white_clipped": {"frame": 0, "value": 0.0},
                  "illegal": {"frame": 0, "value": 0.0}, "out_of_gamut": {"frame": 0, "value": 0.0},
                  "above_chroma_limit": {"frame": 0, "value": 0.0}},
        "scopes": {}, "warnings": [],
    }
    report["levels"].update(levels)
    return report


LIMITS = {"illegal": 0.01, "out_of_gamut": 0.01, "above_chroma_limit": 0.01,
          "black_clipped": None, "white_clipped": None}


class ScopesGateTests(unittest.TestCase):
    def test_clean_film_passes(self):
        self.assertEqual(gate.scopes_findings(scopes_report(), LIMITS), [])

    def test_each_breach_names_the_measure_and_its_worst_frame(self):
        tail = ": 2.00% of samples, limit 1.00%; worst frame 41 (9.50%)"
        cases = (("illegal", "Levels outside the legal range"), ("out_of_gamut", "Colours outside the gamut"),
                 ("above_chroma_limit", "Chroma above the limit"))
        for measure, label in cases:
            message = label + tail
            with self.subTest(measure=measure):
                report = scopes_report()
                if measure == "illegal":
                    report["levels"]["illegal"] = 0.02
                else:
                    report["levels"]["saturation"][{"out_of_gamut": "out_of_gamut",
                                                    "above_chroma_limit": "above_limit"}[measure]] = 0.02
                report["worst"][measure] = {"frame": 41, "value": 0.095}
                self.assertEqual(gate.scopes_findings(report, LIMITS), [message])

    def test_a_measure_at_its_limit_passes(self):
        report = scopes_report(illegal=0.01)
        self.assertEqual(gate.scopes_findings(report, LIMITS), [])

    def test_sources_without_a_legal_range_skip_the_illegal_limit(self):
        report = scopes_report(illegal=None)
        report["worst"]["illegal"] = None
        self.assertEqual(gate.scopes_findings(report, dict(LIMITS, illegal=0.0)), [])

    def test_clipping_is_checked_per_channel_only_when_limited(self):
        report = scopes_report()
        report["levels"]["clipping"]["white"]["g"] = 0.2
        report["worst"]["white_clipped"] = {"frame": 7, "value": 0.6}
        self.assertEqual(gate.scopes_findings(report, LIMITS), [])
        self.assertEqual(gate.scopes_findings(report, dict(LIMITS, white_clipped=0.05)),
                         ["White clipping: 20.00% of samples, limit 5.00%; worst frame 7 (60.00%)"])
        report["levels"]["clipping"]["black"]["y"] = 0.3
        self.assertEqual(len(gate.scopes_findings(report, dict(LIMITS, black_clipped=0.05, white_clipped=0.05))), 2)

    def test_decoding_warnings_fail_the_candidate(self):
        report = scopes_report()
        report["warnings"] = ["frame 12: corrupt packet"]
        self.assertEqual(gate.scopes_findings(report, LIMITS), ["Scopes decoding warning: frame 12: corrupt packet"])

    def test_malformed_reports_are_rejected_not_passed(self):
        def mutate(path, value):
            report = scopes_report()
            node = report
            for key in path[:-1]:
                node = node[key]
            if value is KeyError:
                del node[path[-1]]
            else:
                node[path[-1]] = value
            return report

        cases = [
            ("unsupported schema", mutate(("schema",), "scene-render-scopes/2")),
            ("missing levels", mutate(("levels",), KeyError)),
            ("missing worst", mutate(("worst",), KeyError)),
            ("no frames", mutate(("frames", "analysed"), 0)),
            ("boolean frame count", mutate(("frames", "analysed"), True)),
            ("missing illegal", mutate(("levels", "illegal"), KeyError)),
            ("NaN illegal", mutate(("levels", "illegal"), float("nan"))),
            ("infinite gamut", mutate(("levels", "saturation", "out_of_gamut"), float("inf"))),
            ("negative chroma", mutate(("levels", "saturation", "above_limit"), -0.1)),
            ("fraction above one", mutate(("levels", "clipping", "white", "r"), 1.5)),
            ("string fraction", mutate(("levels", "clipping", "black", "y"), "0")),
            ("boolean fraction", mutate(("levels", "saturation", "above_limit"), False)),
            ("missing channel", mutate(("levels", "clipping", "black", "b"), KeyError)),
            ("missing clipping", mutate(("levels", "clipping"), KeyError)),
            ("negative worst frame", mutate(("worst", "illegal", "frame"), -1)),
            ("worst value out of range", mutate(("worst", "out_of_gamut", "value"), 2.0)),
            ("non-text warnings", mutate(("warnings",), [None])),
            ("warnings not a list", mutate(("warnings",), "none")),
        ]
        for name, report in cases:
            with self.subTest(name):
                with self.assertRaisesRegex(gate.GateError, "scopes report|Scopes analysed no frames"):
                    gate.scopes_findings(report, LIMITS)
        for report in (None, [], "scene-render-scopes/1", {}):
            with self.subTest(report=report):
                with self.assertRaisesRegex(gate.GateError, "Unsupported scopes report"):
                    gate.scopes_findings(report, LIMITS)

    def test_the_report_is_the_result_of_the_scopes_envelope(self):
        report = scopes_report()
        envelope = {"format": "scene-render-cli/1", "schema_version": "1.0.0", "command": "scopes", "ok": True,
                    "exit_code": 0, "result": report, "diagnostics": [], "error": None}
        self.assertEqual(gate.scopes_result(json.dumps(envelope) + "\n"), report)

    def test_scopes_output_other_than_a_successful_scopes_envelope_is_rejected(self):
        ok = {"format": "scene-render-cli/1", "schema_version": "1.0.0", "command": "scopes", "ok": True,
              "exit_code": 0, "result": scopes_report(), "diagnostics": [], "error": None}
        failed = dict(ok, ok=False, exit_code=2, result=None,
                      error={"kind": "io", "message": "cannot read film.mp4: gone", "codes": []})
        cases = [
            ("not JSON", "valid\n", "not one JSON document"),
            ("two documents", json.dumps(ok) + "\n" + json.dumps(ok), "not one JSON document"),
            ("bare report", json.dumps(scopes_report()), "envelope"),
            ("other format", json.dumps(dict(ok, format="scene-render-cli/2")), "envelope"),
            ("other command", json.dumps(dict(ok, command="inspect")), "not a successful scopes run"),
            ("failed run", json.dumps(failed), "cannot read film.mp4: gone"),
            ("ok claimed with an error status", json.dumps(dict(ok, exit_code=2)), "not a successful scopes run"),
            ("no result", json.dumps(dict(ok, result=None)), "Unsupported scopes report"),
        ]
        for name, stdout, message in cases:
            with self.subTest(name):
                with self.assertRaisesRegex(gate.GateError, message):
                    gate.scopes_findings(gate.scopes_result(stdout), LIMITS)

    def test_adversarial_report_cannot_bypass_the_limits(self):
        report = scopes_report(illegal=0.5)
        report["levels"]["status"] = "PASS"
        report["worst"]["illegal"] = None
        self.assertEqual(gate.scopes_findings(report, LIMITS),
                         ["Levels outside the legal range: 50.00% of samples, limit 1.00%"])
        report = scopes_report()
        before = copy.deepcopy(report)
        gate.scopes_findings(report, LIMITS)
        self.assertEqual(report, before)

    def test_invalid_limits_are_rejected(self):
        for limit in (-0.1, 1.5, float("nan"), True):
            with self.subTest(limit=limit):
                with self.assertRaisesRegex(gate.GateError, "limit must be a fraction"):
                    gate.scopes_findings(scopes_report(), dict(LIMITS, illegal=limit))

    def test_command_passes_the_chroma_limit_only_when_set(self):
        self.assertEqual(gate.scopes_argv("sr", "film.mp4"), ["sr", "scopes", "film.mp4", "--format", "json"])
        self.assertEqual(gate.scopes_argv("sr", "film.mp4", 0.45)[-2:], ["--chroma-limit", "0.45"])

    def test_threshold_flags_default_and_reject_non_fractions(self):
        base = ["technical", "p", "--scene", "s", "--video", "v", "--engine", "e", "--encode-report", "r",
                "--tools-dir", "t", "--captions", "c", "--voice-end", "1"]
        args = gate.arguments().parse_args(base)
        limits = {measure: getattr(args, flag) for measure, flag, _ in gate.SCOPE_LIMITS}
        self.assertEqual(limits, LIMITS)
        self.assertIsNone(args.chroma_limit)
        args = gate.arguments().parse_args(base + ["--max-white-clipped", "0.02", "--chroma-limit", "0.5"])
        self.assertEqual((args.max_white_clipped, args.chroma_limit), (0.02, 0.5))
        for flag, value in (("--max-illegal", "1.5"), ("--max-out-of-gamut", "-0.1"), ("--max-oversaturated", "nan"),
                            ("--max-black-clipped", "inf"), ("--chroma-limit", "0"), ("--chroma-limit", "nan")):
            with self.subTest(flag=flag, value=value):
                with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as exit:
                    gate.arguments().parse_args(base + [flag, value])
                self.assertEqual(exit.exception.code, 2)


class EncodedFrameTests(unittest.TestCase):
    def test_actual_decoded_frames_must_match_render_count(self):
        report = {"quality": "final", "frames": 1327, "size": [1080, 1920], "fps": 30}
        probe = {"streams": [{"codec_type": "video", "width": 1080, "height": 1920,
                  "avg_frame_rate": "30/1", "nb_read_frames": "1324"}], "format": {"duration": "44.233"}}
        with self.assertRaisesRegex(gate.GateError, "Frame count mismatch"):
            gate.check_video(report, "short.mp4", probe)
        probe["streams"][0]["nb_read_frames"] = "1327"
        self.assertAlmostEqual(gate.check_video(report, "short.mp4", probe), 1327 / 30)

    def test_the_encode_report_is_the_cli_envelope(self):
        envelope = {"format": "scene-render-cli/1", "schema_version": "1.0.0", "command": "encode", "ok": True,
                    "exit_code": 0, "result": {"outputs": [{"path": "film.mp4", "frames": 3}]},
                    "diagnostics": [], "error": None}
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "encode.json"
            path.write_text(json.dumps(envelope) + "\n")
            self.assertEqual(gate.reports(path), [{"path": "film.mp4", "frames": 3}])
            failed = dict(envelope, ok=False, exit_code=1, result=None,
                          error={"kind": "failed", "message": "1 construct not drawn", "codes": []})
            path.write_text(json.dumps(failed))
            with self.assertRaisesRegex(gate.GateError, "not a successful encode: 1 construct not drawn"):
                gate.reports(path)
            path.write_text(json.dumps(dict(envelope, command="render")))
            with self.assertRaisesRegex(gate.GateError, "not a successful encode"):
                gate.reports(path)
            path.write_text(json.dumps(dict(envelope, result={"outputs": "film.mp4"})))
            with self.assertRaisesRegex(gate.GateError, "result.outputs"):
                gate.reports(path)
            # the line-per-output report of earlier releases, and anything else, is refused
            path.write_text('{"path": "a.mp4", "frames": 3}\n{"path": "b.mp4", "frames": 3}\n')
            with self.assertRaisesRegex(gate.GateError, "not one JSON document"):
                gate.reports(path)
            path.write_text(json.dumps([{"path": "film.mp4"}]))
            with self.assertRaisesRegex(gate.GateError, "scene-render-cli/1 envelope"):
                gate.reports(path)

    def test_degraded_output_is_not_certified(self):
        with self.assertRaisesRegex(gate.GateError, "unsupported"):
            gate.check_video({"unsupported": ["missing shader"]}, "film.mp4", {})


if __name__ == "__main__":
    unittest.main()
