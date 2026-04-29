#!/usr/bin/env python3
# /// script
# requires-python = ">=3.9"
# dependencies = ["mako", "pytest"]
# ///
"""
Regression tests for Mako path traversal via double-slash URI prefix.

CVE: Path traversal in TemplateLookup.get_template() when URI starts with //
Fix: Template.__init__ must strip ALL leading slashes (lstrip("/")) not just one.
"""

import os
import tempfile
import pytest
from mako.template import Template
from mako.lookup import TemplateLookup
from mako.exceptions import TemplateLookupException


def test_template_init_double_slash_uri_raises():
    """//../../etc/passwd must not bypass the traversal check in Template.__init__."""
    with pytest.raises(TemplateLookupException):
        Template(uri="//../../etc/passwd", text="hello")


def test_template_init_many_slashes_raises():
    """Any number of leading slashes before .. must be caught."""
    with pytest.raises(TemplateLookupException):
        Template(uri="///../../etc/passwd", text="hello")


def test_template_lookup_double_slash_raises():
    """TemplateLookup.get_template() with a //.. URI must not read arbitrary files."""
    with tempfile.TemporaryDirectory() as tmpdir:
        # Create a harmless file inside the lookup directory
        safe_file = os.path.join(tmpdir, "safe.html")
        with open(safe_file, "w") as f:
            f.write("safe content")

        # Create a file OUTSIDE the lookup directory to attempt to reach
        secret_file = os.path.join(tempfile.gettempdir(), "secret_412.txt")
        with open(secret_file, "w") as f:
            f.write("secret")

        try:
            lookup = TemplateLookup(directories=[tmpdir])
            # Construct a URI that traverses out of tmpdir using double-slash
            # e.g. if tmpdir is /tmp/abc, this tries //secret_412.txt via /../
            with pytest.raises(TemplateLookupException):
                lookup.get_template("///../secret_412.txt")
        finally:
            if os.path.exists(secret_file):
                os.unlink(secret_file)


def test_normal_single_slash_uri_works():
    """A well-formed /path/template.html URI must still be accepted."""
    t = Template(uri="/templates/index.html", text="hello world")
    assert t.uri == "/templates/index.html"


def test_normal_no_slash_uri_works():
    """A URI without a leading slash must work fine."""
    t = Template(uri="index.html", text="hello world")
    assert t.uri == "index.html"


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
