"""Read-only IDAPython command graph export; real exports stay outside Git.

In IDA: export_commands(profile_path, output_path). In batch mode:
idat -A -S"ida_export_commands.py PROFILE OUTPUT" LOCAL_COPY.i64
No renaming, patching, analysis creation, or database saves are performed here.
"""
import hashlib
import json
import struct
from pathlib import Path

SCHEMA = "cmvs-command-graph-v1"
POLICY = "raw-operands-closed-cfg-readonly-data-v1"


def export_commands(profile_path, output_path):
    import ida_auto
    import ida_bytes
    import ida_funcs
    import ida_ida
    import ida_nalt
    import ida_segment
    import ida_ua
    import ida_idp
    import ida_fixup
    import idautils
    import idc

    ida_auto.auto_wait()
    arch = "x64" if ida_ida.inf_is_64bit() else "x86"
    image_base = ida_nalt.get_imagebase()
    source_hash = ida_nalt.retrieve_input_file_sha256().hex()
    output = Path(output_path).resolve()
    source = Path(ida_nalt.get_input_file_path()).resolve()
    if (output.suffix.lower() != ".json" or source.parent in output.parents
            or output == Path(profile_path).resolve()):
        raise ValueError("export must be a JSON file outside the original game directory and profile input")
    if hashlib.sha256(source.read_bytes()).hexdigest() != source_hash:
        raise ValueError("original executable no longer matches the database input hash")
    maximum = max(ida_segment.getseg(ea).end_ea for ea in idautils.Segments())
    if ida_bytes.visit_patched_bytes(image_base, maximum, lambda ea, pos, old, new: int(old != new)):
        raise ValueError("patched IDA bytes cannot supply inheritance evidence")
    lines = [line.split() for line in Path(profile_path).read_text(encoding="utf-8-sig").splitlines()
             if line.strip() and not line.lstrip().startswith("#")]
    profile_id = next(w[1] for w in lines if w[0] == "profile")
    binaries = [w for w in lines if w[0] == "binary"]
    binary_index = next(i for i, w in enumerate(binaries) if w[1] == arch)
    fields = dict(w.split("=", 1) for w in binaries[binary_index][2:])
    if fields["sha256"] != source_hash or int(fields["image-base"], 0) != image_base:
        raise ValueError("IDA input hash/image base does not match profile binary")
    rows = [w for w in lines if w[0].startswith("0x")]

    # Independently find the unique switch, then check every case against the profile.
    prefixes = ([bytes.fromhex("0fb7c2488bd9")] if arch == "x64" else
                [bytes.fromhex(s) for s in ("0fb7442404", "558bec0fb7450853", "558bec0fb74508568bf1")])
    candidates = []
    for seg_ea in idautils.Segments():
        seg = ida_segment.getseg(seg_ea)
        if not seg.perm & ida_segment.SEGPERM_EXEC:
            continue
        raw = ida_bytes.get_bytes(seg.start_ea, seg.end_ea - seg.start_ea) or b""
        for prefix in prefixes:
            pos = raw.find(prefix)
            while pos >= 0:
                at = pos + len(prefix)
                if raw[at:at+1] == b"\x3d" and raw[at+5:at+6] == b"\x77":
                    count = struct.unpack_from("<I", raw, at+1)[0] + 1
                    after = at + 7
                    default = seg.start_ea + after + struct.unpack_from("b", raw, at+6)[0]
                    table = None
                    if arch == "x86" and raw[after:after+3] == bytes.fromhex("ff2485"):
                        table = struct.unpack_from("<I", raw, after+3)[0]
                    elif (arch == "x64" and raw[after:after+3] == bytes.fromhex("488d0d")
                          and raw[after+7:after+10] == bytes.fromhex("8b9481")
                          and raw[after+14:after+19] == bytes.fromhex("4803d1ffe2")
                          and seg.start_ea + after + 7 + struct.unpack_from("<i", raw, after+3)[0] == image_base):
                        table = image_base + struct.unpack_from("<I", raw, after+10)[0]
                    if table and 1 <= count <= 2048:
                        candidates.append((table, default, count))
                pos = raw.find(prefix, pos+1)
    if len(candidates) != 1:
        raise ValueError("expected exactly one supported system-command switch")
    table, default, count = candidates[0]
    if (table-image_base != int(fields["table"], 0) or default-image_base != int(fields["default"], 0)
            or count != int(fields["cases"]) or count != len(rows)):
        raise ValueError("located switch disagrees with profile")

    imports = {}
    for i in range(ida_nalt.get_import_module_qty()):
        module = ida_nalt.get_import_module_name(i) or "?"
        def imported(ea, name, ordinal, module=module):
            imports[ea] = module.lower() + "!" + (name or "#" + str(ordinal))
            return True
        ida_nalt.enum_import_names(i, imported)
    commands = []
    noop_shapes = [bytes.fromhex(s) for s in (
        "b8004000004883c4205bc3", "66b80040c20400", "b800400000c20400",
        "b8004000005b5dc20400", "b8004000005e5dc20400")]
    for index, row in enumerate(rows):
        value = ida_bytes.get_dword(table + index*4)
        entry = image_base + value if arch == "x64" else value
        kind = "absent" if entry == default else "noop" if any(
            ida_bytes.get_bytes(entry, len(shape)) == shape for shape in noop_shapes) else "handler"
        recorded = row[2+binary_index]
        if int(row[0], 0) != 0x2000+index or (
                recorded != "-" and int(recorded, 0) != entry-image_base) or (
                (recorded == "-") != (kind == "absent")) or ((row[1] == "noop") != (kind == "noop")):
            raise ValueError("case disagrees with profile: " + row[0])
        commands.append({"opcode": 0x2000+index, "kind": kind,
                         "entry": hex(entry-image_base) if kind != "absent" else None})

    nodes = {}
    queue = [image_base+int(c["entry"], 0) for c in commands if c["kind"] == "handler"]

    def executable(ea):
        seg = ida_segment.getseg(ea)
        return seg is not None and bool(seg.perm & ida_segment.SEGPERM_EXEC)

    while queue:
        start = queue.pop()
        key = hex(start-image_base)
        if key in nodes:
            continue
        node = {"instructions": [], "issues": []}
        nodes[key] = node
        if len(nodes) > 8192 or not executable(start):
            node["issues"].append("unresolved-function")
            continue
        owner = ida_funcs.get_func(start)
        if owner is None:
            node["issues"].append("no-function-boundary")
            continue
        todo, decoded = [start], {}
        while todo:
            ea = todo.pop()
            if ea in decoded:
                continue
            if len(decoded) >= 20000:
                node["issues"].append("instruction-limit")
                break
            insn = ida_ua.insn_t()
            if not ida_bytes.is_code(ida_bytes.get_full_flags(ea)) or not ida_ua.decode_insn(insn, ea):
                node["issues"].append("undecoded-instruction")
                continue
            mnemonic = idc.print_insn_mnem(ea).lower()
            features = insn.get_canon_feature()
            call = bool(features & ida_idp.CF_CALL)
            jump = bool(features & ida_idp.CF_JUMP)
            raw = bytearray(ida_bytes.get_bytes(ea, insn.size))
            refs = []
            ops = [op for op in insn.ops if op.type]
            for op in ops:
                if op.type in (ida_ua.o_near, ida_ua.o_far):
                    if op.type == ida_ua.o_far:
                        node["issues"].append("far-control-flow")
                    target = op.addr
                    other = ida_funcs.get_func(target)
                    local = not call and other is not None and other.start_ea == owner.start_ea
                    if local:
                        todo.append(target)
                        ref = {"kind": "local", "target": target}
                    else:
                        ref = {"kind": "function", "target": hex(target-image_base)}
                        queue.append(target)
                    refs.append((op, ref))
                elif op.type in (ida_ua.o_mem, ida_ua.o_displ, ida_ua.o_imm):
                    target = op.value if op.type == ida_ua.o_imm else op.addr
                    seg = ida_segment.getseg(target)
                    if target in imports:
                        refs.append((op, {"kind": "import", "name": imports[target]}))
                    elif seg:
                        # Unknown/writable data and function pointers prevent inheritance.
                        head = ida_bytes.get_item_head(target)
                        size = ida_bytes.get_item_size(head)
                        access = ida_ua.get_dtype_size(op.dtype)
                        if (seg.perm & (ida_segment.SEGPERM_WRITE | ida_segment.SEGPERM_EXEC)
                                or not ida_bytes.is_data(ida_bytes.get_full_flags(head))
                                or not 0 < size <= 65536 or head+size > seg.end_ea
                                or op.type != ida_ua.o_mem or mnemonic == "lea"
                                or access <= 0 or target+access > head+size):
                            node["issues"].append("unresolved-or-mutable-data")
                            refs.append((op, {"kind": "unknown-data"}))
                        else:
                            data = ida_bytes.get_bytes(head, size)
                            if ida_fixup.contains_fixups(head, size):
                                node["issues"].append("relocated-data")
                            refs.append((op, {"kind": "data", "offset": target-head,
                                              "bytes": data.hex()}))
            if call or jump:
                if not any(op.type in (ida_ua.o_near, ida_ua.o_far) or
                           ref["kind"] == "import" for op, ref in refs):
                    node["issues"].append("indirect-control-flow")
            for op, ref in refs:
                off = op.offb
                later = [o.offb for o in ops if o.offb > off]
                end = min(later) if later else insn.size
                if not off or end-off not in (1, 2, 4, 8):
                    node["issues"].append("unknown-reference-encoding")
                else:
                    raw[off:end] = bytes(end-off)
            # If IDA knows another data reference that no operand explained, do not guess.
            known = {op.value if op.type == ida_ua.o_imm else op.addr for op, _ in refs}
            if any(target not in known for target in idautils.DataRefsFrom(ea)):
                node["issues"].append("unmodeled-data-reference")
            decoded[ea] = {"bytes": raw.hex(), "refs": [ref for _, ref in refs], "next": None}
            if not features & ida_idp.CF_STOP:
                nxt = ea+insn.size
                other = ida_funcs.get_func(nxt)
                if other is not None and other.start_ea == owner.start_ea:
                    todo.append(nxt)
                    decoded[ea]["next"] = nxt
                else:
                    node["issues"].append("fallthrough-outside-function")
        order = {ea: i for i, ea in enumerate(sorted(decoded))}
        node["entry_index"] = order.get(start, -1)
        for ea in sorted(decoded):
            instruction = decoded[ea]
            if instruction["next"] is not None:
                if instruction["next"] not in order:
                    node["issues"].append("missing-fallthrough")
                instruction["next"] = order.get(instruction["next"], -1)
            for ref in instruction["refs"]:
                if ref["kind"] == "local":
                    if ref["target"] not in order:
                        node["issues"].append("missing-local-target")
                    ref["target"] = order.get(ref["target"], -1)
            node["instructions"].append(instruction)
        node["issues"] = sorted(set(node["issues"]))
    result = {"schema": SCHEMA, "policy": POLICY, "arch": arch, "profile": profile_id,
              "binary_sha256": source_hash, "image_base": hex(image_base),
              "table_rva": hex(table-image_base), "commands": commands, "functions": nodes}
    Path(output_path).write_text(json.dumps(result, sort_keys=True, separators=(",", ":"))+"\n", encoding="utf-8")
    return {"arch": arch, "commands": len(commands), "functions": len(nodes), "output": str(output_path)}


if __name__ == "__main__":
    import idc
    try:
        print(export_commands(idc.ARGV[1], idc.ARGV[2]))
    except Exception:
        import traceback
        traceback.print_exc()
        idc.qexit(1)
    else:
        idc.qexit(0)
