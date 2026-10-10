#!/usr/bin/env python3
"""ves script sandbox -- a network-only Python execution environment.

Design
------
1. The user script is parsed with ast and validated against a whitelist before a
   single line runs. Filesystem, process, socket and dynamic-execution surfaces are
   rejected statically, with line numbers.
2. Only an explicit subset of builtins is exposed; `__builtins__` is replaced.
3. `__import__` is guarded: only whitelisted modules may be imported from the user
   script (stdlib internals keep working).
4. `socket.socket.connect` / `socket.create_connection` are patched so every
   outbound connection - including those made by urllib - is checked against the
   host policy. Private/loopback targets are refused.
5. Resource limits (CPU, memory, file descriptors) and a wall-clock alarm are set.

Exit codes: 0 ok, 3 rejected by policy, 4 runtime error, 5 timeout.
"""

import ast
import builtins as _builtins
import json
import os
import signal
import sys

MARK = "VES_REJECT: "


def reject(msg):
    sys.stderr.write(MARK + msg + "\n")
    sys.stderr.flush()
    sys.exit(3)


def load_config():
    path = os.environ.get("VES_SANDBOX_CONFIG", "")
    if not path or not os.path.exists(path):
        return {
            "allowed_modules": ["json", "re"],
            "allow_hosts": [],
            "deny_hosts": [],
            "max_fetch_bytes": 1000000,
            "network_enabled": True,
            "cpu_seconds": 30,
            "max_memory_mb": 512,
            "max_source_bytes": 200000,
        }
    with open(path, "r", encoding="utf-8") as fh:
        cfg = json.load(fh)
    cfg.setdefault("allowed_modules", ["json", "re"])
    cfg.setdefault("allow_hosts", [])
    cfg.setdefault("deny_hosts", [])
    cfg.setdefault("max_fetch_bytes", 1000000)
    cfg.setdefault("network_enabled", True)
    cfg.setdefault("cpu_seconds", 30)
    cfg.setdefault("max_memory_mb", 512)
    cfg.setdefault("max_source_bytes", 200000)
    return cfg


CFG = load_config()
USER_PATH = os.path.abspath(sys.argv[1])
ALLOWED_MODULES = set(CFG["allowed_modules"])

# Names that may never appear in user code, in any position.
BANNED_NAMES = {
    "exec", "eval", "compile", "__import__", "open", "input", "breakpoint",
    "globals", "locals", "vars", "getattr", "setattr", "delattr", "hasattr",
    "exit", "quit", "help", "credits", "license", "reload",
}
# Attribute names that may never be accessed, on any object.
BANNED_ATTRS = {
    "system", "popen", "spawn", "spawnl", "spawnle", "spawnlp", "spawnv", "spawnve",
    "spawnvp", "execv", "execve", "execl", "execle", "execlp", "execvp", "execvpe",
    "fork", "forkpty", "kill", "killpg", "remove", "unlink", "rmdir", "removedirs",
    "mkdir", "makedirs", "rename", "renames", "rmtree", "chmod", "chown",
    "chdir", "walk", "listdir", "scandir", "read", "readline", "readlines",
    "write", "writelines", "truncate", "dump", "putenv", "unsetenv",
    "fileno", "dup", "dup2", "connect", "bind", "listen", "accept",
    "urlopen", "send", "sendall", "recv", "environ", "startfile",
}

# Benign dunders that user code may reference (the classic __main__ guard).
ALLOWED_DUNDERS = {"__name__", "__file__", "__doc__", "__package__"}
MAX_NODES = 40000


def module_allowed(name):
    if not name:
        return False
    if name in ALLOWED_MODULES:
        return True
    for allowed in ALLOWED_MODULES:
        if allowed.startswith(name + "."):
            return True
    return False


def is_dunder(name):
    """True for sandbox-dangerous dunder names (benign ones are allowlisted)."""
    if name in ALLOWED_DUNDERS:
        return False
    return name.startswith("__") and name.endswith("__")


