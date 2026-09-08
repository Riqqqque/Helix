"""Small stdlib client for trusted Helix integrations; no automatic mutation retries."""

import http.cookiejar
import base64
import hashlib
import ipaddress
import json
import math
import os
import stat as file_stat
import tempfile
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
        api_token = getattr(self, "_api_token", None)
        if api_token is not None:
            if path not in ("/api/v1/automation/server", "/api/v1/automation/jobs"):
                raise ValueError("Server tokens can only call the automation API")
            headers["Authorization"] = "Bearer " + api_token
        if self._csrf:
            headers["X-Helix-CSRF"] = self._csrf
        if method != "GET":
            if api_token is None:
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
            self.request("POST", "/api/v1/auth/logout", {})
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

    def server_capabilities(self, server_id):
        return self.request("GET", "/api/v1/servers/" + urllib.parse.quote(server_id, safe="") + "/capabilities")

    def server_files(self, server_id, action, **fields):
        """All paths are relative to this exact server; mutations require it stopped."""
        return self.request("POST", "/api/v1/servers/" + urllib.parse.quote(server_id, safe="") + "/files", {"action": action, **fields})

    def iter_server_directory(self, server_id, path="", *, page_size=100):
        cursor = None
        seen = set()
        while True:
            page = self.server_files(server_id, "list", path=path, cursor=cursor, limit=page_size)
            if not isinstance(page, dict) or not isinstance(page.get("entries"), list):
                raise HelixError(200, "invalid_file_listing")
            yield from page["entries"]
            cursor = page.get("next_cursor")
            if cursor is None:
                return
            if not isinstance(cursor, str) or cursor in seen:
                raise HelixError(200, "invalid_directory_cursor")
            seen.add(cursor)

    def _download_chunks(self, server_id, path, stat):
        size, revision = stat.get("size"), stat.get("revision")
        if type(size) is not int or size < 0 or not isinstance(revision, str) or stat.get("kind") != "file":
            raise HelixError(200, "invalid_file_metadata")
        offset = 0
        while offset < size:
            chunk = self.server_files(server_id, "download", path=path, offset=offset,
                                      length=min(1024 * 1024, size - offset), expected_revision=revision)
            try:
                data = base64.b64decode(chunk["data_base64"], validate=True)
            except (KeyError, ValueError, TypeError):
                raise HelixError(200, "invalid_download_chunk") from None
            if (not data or len(data) > 1024 * 1024 or chunk.get("offset") != offset
                    or chunk.get("next_offset") != offset + len(data) or offset + len(data) > size
                    or chunk.get("size") != size or chunk.get("revision") != revision
                    or chunk.get("sha256") != hashlib.sha256(data).hexdigest()):
                raise HelixError(200, "invalid_download_chunk")
            offset += len(data)
            yield data
        # Also catches replacement of an empty file or changes after the last chunk.
        if self.server_files(server_id, "stat", path=path).get("revision") != revision:
            raise HelixError(200, "file_revision_conflict")

    def download_file(self, server_id, path, destination, *, max_bytes=8 * 1024**3):
        """Write a new local file atomically; never overwrite an existing destination."""
        stat = self.server_files(server_id, "stat", path=path)
        if type(stat.get("size")) is not int or not 0 <= stat["size"] <= max_bytes:
            raise HelixError(200, "download_size_limit")
        destination = os.path.abspath(destination)
        descriptor, staged = tempfile.mkstemp(prefix=".helix-download-", dir=os.path.dirname(destination))
        try:
            with os.fdopen(descriptor, "wb") as output:
                for data in self._download_chunks(server_id, path, stat):
                    output.write(data)
                output.flush()
                os.fsync(output.fileno())
            os.link(staged, destination)  # Atomic create-new, including on Windows/NTFS.
        finally:
            os.unlink(staged)
        return destination

    def upload_file(self, server_id, source, path, *, expected_revision=None):
        """Hash and stream a local file; preserve the old destination on failed checks."""
        with open(source, "rb") as stream:
            before = os.fstat(stream.fileno())
            if not file_stat.S_ISREG(before.st_mode) or not 0 <= before.st_size <= 8 * 1024**3:
                raise ValueError("Upload a regular file of at most 8 GiB")
            digest = hashlib.sha256()
            remaining = before.st_size
            while remaining:
                data = stream.read(min(1024 * 1024, remaining))
                if not data:
                    raise ValueError("Source changed while hashing")
                digest.update(data)
                remaining -= len(data)
            after = os.fstat(stream.fileno())
            if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
                raise ValueError("Source changed while hashing")
            stream.seek(0)
            return self._upload_chunks(server_id, path, before.st_size, digest.hexdigest(),
                                       iter(lambda: stream.read(1024 * 1024), b""), expected_revision)

    def transfer_file(self, source_server, source_path, destination_server, destination_path, *, expected_revision=None):
        """Copy through this client in bounded chunks, without granting host-path access."""
        stat = self.server_files(source_server, "stat", path=source_path)
        if type(stat.get("size")) is not int or not 0 <= stat["size"] <= 8 * 1024**3:
            raise HelixError(200, "transfer_size_limit")
        digest = hashlib.sha256()
        for data in self._download_chunks(source_server, source_path, stat):
            digest.update(data)
        return self._upload_chunks(destination_server, destination_path, stat["size"], digest.hexdigest(),
                                   self._download_chunks(source_server, source_path, stat), expected_revision)

    def _upload_chunks(self, server_id, path, size, digest, chunks, expected_revision):
        started = self.server_files(server_id, "upload_begin", path=path, size=size,
                                    sha256=digest, expected_revision=expected_revision)
        upload_id = started.get("upload_id")
        if not isinstance(upload_id, str) or not upload_id:
            raise HelixError(200, "invalid_upload_response")
        offset = 0
        try:
            for data in chunks:
                progress = self.server_files(server_id, "upload_chunk", upload_id=upload_id, offset=offset,
                                             data_base64=base64.b64encode(data).decode("ascii"))
                offset += len(data)
                if progress.get("bytes_written") != offset:
                    raise HelixError(200, "invalid_upload_progress")
            return self.server_files(server_id, "upload_finish", upload_id=upload_id)
        except Exception:
            try:
                self.server_files(server_id, "upload_abort", upload_id=upload_id)
            except Exception:
                pass  # A completed upload may already be gone; never replay finish.
            raise

    def wait_for_job(self, job_id, *, timeout=300, interval=2):
        if not all(math.isfinite(value) and value > 0 for value in (timeout, interval)):
            raise ValueError("Job timeout and interval must be finite and positive")
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise HelixError(None, "job_deadline_reached_not_cancelled")
            job = self.job_status(job_id, timeout=min(self.timeout, remaining))
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

    def job_status(self, job_id, *, timeout=None):
        return self.request("GET", "/api/v1/jobs/" + urllib.parse.quote(job_id, safe=""), _timeout=timeout)


class HelixTokenClient(HelixClient):
    """Server-scoped automation. Use HTTPS or an SSH-forwarded loopback origin."""

    def __init__(self, origin, token, *, timeout=30):
        if not isinstance(token, str) or len(token) != 43 or not all(c.isascii() and (c.isalnum() or c in "-_") for c in token):
            raise ValueError("Invalid server token encoding")
        super().__init__(origin, timeout=timeout)
        self._api_token = token

    def close(self):
        super().close()
        self._api_token = None

    def execute(self, operation, **fields):
        if self._api_token is None:
            raise ValueError("This token client is closed")
        return self.request("POST", "/api/v1/automation/server", {"operation": operation, **fields})

    def server_capabilities(self, server_id):
        return self.execute("server_capabilities", instance_id=server_id)

    def server_files(self, server_id, action, **fields):
        return self.execute("server_files", instance_id=server_id, request={"action": action, **fields})

    def server_action(self, server_id, action):
        return self.execute("server_action", instance_id=server_id, action=action)

    def job_status(self, job_id, *, timeout=None):
        return self.request("POST", "/api/v1/automation/server", {"operation": "job_status", "job_id": job_id}, _timeout=timeout)
