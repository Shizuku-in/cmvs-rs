"""Combine cmvs-probe summaries into a game-by-check matrix (Python standard library only).

python -I tools/probe_matrix.py OUT_DIR... [--markdown]
Each OUT_DIR holds a summary.json written by cmvs-probe. Rows are games, labelled by the
game record, else the profile, else the root directory name; columns are check IDs in the
order first seen. Cells show the result and, for passes, the level reached. Nothing is
written; the matrix goes to standard output as TSV, or as a Markdown table.
"""
import argparse
import json
from pathlib import Path

SHORT = {"pass": "pass", "fail": "FAIL", "blocked": "BLOCKED", "skipped": "skip",
         "inconclusive": "inconclusive"}


def label(summary, directory):
    target = summary.get("target", {})
    for key in ("record", "profile"):
        if target.get(key):
            return target[key]
    root = target.get("root") or str(directory)
    return Path(root.replace("\\", "/")).name or str(directory)


def cell(check):
    result = check.get("result", "?")
    text = SHORT.get(result, result)
    if result == "pass" and check.get("level"):
        text += " " + check["level"]
    if result == "blocked" and check.get("blocked"):
        text += " " + check["blocked"].get("command", "")
    return text


def matrix(directories):
    columns, rows = [], []
    for directory in directories:
        summary = json.loads((Path(directory) / "summary.json").read_text(encoding="utf-8"))
        cells = {}
        for check in summary.get("checks", []):
            if check["id"] not in columns:
                columns.append(check["id"])
            cells[check["id"]] = cell(check)
        options = summary.get("options", {})
        policy = "+".join(name for name in ("allow_presumed", "allow_unreviewed") if options.get(name))
        rows.append((label(summary, directory), policy or "default", cells))
    return columns, rows


def render(columns, rows, markdown):
    header = ["game", "policy"] + columns
    lines = []
    if markdown:
        lines.append("| " + " | ".join(header) + " |")
        lines.append("|" + "---|" * len(header))
    else:
        lines.append("\t".join(header))
    for game, policy, cells in rows:
        values = [game, policy] + [cells.get(c, "") for c in columns]
        lines.append("| " + " | ".join(values) + " |" if markdown else "\t".join(values))
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("directories", nargs="+")
    parser.add_argument("--markdown", action="store_true")
    args = parser.parse_args()
    print(render(*matrix(args.directories), args.markdown))


if __name__ == "__main__":
    main()