class Validator(ast.NodeVisitor):
    def __init__(self):
        self.errors = []
        self.nodes = 0

    def fail(self, node, msg):
        line = getattr(node, "lineno", 0)
        self.errors.append("line %s: %s" % (line, msg))

    def generic_visit(self, node):
        self.nodes += 1
        if self.nodes > MAX_NODES:
            self.fail(node, "script is too complex (more than %d syntax nodes)" % MAX_NODES)
            raise _TooBig()
        super().generic_visit(node)

    # -- imports -----------------------------------------------------------
    def visit_Import(self, node):
        for alias in node.names:
            if not module_allowed(alias.name):
                self.fail(node, "import of '%s' is not allowed; permitted modules: %s"
                          % (alias.name, ", ".join(sorted(ALLOWED_MODULES))))
        self.generic_visit(node)

    def visit_ImportFrom(self, node):
        if node.level and node.level > 0:
            self.fail(node, "relative imports are not allowed")
        elif not module_allowed(node.module or ""):
            self.fail(node, "import from '%s' is not allowed; permitted modules: %s"
                      % (node.module, ", ".join(sorted(ALLOWED_MODULES))))
        for alias in node.names:
            if alias.name == "*":
                self.fail(node, "star imports are not allowed - import explicit names")
            elif is_dunder(alias.name):
                self.fail(node, "importing '%s' is not allowed" % alias.name)
        self.generic_visit(node)

    # -- names / attributes ------------------------------------------------
    def visit_Name(self, node):
        if node.id in BANNED_NAMES:
            self.fail(node, "'%s' is not available in the network-only sandbox" % node.id)
        elif is_dunder(node.id):
            self.fail(node, "access to '%s' is not allowed" % node.id)
        self.generic_visit(node)

    def visit_Attribute(self, node):
        if is_dunder(node.attr):
            self.fail(node, "attribute '%s' is not allowed (sandbox-escape vector)" % node.attr)
        elif node.attr in BANNED_ATTRS:
            self.fail(node, "attribute '.%s' is not allowed in the network-only sandbox" % node.attr)
        self.generic_visit(node)

    def visit_ExceptHandler(self, node):
        if node.name and is_dunder(node.name):
            self.fail(node, "binding to '%s' is not allowed" % node.name)
        self.generic_visit(node)

    def visit_AnnAssign(self, node):
        if isinstance(node.target, ast.Name) and is_dunder(node.target.id):
            self.fail(node, "assignment to '%s' is not allowed" % node.target.id)
        self.generic_visit(node)

    def visit_Assign(self, node):
        for target in node.targets:
            if isinstance(target, ast.Name) and is_dunder(target.id):
                self.fail(node, "assignment to '%s' is not allowed" % target.id)
        self.generic_visit(node)

    def visit_Global(self, node):
        for name in node.names:
            if is_dunder(name):
                self.fail(node, "global '%s' is not allowed" % name)
        self.generic_visit(node)


class _TooBig(Exception):
    pass


def read_source():
    if not os.path.exists(USER_PATH):
        reject("user script not found")
    size = os.path.getsize(USER_PATH)
    if size > CFG["max_source_bytes"]:
        reject("script is too large (%d bytes, max %d)" % (size, CFG["max_source_bytes"]))
    with open(USER_PATH, "r", encoding="utf-8") as fh:
        return fh.read()


def validate(source):
    try:
        tree = ast.parse(source, filename=USER_PATH)
    except SyntaxError as exc:
        reject("syntax error on line %s: %s" % (exc.lineno, exc.msg))
    validator = Validator()
    try:
        validator.visit(tree)
    except _TooBig:
        pass
    if validator.errors:
        lines = validator.errors[:12]
        extra = "" if len(validator.errors) <= 12 else " (+%d more)" % (len(validator.errors) - 12)
        reject("; ".join(lines) + extra)
    return tree


# ---------------------------------------------------------------------------
# Host policy (shared by the prelude and the socket guard)
# ---------------------------------------------------------------------------

import ipaddress
import socket as _socket


def host_of(url):
    from urllib.parse import urlparse
    parsed = urlparse(url)
    if parsed.scheme not in ("http", "https"):
        raise ValueError("only http and https URLs may be fetched (got '%s')" % parsed.scheme)
    host = parsed.hostname
    if not host:
        raise ValueError("URL has no host: %s" % url)
    return host


PUBLIC_BLOCK_HINT = "private, loopback and link-local addresses are not reachable from the sandbox"


def enforce_host(host):
    host = (host or "").strip().strip("[]").lower()
    if not host:
        raise ValueError("empty host")
    for denied in CFG["deny_hosts"]:
        d = denied.strip().lower()
        if d and (host == d or host.endswith("." + d)):
            raise ValueError("host '%s' is on the sandbox deny list" % host)
    if CFG["allow_hosts"]:
        allowed = [h.strip().lower() for h in CFG["allow_hosts"] if h.strip()]
        if not any(host == a or host.endswith("." + a) for a in allowed):
            raise ValueError("host '%s' is not on the sandbox allow list (%s)" % (host, ", ".join(allowed)))
    if host in ("localhost",) or host.endswith(".localhost") or host.endswith(".local"):
        raise ValueError("host '%s': %s" % (host, PUBLIC_BLOCK_HINT))
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        ip = None
    if ip is not None and (ip.is_private or ip.is_loopback or ip.is_link_local or ip.is_reserved or ip.is_multicast or ip.is_unspecified):
        raise ValueError("host '%s': %s" % (host, PUBLIC_BLOCK_HINT))
    # resolve and re-check, so DNS names pointing inward are refused too
    try:
        infos = _socket.getaddrinfo(host, None)
    except Exception:
        return
    for info in infos:
        try:
            addr = ipaddress.ip_address(info[4][0])
        except ValueError:
            continue
        if addr.is_private or addr.is_loopback or addr.is_link_local or addr.is_reserved or addr.is_unspecified:
            raise ValueError("host '%s' resolves to %s: %s" % (host, addr, PUBLIC_BLOCK_HINT))


