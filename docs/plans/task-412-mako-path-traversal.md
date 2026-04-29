# Task 412: Mako path traversal via double-slash URI prefix

## Vulnerability

`TemplateLookup.get_template()` accepted URIs like `//../../../../etc/passwd`, bypassing the
path traversal check in `Template.__init__`.

Root cause — inconsistent slash stripping:

| Code path | Stripping | Effect on `//../../etc/passwd` |
|---|---|---|
| `get_template()` | `re.sub(r"^\/+", "")` | → `../../etc/passwd` (file found on disk) |
| `Template.__init__` | `u_norm[1:]` | → `/../../etc/passwd` → `normpath` → `/etc/passwd` |

`/etc/passwd` does not start with `..`, so the check was bypassed.

## Fix

`Template.__init__` now uses `lstrip("/")` instead of `[1:]`, so any number of leading
slashes are stripped before the `..` check.

This was released in **Mako 1.3.12** (upstream commit in `mako/template.py`):

```python
# before (vulnerable ≤ 1.3.10)
if u_norm.startswith("/"):
    u_norm = u_norm[1:]

# after (fixed ≥ 1.3.12)
u_norm = self.uri.replace("\\", "/").lstrip("/")
```

## Verification

Regression tests live in `scripts/test_mako_path_traversal.py`. They fail on 1.3.10 and
pass on 1.3.12:

```bash
# Fails (vulnerable):
uv run --with "mako==1.3.10" scripts/test_mako_path_traversal.py

# Passes (fixed):
uv run scripts/test_mako_path_traversal.py   # resolves to 1.3.12+
```

## Action required in affected projects

Projects using Mako (e.g. `annotell/airflow-dags`) should pin `mako>=1.3.12` in their
dependency files and regenerate lock files.
