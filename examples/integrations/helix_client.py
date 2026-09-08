"""Small stdlib client for trusted Helix integrations; no automatic mutation retries."""

import http.cookiejar
import ipaddress
import json
import math
import time
import urllib.error
import urllib.parse
import urllib.request


class HelixError(Exception):
    def __init__(self, status, code, request_id=None):
        # Remote response text can contain console/file data. Do not log it here.
        super().__init__(f"Helix request failed ({status or 'transport'}): {code}")
        self.status = status
        self.code = code
        self.request_id = request_id


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise HelixError(code, "redirect_refused")


class HelixClient:
    """One origin, one in-memory session. Do not share an instance across threads."""

    MAX_RESPONSE_BYTES = 4 * 1024 * 1024

    def __init__(self, origin, *, allow_private_http=False, timeout=30):
        parts = urllib.parse.urlsplit(origin)
        if (parts.scheme not in ("http", "https") or not parts.hostname
                or parts.username is not None or parts.password is not None
                or parts.path not in ("", "/") or parts.query or parts.fragment
                or any(c.isspace() for c in origin) or "\\" in origin):
            raise ValueError("Use only a Helix origin, without credentials or a path")
        if parts.port == 0:
            raise ValueError("Invalid origin port")
        if parts.scheme == "http":
            try:
                address = ipaddress.ip_address(parts.hostname)
                loopback = address.is_loopback
                private = address.is_private and not address.is_unspecified and not address.is_multicast
            except ValueError:
                loopback = parts.hostname.lower() == "localhost"
                private = False
            if not loopback and not (allow_private_http and private):
                raise ValueError("HTTP needs an explicit private IP and allow_private_http=True; prefer a trusted TLS endpoint or SSH tunnel")
        if not math.isfinite(timeout) or timeout <= 0:
            raise ValueError("timeout must be finite and positive")
        self.origin = urllib.parse.urlunsplit((parts.scheme, parts.netloc, "", "", ""))
        self.timeout = timeout
        self._csrf = None
        self._cookies = http.cookiejar.CookieJar()
        self._opener = urllib.request.build_opener(
            urllib.request.ProxyHandler({}), _NoRedirect(),
            urllib.request.HTTPCookieProcessor(self._cookies),
        )

    def request(self, method, path, body=None, *, _timeout=None):
        parts = urllib.parse.urlsplit(path)
        decoded = urllib.parse.unquote(parts.path)
        if (parts.scheme or parts.netloc or parts.fragment
                or not decoded.startswith("/api/v1/")
                or "\\" in decoded or any(c.isspace() for c in path)
                or any(segment in (".", "..") for segment in decoded.split("/"))):
            raise ValueError("Requests must stay within /api/v1/ on this Helix origin")
        method = method.upper()
        if method not in ("GET", "POST", "PUT", "PATCH", "DELETE"):
            raise ValueError("Unsupported HTTP method")
        if method == "GET" and body is not None:
            raise ValueError("GET requests cannot have a body")
        headers = {"Accept": "application/json"}
        if self._csrf:
            headers["X-Helix-CSRF"] = self._csrf
        if method != "GET":
            headers["Origin"] = self.origin
            headers["Content-Type"] = "application/json"
        payload = None if body is None else json.dumps(body, allow_nan=False).encode("utf-8")
        request = urllib.request.Request(self.origin + path, data=payload, headers=headers, method=method)
        try:
            with self._opener.open(request, timeout=self.timeout if _timeout is None else _timeout) as response:
                return self._read_json(response)
        except urllib.error.HTTPError as error:
            try:
                problem = self._read_json(error)
            except HelixError:
                problem = {}
            code = problem.get("code") if isinstance(problem, dict) else None
            if not isinstance(code, str) or len(code) > 96 or not all(c.isascii() and (c.isalnum() or c == "_") for c in code):
                code = "http_error"
            if error.code == 401:
                self.close()
            request_id = error.headers.get("X-Request-ID")
            error.close()
            raise HelixError(error.code, code, request_id) from None
        except (urllib.error.URLError, TimeoutError, OSError):
            raise HelixError(None, "transport_error_outcome_unknown" if method != "GET" else "transport_error") from None

    def _read_json(self, response):
        if response.status == 204:
            return None
        if response.headers.get_content_type() != "application/json":
            raise HelixError(response.status, "unexpected_content_type")
        raw = response.read(self.MAX_RESPONSE_BYTES + 1)
        if len(raw) > self.MAX_RESPONSE_BYTES:
            raise HelixError(response.status, "response_too_large")
        try:
            return json.loads(raw)
        except (ValueError, UnicodeError):
            raise HelixError(response.status, "invalid_json") from None

    def login(self, login_name, password):
        self.close()
        try:
            result = self.request("POST", "/api/v1/auth/login", {"loginName": login_name, "password": password})
            token = result.get("csrfToken") if isinstance(result, dict) else None
            if not isinstance(token, str) or len(token) != 43 or not all(c.isascii() and (c.isalnum() or c in "-_") for c in token):
                raise HelixError(200, "invalid_login_response")
            self._csrf = token
            return result["user"]
        except Exception:
            self.close()
            raise

    def logout(self):
        try:
            self.request("POST", "/api/v1/auth/logout")
        finally:
            self.close()

    def close(self):
        """Forget local credentials. Use logout() to revoke them on Helix too."""
        self._csrf = None
        self._cookies.clear()

    def discovery(self):
        return self.request("GET", "/api/v1/discovery")

    def servers(self):
        result = self.request("GET", "/api/v1/servers")
        if not isinstance(result, list):
            raise HelixError(200, "invalid_inventory")
        return result

    def server(self, server_id):
        return self.request("GET", "/api/v1/servers/" + urllib.parse.quote(server_id, safe=""))

    def server_action(self, server_id, action):
        if action not in ("start", "stop", "restart", "kill", "update", "backup"):
            raise ValueError("Unknown server action")
        return self.request("POST", "/api/v1/servers/" + urllib.parse.quote(server_id, safe="") + "/actions", {"action": action})

    def wait_for_job(self, job_id, *, timeout=300, interval=2):
        if not all(math.isfinite(value) and value > 0 for value in (timeout, interval)):
            raise ValueError("Job timeout and interval must be finite and positive")
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise HelixError(None, "job_deadline_reached_not_cancelled")
            job = self.request("GET", "/api/v1/jobs/" + urllib.parse.quote(job_id, safe=""), _timeout=min(self.timeout, remaining))
            if not isinstance(job, dict) or job.get("id") != job_id:
                raise HelixError(200, "invalid_job_response")
            status = job.get("status")
            if status == "complete":
                return job
            if status == "failed":
                raise HelixError(200, "job_failed")
            if status not in ("queued", "running"):
                raise HelixError(200, "unknown_job_status")
            time.sleep(min(interval, max(0, deadline - time.monotonic())))
