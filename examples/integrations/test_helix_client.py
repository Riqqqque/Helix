import http.server
import json
from pathlib import Path
import threading
import unittest
from unittest.mock import patch

from helix_client import HelixClient, HelixError


class ContractTests(unittest.TestCase):
    def test_contract_references_and_path_parameters_resolve(self):
        document = json.loads((Path(__file__).resolve().parents[2] / "docs/openapi.json").read_text(encoding="utf-8"))

        def resolve(ref):
            self.assertTrue(ref.startswith("#/"), "contract must not fetch remote references")
            value = document
            for key in ref[2:].split("/"):
                value = value[key.replace("~1", "/").replace("~0", "~")]
            return value

        def walk(value):
            if isinstance(value, dict):
                if "$ref" in value:
                    resolve(value["$ref"])
                for child in value.values():
                    walk(child)
            elif isinstance(value, list):
                for child in value:
                    walk(child)

        walk(document)
        self.assertEqual(document["security"], [{"SessionCookie": [], "CsrfProof": []}])
        ids = set()
        for path, item in document["paths"].items():
            for method, operation in item.items():
                if method == "parameters":
                    continue
                self.assertNotIn(operation["operationId"], ids)
                ids.add(operation["operationId"])
                parameters = item.get("parameters", []) + operation.get("parameters", [])
                parameters = [resolve(p["$ref"]) if "$ref" in p else p for p in parameters]
                declared = {p["name"] for p in parameters if p["in"] == "path" and p["required"]}
                expected = {segment[1:-1] for segment in path.split("/") if segment.startswith("{")}
                self.assertEqual(declared, expected, path)
                self.assertIn("default", operation["responses"])
        inventory = document["paths"]["/servers"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]
        self.assertEqual(inventory["type"], "array")
        action = document["paths"]["/servers/{instance_id}/actions"]["post"]
        self.assertIn("job_id", action["responses"]["200"]["content"]["application/json"]["schema"]["properties"])
        logout = document["paths"]["/auth/logout"]["post"]
        self.assertTrue(logout["requestBody"]["required"])


class Handler(http.server.BaseHTTPRequestHandler):
    requests = []

    def log_message(self, *args):
        pass

    def do_GET(self):
        self.respond()

    def do_POST(self):
        self.respond()

    def respond(self):
        type(self).requests.append((self.command, self.path, dict(self.headers)))
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        if self.path.endswith("/auth/login"):
            self.send_response(200)
            self.send_header("Set-Cookie", "helix_session=fixture; HttpOnly; Path=/")
            value = {"csrfToken": "A" * 43, "user": {"id": "fixture"}}
        elif self.path.endswith("/auth/logout"):
            if body != b"{}":
                self.send_response(400)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(b'{"code":"invalid_json"}')
                return
            self.send_response(204)
            self.end_headers()
            return
        elif self.path.endswith("/redirect"):
            self.send_response(302)
            self.send_header("Location", "/api/v1/should-not-be-called")
            self.end_headers()
            return
        elif self.path.endswith("/failure"):
            self.send_response(503)
            self.send_header("X-Request-ID", "test-request")
            value = {"code": "host_broker_unavailable", "message": "private diagnostic"}
        elif self.path.endswith("/unauthorized"):
            self.send_response(401)
            value = {"code": "authentication_required"}
        elif self.path.endswith("/html"):
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.end_headers()
            self.wfile.write(b"not an API response")
            return
        else:
            self.send_response(200)
            value = [] if self.path.endswith("/servers") else {"ok": True}
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(value).encode())


class ClientTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join(timeout=5)

    def setUp(self):
        Handler.requests.clear()
        self.client = HelixClient(f"http://127.0.0.1:{self.server.server_port}")

    def test_session_proofs_origin_and_logout(self):
        self.client.login("fixture", "fixture")
        self.assertEqual(self.client.servers(), [])
        headers = Handler.requests[-1][2]
        self.assertEqual(headers["Cookie"], "helix_session=fixture")
        self.assertEqual(headers["X-Helix-Csrf"], "A" * 43)
        self.client.server_action("selected-id", "backup")
        self.assertEqual(Handler.requests[-1][2]["Origin"], self.client.origin)
        self.client.logout()
        self.assertIsNone(self.client._csrf)
        self.assertEqual(list(self.client._cookies), [])

    def test_rejects_foreign_paths_and_ambiguous_origins(self):
        for path in ["https://example.com/api/v1/servers", "//example.com/api/v1/servers", "/api/v1/../secret", "/api/v1/%2e%2e/secret", "/host", "/api/v1/x#fragment"]:
            with self.subTest(path=path), self.assertRaises(ValueError):
                self.client.request("GET", path)
        for origin in ["https://user:password@example.com", "https://example.com/path", "http://example.com", "http://127.0.0.1:0", "https://example.com?token=x"]:
            with self.subTest(origin=origin), self.assertRaises(ValueError):
                HelixClient(origin)
        self.assertEqual(Handler.requests, [])

    def test_private_http_requires_explicit_opt_in(self):
        with self.assertRaises(ValueError):
            HelixClient("http://192.168.1.20:3100")
        HelixClient("http://192.168.1.20:3100", allow_private_http=True)

    def test_redirect_never_forwards_session(self):
        self.client.login("fixture", "fixture")
        with self.assertRaises(HelixError) as caught:
            self.client.request("GET", "/api/v1/redirect")
        self.assertEqual(caught.exception.code, "redirect_refused")
        self.assertEqual(len(Handler.requests), 2)

    def test_mutations_are_not_retried_and_errors_do_not_echo_remote_text(self):
        with self.assertRaises(HelixError) as caught:
            self.client.request("POST", "/api/v1/failure", {})
        self.assertEqual(caught.exception.status, 503)
        self.assertEqual(caught.exception.request_id, "test-request")
        self.assertNotIn("private diagnostic", str(caught.exception))
        self.assertEqual(len(Handler.requests), 1)

    def test_expired_auth_clears_both_proofs(self):
        self.client.login("fixture", "fixture")
        with self.assertRaises(HelixError):
            self.client.request("GET", "/api/v1/unauthorized")
        self.assertIsNone(self.client._csrf)
        self.assertEqual(list(self.client._cookies), [])

    def test_non_json_and_oversized_responses_fail_closed(self):
        with self.assertRaises(HelixError) as caught:
            self.client.request("GET", "/api/v1/html")
        self.assertEqual(caught.exception.code, "unexpected_content_type")
        self.client.MAX_RESPONSE_BYTES = 1
        with self.assertRaises(HelixError) as caught:
            self.client.servers()
        self.assertEqual(caught.exception.code, "response_too_large")

    def test_job_completion_is_not_http_success(self):
        for status in ["failed", "unknown"]:
            with patch.object(self.client, "request", return_value={"id": "job", "status": status}), self.assertRaises(HelixError):
                self.client.wait_for_job("job")
        with patch.object(self.client, "request", return_value={"id": "other", "status": "complete"}), self.assertRaises(HelixError):
            self.client.wait_for_job("job")
        with patch.object(self.client, "request", side_effect=[{"id": "job", "status": "running"}, {"id": "job", "status": "complete"}]) as request, patch("helix_client.time.sleep"):
            self.assertEqual(self.client.wait_for_job("job")["status"], "complete")
            self.assertEqual(request.call_count, 2)
            self.assertTrue(all(call.args[0] == "GET" for call in request.call_args_list))

    def test_job_deadline_does_not_cancel_or_repeat_mutation(self):
        with patch("helix_client.time.monotonic", side_effect=[0, 2]), patch.object(self.client, "request") as request, self.assertRaises(HelixError) as caught:
            self.client.wait_for_job("job", timeout=1)
        self.assertEqual(caught.exception.code, "job_deadline_reached_not_cancelled")
        request.assert_not_called()


if __name__ == "__main__":
    unittest.main()