_real_connect = _socket.socket.connect
_real_connect_ex = _socket.socket.connect_ex
_real_create_connection = _socket.create_connection


def _guard_address(address):
    if isinstance(address, tuple) and address:
        enforce_host(str(address[0]))
    elif isinstance(address, str):
        enforce_host(address)


def guarded_connect(self, address):
    _guard_address(address)
    return _real_connect(self, address)


def guarded_connect_ex(self, address):
    _guard_address(address)
    return _real_connect_ex(self, address)


def guarded_create_connection(address, *args, **kwargs):
    _guard_address(address)
    return _real_create_connection(address, *args, **kwargs)


_socket.socket.connect = guarded_connect
_socket.socket.connect_ex = guarded_connect_ex
_socket.create_connection = guarded_create_connection


# ---------------------------------------------------------------------------
# Prelude available to the script
# ---------------------------------------------------------------------------

import urllib.request as _urlreq


def _decode(raw, content_type):
    charset = "utf-8"
    for part in (content_type or "").split(";"):
        part = part.strip()
        if part.lower().startswith("charset="):
            charset = part.split("=", 1)[1].strip().strip('"')
    try:
        return raw.decode(charset, errors="replace")
    except LookupError:
        return raw.decode("utf-8", errors="replace")


def fetch(url, method="GET", headers=None, data=None, timeout=20, max_bytes=None):
    """HTTP fetch with the sandbox host policy applied. Returns a dict."""
    if not CFG["network_enabled"]:
        raise RuntimeError("network access is disabled by configuration")
    host = host_of(url)
    enforce_host(host)
    limit = int(max_bytes or CFG["max_fetch_bytes"])
    body = None
    if data is not None:
        body = data.encode("utf-8") if isinstance(data, str) else data
    request = _urlreq.Request(url, data=body, headers=headers or {}, method=method.upper())
    request.add_header("User-Agent", "ves-sandbox/0.1 (+network-only sandbox)")
    with _urlreq.urlopen(request, timeout=timeout) as response:
        raw = response.read(limit + 1)
        truncated = len(raw) > limit
        if truncated:
            raw = raw[:limit]
        return {
            "status": getattr(response, "status", 200),
            "url": response.geturl(),
            "content_type": response.headers.get("Content-Type", ""),
            "headers": dict(response.headers.items()),
            "text": _decode(raw, response.headers.get("Content-Type", "")),
            "truncated": truncated,
        }


def fetch_json(url, **kwargs):
    """Fetch a URL and parse the body as JSON."""
    payload = fetch(url, **kwargs)
    return json.loads(payload["text"])


