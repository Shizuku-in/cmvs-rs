"""Maps CMVS release versions into PS Vita SFO APP_VER (XX.YY) monotonically.

Vita's SFO APP_VER field holds four ASCII digits formatted as ``XX.YY``
(``00.00``..``99.99``, 10,000 distinct values) and compares versions
lexicographically when deciding whether a VPK upgrades an installed title.
Plain ``MAJOR.MINOR.PATCH`` versions in ``(0..=9).(0..=49).(0..=19)`` map
bijectively onto ``00.00``..``99.99`` via ``major * 1000 + minor * 20 + patch``.
Any version outside that domain (including pre-release suffixes) fails fast
unless ``CMVS_APP_VER=XX.YY`` is supplied explicitly.
"""
import os
import re
import sys

APP_VER_RE = re.compile(r"[0-9]{2}\.[0-9]{2}")
SEMVER_RE = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")

MAX_MAJOR = 9
MAX_MINOR = 49
MAX_PATCH = 19
MINOR_STRIDE = MAX_PATCH + 1  # 20
MAJOR_STRIDE = (MAX_MINOR + 1) * MINOR_STRIDE  # 1000


def resolve_app_ver(version="0.1.0", app_ver=None):
    """Return the ``XX.YY`` SFO ``APP_VER`` string or raise ``ValueError``."""
    if app_ver:
        if not isinstance(app_ver, str) or not APP_VER_RE.fullmatch(app_ver):
            raise ValueError(
                f"invalid CMVS_APP_VER={app_ver!r}: expected XX.YY (00.00..99.99)"
            )
        return app_ver

    if not isinstance(version, str):
        raise ValueError(
            f"invalid CMVS_VERSION={version!r}: expected strict MAJOR.MINOR.PATCH; "
            "set CMVS_APP_VER=XX.YY explicitly"
        )

    match = SEMVER_RE.fullmatch(version)
    if not match:
        raise ValueError(
            f"cannot map CMVS_VERSION={version!r} to PS Vita APP_VER: "
            "expected strict MAJOR.MINOR.PATCH; set CMVS_APP_VER=XX.YY explicitly"
        )

    major, minor, patch = (int(part) for part in match.groups())
    if major > MAX_MAJOR or minor > MAX_MINOR or patch > MAX_PATCH:
        raise ValueError(
            f"CMVS_VERSION={version!r} exceeds representable PS Vita APP_VER range "
            f"(0..={MAX_MAJOR}).(0..={MAX_MINOR}).(0..={MAX_PATCH}); "
            "set CMVS_APP_VER=XX.YY explicitly"
        )

    value = major * MAJOR_STRIDE + minor * MINOR_STRIDE + patch
    return f"{value // 100:02d}.{value % 100:02d}"


def app_ver_from_env(env=None):
    """Resolve ``APP_VER`` from ``CMVS_APP_VER`` / ``CMVS_VERSION`` in ``env``."""
    source = os.environ if env is None else env
    return resolve_app_ver(
        version=source.get("CMVS_VERSION", "0.1.0"),
        app_ver=source.get("CMVS_APP_VER"),
    )


def main(env=None):
    try:
        print(app_ver_from_env(env))
    except ValueError as exc:
        print(f"app_ver.py: error: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
