import importlib.util
import json
from pathlib import Path
import unittest
import urllib.error
from unittest import mock

MODULE = Path(__file__).parents[1] / "crates-to-publish.py"
SPEC = importlib.util.spec_from_file_location("crates_to_publish", MODULE)
crates = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(crates)


def response(payload):
    class Response:
        def __enter__(self):
            return self

        def __exit__(self, *args):
            return False

        def read(self):
            return json.dumps(payload).encode()

    return Response()


def http_404(url, *_args, **_kwargs):
    raise urllib.error.HTTPError(url, 404, "not found", None, None)


class CrateSetupTests(unittest.TestCase):
    def test_missing_crate_name_fails(self):
        with mock.patch.object(crates.urllib.request, "urlopen", side_effect=http_404):
            with self.assertRaisesRegex(SystemExit, "setup is incomplete"):
                crates.check_crate_setup([{"name": "graphviz-pure-new"}])

    def test_missing_expected_owner_fails(self):
        with mock.patch.object(crates.urllib.request, "urlopen", return_value=response({"users": []})):
            with self.assertRaisesRegex(SystemExit, "setup is incomplete"):
                crates.check_crate_setup([{"name": "graphviz-pure-new"}])

    def test_expected_owner_passes_and_missing_version_is_not_an_error(self):
        package = {"name": "graphviz-pure-new", "version": "1.2.1"}
        with mock.patch.object(crates.urllib.request, "urlopen", return_value=response({"users": [{"login": "cloudbring"}]})):
            crates.check_crate_setup([package])
        with mock.patch.object(crates.urllib.request, "urlopen", side_effect=http_404):
            self.assertEqual(crates.already_published([package]), [])


if __name__ == "__main__":
    unittest.main()