def html_to_text(html):
    """Very small HTML to text reducer (scripts/styles dropped, blocks separated)."""
    out = []
    skip = False
    tag = []
    for ch in html:
        if ch == "<":
            tag = []
            skip = True
        elif ch == ">":
            name = "".join(tag).strip().lower()
            if name.startswith("script") or name.startswith("style"):
                skip = False
                out.append(" ")
            elif name.startswith(("br", "/p", "/div", "/li", "/tr", "/h1", "/h2", "/h3", "/h4", "/td")):
                out.append("\n")
            skip = False
            tag = []
        elif skip:
            tag.append(ch)
        else:
            out.append(ch)
    text = "".join(out)
    for entity, repl in (("&amp;", "&"), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", '"'), ("&#39;", "'"), ("&nbsp;", " ")):
        text = text.replace(entity, repl)
    lines = [line.strip() for line in text.split("\n")]
    return "\n".join([line for line in lines if line])


def strip_tags(value):
    out = []
    depth = 0
    for ch in value:
        if ch == "<":
            depth += 1
        elif ch == ">":
            depth = max(0, depth - 1)
        elif depth == 0:
            out.append(ch)
    return "".join(out)


def truncate(value, limit=2000):
    text = value if isinstance(value, str) else str(value)
    return text if len(text) <= limit else text[:limit] + "...[truncated]"


def emit(obj):
    """Print a compact JSON result - the intended way to return data."""
    print(json.dumps(obj, ensure_ascii=False, indent=2, default=str))


def now():
    from datetime import datetime, timezone
    return datetime.now(timezone.utc).isoformat()


# ---------------------------------------------------------------------------
# Restricted builtins + import guard
# ---------------------------------------------------------------------------

SAFE_BUILTIN_NAMES = [
    "abs", "all", "any", "ascii", "bin", "bool", "bytearray", "bytes", "callable",
    "chr", "classmethod", "complex", "dict", "divmod", "enumerate", "filter", "float",
    "format", "frozenset", "hash", "hex", "id", "int", "isinstance", "issubclass",
    "iter", "len", "list", "map", "max", "min", "next", "object", "oct", "ord", "pow",
    "print", "property", "range", "repr", "reversed", "round", "set", "slice", "sorted",
    "staticmethod", "str", "sum", "super", "tuple", "type", "zip", "True", "False", "None",
    "ArithmeticError", "AssertionError", "AttributeError", "BaseException", "Exception",
    "FloatingPointError", "IndexError", "KeyError", "LookupError", "MemoryError",
    "NameError", "NotImplementedError", "OverflowError", "RuntimeError", "StopIteration",
    "SyntaxError", "TimeoutError", "TypeError", "UnboundLocalError", "UnicodeDecodeError",
    "UnicodeEncodeError", "UnicodeError", "ValueError", "ZeroDivisionError",
    "__build_class__",
]

safe_builtins = {}
for _name in SAFE_BUILTIN_NAMES:
    if hasattr(_builtins, _name):
        safe_builtins[_name] = getattr(_builtins, _name)
safe_builtins["__build_class__"] = _builtins.__build_class__

_real_import = _builtins.__import__


def guarded_import(name, globals=None, locals=None, fromlist=(), level=0):
    if level != 0:
        raise ImportError("relative imports are not permitted")
    try:
        caller = sys._getframe(1).f_code.co_filename
    except Exception:
        caller = ""
    if os.path.abspath(caller) == USER_PATH and not module_allowed(name):
        raise ImportError("module '%s' is not permitted in the network-only sandbox" % name)
    return _real_import(name, globals, locals, fromlist, level)


safe_builtins["__import__"] = guarded_import

NAMESPACE = {
    "__name__": "__main__",
    "__builtins__": safe_builtins,
    "fetch": fetch,
    "fetch_json": fetch_json,
    "html_to_text": html_to_text,
    "strip_tags": strip_tags,
    "truncate": truncate,
    "emit": emit,
    "now": now,
    "json": __import__("json"),
    "re": __import__("re"),
    "math": __import__("math"),
    "sys_argv": list(sys.argv[2:]),
}
# `json`/`re` modules give the script the obvious primitives without an import line.
NAMESPACE["argv"] = NAMESPACE["sys_argv"]
del NAMESPACE["sys_argv"]


def apply_limits():
    try:
        import resource
        cpu = max(1, int(CFG["cpu_seconds"]))
        try:
            resource.setrlimit(resource.RLIMIT_CPU, (cpu, cpu + 5))
        except Exception:
            pass
        mem = max(64, int(CFG["max_memory_mb"])) * 1024 * 1024
        try:
            resource.setrlimit(resource.RLIMIT_AS, (mem, mem))
        except Exception:
            pass
        try:
            resource.setrlimit(resource.RLIMIT_NOFILE, (64, 64))
        except Exception:
            pass
        try:
            resource.setrlimit(resource.RLIMIT_NPROC, (0, 0))
        except Exception:
            pass
    except Exception:
        pass


def main():
    apply_limits()
    source = read_source()
    tree = validate(source)
    code = compile(tree, USER_PATH, "exec")

    wall = max(1, int(CFG["cpu_seconds"]) + 5)

    def on_alarm(signum, frame):
        raise TimeoutError("script exceeded the %ds wall-clock limit" % wall)

    try:
        signal.signal(signal.SIGALRM, on_alarm)
        signal.alarm(wall)
    except Exception:
        pass
    try:
        exec(code, NAMESPACE)
    except SystemExit as exc:
        code_val = exc.code if isinstance(exc.code, int) else 0
        sys.exit(code_val or 0)
    except TimeoutError as exc:
        sys.stderr.write("VES_TIMEOUT: %s\n" % exc)
        sys.exit(5)
    except BaseException as exc:  # noqa: BLE001 - report everything to the caller
        import traceback as _tb
        text = "".join(_tb.format_exception_only(type(exc), exc)).strip()
        sys.stderr.write("VES_ERROR: %s\n" % text)
        sys.exit(4)
    finally:
        try:
            signal.alarm(0)
        except Exception:
            pass


if __name__ == "__main__":
    main()
