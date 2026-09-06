import contextlib
import importlib.util
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("helix_servers", Path(__file__).with_name("helix-servers.py"))
discovery = importlib.util.module_from_spec(spec)
spec.loader.exec_module(discovery)


class DiscoveryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.state = self.root / "state"
        self.state.mkdir()
        self.config = self.root / "config.json"
        self.config.write_text(json.dumps({"native": {
            "state_root": str(self.state), "instance_root": str(self.root / "instances")
        }}))
        self.id = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
        self.item = {
            "schema_version": 1, "id": self.id, "name": "Test Server",
            "instance_name": "test-server", "container_name": f"helix-game-{self.id}",
            "software": "paper", "minecraft_version": "26.2", "build": "112",
            "game_port": 25569, "rcon_password": "never-print-me",
            "artifact_url": "https://example.invalid/?token=secret",
        }
        self.save()

    def save(self):
        (self.state / f"{self.id}.json").write_text(json.dumps(self.item))

    def test_old_manifest_names_and_paths_without_container_labels(self):
        result = discovery.inventory(self.config)[0]
        self.assertEqual(result["name"], "Test Server")
        self.assertEqual(result["game"], "minecraft")
        self.assertEqual(result["plugins_path"], str(self.root / "instances" / self.id / "plugins"))
        self.assertFalse(result["data_exists"])
        self.assertNotIn("never-print-me", json.dumps(result))
        self.assertNotIn("artifact_url", result)

    def test_exact_selector_and_duplicate_names_fail_closed(self):
        servers = discovery.inventory(self.config)
        self.assertEqual(discovery.select(servers, name="test-server"), servers)
        for options in ({"name": "Test"}, {"instance_id": self.id[:8]}, {"name": "missing"}):
            with self.assertRaises(ValueError):
                discovery.select(servers, **options)
        with self.assertRaises(ValueError):
            discovery.select(servers * 2, name="Test Server")

    def test_identity_mismatch_is_not_silently_skipped(self):
        self.item["container_name"] = "unrelated-server"
        self.save()
        with self.assertRaises(ValueError):
            discovery.inventory(self.config)

    def test_unsupported_schema(self):
        self.item["schema_version"] = 2
        self.save()
        with self.assertRaises(ValueError):
            discovery.inventory(self.config)

    def test_non_minecraft_has_no_plugin_directory(self):
        self.item["kind"] = "valheim"
        self.save()
        self.assertIsNone(discovery.inventory(self.config)[0]["plugins_path"])

    def test_cli_error_has_no_partial_output(self):
        (self.state / "broken.json").write_text('{"secret":"never-print-me"')
        output, errors = io.StringIO(), io.StringIO()
        with patch("sys.argv", ["helix-servers", "--config", str(self.config), "--json"]):
            with contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
                self.assertEqual(discovery.main(), 1)
        self.assertEqual(output.getvalue(), "")
        self.assertNotIn("never-print-me", errors.getvalue())

    def test_oversized_json_rejected(self):
        (self.state / f"{self.id}.json").write_bytes(b" " * (discovery.MAX_JSON_BYTES + 1))
        with self.assertRaises(ValueError):
            discovery.inventory(self.config)

    def test_missing_registry_is_an_error(self):
        with self.assertRaises(OSError):
            discovery.inventory(self.root / "missing.json")

    def test_symlink_definition_rejected(self):
        with patch.object(Path, "is_symlink", return_value=True):
            with self.assertRaises(ValueError):
                discovery.inventory(self.config)

    def test_permission_error_explains_authorized_access(self):
        errors = io.StringIO()
        with patch("sys.argv", ["helix-servers"]), patch.object(discovery, "inventory", side_effect=PermissionError):
            with contextlib.redirect_stderr(errors):
                self.assertEqual(discovery.main(), 1)
        self.assertIn("authorized sudo", errors.getvalue())

    def test_json_output_is_versioned_and_secret_free(self):
        output = io.StringIO()
        with patch("sys.argv", ["helix-servers", "--config", str(self.config), "--json", "--id", self.id]):
            with contextlib.redirect_stdout(output):
                self.assertEqual(discovery.main(), 0)
        result = json.loads(output.getvalue())
        self.assertEqual(result["schema_version"], 1)
        self.assertEqual(len(result["servers"]), 1)
        self.assertNotIn("never-print-me", output.getvalue())


if __name__ == "__main__":
    unittest.main()
