"""Join static system-command sites with the runner dispatch probe.

Inputs: the `external_script_structure` report (system/site lines), the
`command_inventory` probe output and an engine profile table. Writes a new TSV
(refuses to overwrite) and prints the class counts. Standard library only.
"""
import argparse
import collections
import re


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("sites")
    parser.add_argument("dispatch")
    parser.add_argument("profile")
    parser.add_argument("output")
    args = parser.parse_args()

    sites = collections.defaultdict(collections.Counter)
    with open(args.sites, encoding="utf-8", errors="replace") as f:
        for line in f:
            p = line.rstrip("\n").split("\t")
            if p[0] == "system":
                sites[int(p[2], 16)][p[1]] += int(p[3])
    if not sites:
        raise SystemExit("no system lines in the sites report")

    status = {}
    with open(args.profile, encoding="utf-8") as f:
        for line in f:
            m = re.match(r"0x([0-9a-f]{4}) (\w+)", line)
            if m:
                status[int(m.group(1), 16)] = m.group(2)

    probes = collections.defaultdict(list)
    with open(args.dispatch, encoding="utf-8") as f:
        for line in f:
            p = line.rstrip("\n").split("\t")
            probes[int(p[0], 16)].append((p[2], p[3] if len(p) > 3 else ""))

    rows = []
    for command in sorted(sites):
        results = probes.get(command)
        if not results:
            raise SystemExit(f"0x{command:04x} missing from the dispatch probe")
        kinds = {kind for kind, _ in results}
        if kinds & {"handled", "waits"}:
            cls = "handled"
        elif "error" in kinds:
            cls = "error"
        elif "panic" in kinds:
            cls = "panic"
        else:
            cls = "unhandled"
        messages = sorted({m for kind, m in results if kind == "error"})
        rows.append((command, status.get(command, "absent"), cls, sites[command], messages))

    with open(args.output, "x", encoding="utf-8", newline="\n") as f:
        f.write("command\tstatus\tdispatch\tsites\tscripts\tby_script\terrors\n")
        for command, st, cls, per_script, messages in rows:
            by_script = ",".join(
                f"{name}:{n}" for name, n in sorted(per_script.items(), key=lambda kv: (-kv[1], kv[0]))
            )
            f.write(
                f"0x{command:04x}\t{st}\t{cls}\t{sum(per_script.values())}\t"
                f"{len(per_script)}\t{by_script}\t{' | '.join(messages)}\n"
            )
    print(collections.Counter((st, cls) for _, st, cls, _, _ in rows))


if __name__ == "__main__":
    main()
