"""Read-only M1 command contract lifter for same-architecture x64 graph exports.

Symbolically executes one system-command case block of a cmvs-command-graph-v3 export
under an explicit context (dispatcher ABI, VM parameter stack fields), field mappings and
helper summaries, and emits a cmvs-command-contract-v1 envelope. Any instruction, memory
access, call or effect outside the modelled subset blocks the whole command (`unknown`);
nothing is skipped or defaulted. Optional original executables bind every lifted
instruction to the real bytes. Never writes engine profiles or grants runtime dispatch.
"""
import argparse
import copy
import hashlib
import importlib.util
import itertools
import json
import struct
from pathlib import Path


def sibling(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


pc = sibling("profile_compare")
cc = sibling("command_contract")
x64 = sibling("contract_x64")

CONTEXT = "cmvs-contract-context-v1"
MAPPINGS = "cmvs-contract-mappings-v1"
SUMMARIES = "cmvs-contract-summaries-v1"
ENVELOPE = "cmvs-command-contract-v1"
CALLEE_SAVED = ("rbx", "rbp", "rsi", "rdi", "r12", "r13", "r14", "r15")
VOLATILE = ("rax", "rcx", "rdx", "r8", "r9", "r10", "r11")
# Dispatcher x64 (verified bytes, see command-contracts-m1.md): push rbx; sub rsp,0x20;
# movzx eax,dx; mov rbx,rcx; ... jmp rdx. Case blocks run with rbx = VM, saved rbx at
# rsp+0x20 and the dispatcher's return address at rsp+0x28.
DISPATCH_PROLOGUE = bytes.fromhex("40534883ec200fb7c2488bd9")
FRAME_RET = 0x28
BUDGET = {"instructions": 4096, "leaves": 64, "call_depth": 8}
SOURCES = ("contract_extract.py", "contract_x64.py", "command_contract.py", "profile_compare.py")


class Blocked(Exception):
    def __init__(self, issue, where, detail=""):
        super().__init__(issue)
        self.issue = {"issue": issue, "function": where[0], "instruction_index": where[1],
                      "detail": detail}


InvalidEvidence = cc.InvalidEvidence


def evidence_require(condition, message):
    if not condition:
        raise InvalidEvidence(message)


def revision():
    here = Path(__file__).parent
    h = hashlib.sha256()
    for name in SOURCES:
        h.update(name.encode() + b"\0" + (here / name).read_bytes() + b"\0")
    return h.hexdigest()


def body_digest(graph, rva):
    """Summary binding: the exported node with its instruction bytes, refs and CFG."""
    return pc.digest(graph["functions"][rva])


# ---------------------------------------------------------------- rule documents

def check_context(doc):
    cc.keys(doc, "schema revision sides")
    evidence_require(doc["schema"] == CONTEXT, "context schema")
    sides = {}
    for side in doc["sides"]:
        cc.keys(side, "binary_sha256 arch image_base table_rva vm_stack evidence")
        evidence_require(cc.SHA.fullmatch(side["binary_sha256"]) and side["arch"] == "x64", "context side identity")
        cc.keys(side["vm_stack"], "sp_offset base_offset")
        sides[side["binary_sha256"]] = side
    return sides


def check_mappings(doc):
    cc.keys(doc, "schema revision entries")
    evidence_require(doc["schema"] == MAPPINGS, "mapping schema")
    table, ids = {}, set()
    for entry in doc["entries"]:
        cc.keys(entry, "id logical_target sides evidence scope review")
        evidence_require(entry["id"] not in ids, "duplicate mapping id")
        ids.add(entry["id"])
        target = entry["logical_target"]
        evidence_require(isinstance(target, dict) and set(target) <= {"region", "object", "field", "width", "points_to"}
                         and {"region", "object", "field", "width"} <= set(target), "mapping logical target")
        evidence_require(target["region"] in ("vm", "object") and target["width"] in cc.WIDTHS, "mapping region/width")
        evidence_require(isinstance(entry["evidence"], list) and entry["evidence"], "mapping needs evidence")
        for side in entry["sides"]:
            cc.keys(side, "binary_sha256 arch base_object offset width")
            evidence_require(side["arch"] == "x64" and side["width"] == target["width"]
                             and side["base_object"] == target["object"], "mapping side")
            key = (side["binary_sha256"], side["base_object"], int(side["offset"], 16), side["width"])
            evidence_require(key not in table, "ambiguous mapping")
            table[key] = entry
    return table


def check_summaries(doc):
    cc.keys(doc, "schema revision entries")
    evidence_require(doc["schema"] == SUMMARIES, "summary schema")
    table = {}
    for entry in doc["entries"]:
        cc.keys(entry, "id binary_sha256 arch function_rva body_sha256 abi arguments precondition "
                       "tree coverage dependencies evidence review")
        evidence_require(entry["arch"] == "x64" and entry["abi"] == "win64", "summary ABI")
        evidence_require(entry["precondition"] == ["bool", True], "summary precondition")
        evidence_require(entry["tree"]["kind"] == "leaf", "M1 summaries are single-path")
        cov = entry["coverage"]
        evidence_require(all(cov.get(k) is True for k in cc.CLOSED) and cov.get("issues") == [],
                         "summary must be closed")
        key = (entry["binary_sha256"], entry["function_rva"])
        evidence_require(key not in table, "duplicate summary")
        table[key] = entry
    return table


# ---------------------------------------------------------------- symbolic values
# ("bv", width, expr, zx)  value of `width` bits; zx: upper bits of a 64-bit register are 0
# ("ptr", root, offset)    root: "frame", "pstack", "stackbase" or "obj:<logical object>"
# ("sp", 0)                sign-extended VM parameter SP (byte index)
# ("spraw", 0)             32-bit raw SP value, only valid as movsxd source
# ("opaque", name)         entry register/return address identity; any use blocks

def const(width, value):
    return ["bv", width, "%0*x" % (width // 4, value & ((1 << width) - 1))]


def bv(width, expr, zx=False):
    return ("bv", width, expr, zx)


def constant_of(expr):
    if expr[0] == "bv":
        return int(expr[2], 16)
    return None


def fold(op, width, *args):
    values = [constant_of(a) for a in args]
    if all(v is not None for v in values):
        return list(const(width, cc.evaluate([op] + list(args), {}, {})[0]))
    return [op] + list(args)


def resize(expr, source, target, signed=False):
    if source == target:
        return expr
    op = "trunc" if target < source else "sext" if signed else "zext"
    c = constant_of(expr)
    if c is not None:
        if op == "sext" and c >> (source - 1):
            c -= 1 << source
        return const(target, c)
    return [op, target, expr]


# ---------------------------------------------------------------- symbolic execution

class State:
    def __init__(self, entry):
        self.regs = {r: ("opaque", "entry:" + r) for r in CALLEE_SAVED}
        self.regs.update({r: ("opaque", "dispatch-scratch") for r in VOLATILE})
        self.regs["rbx"] = ("ptr", "obj:vm", 0)
        self.regs["rsp"] = ("ptr", "frame", 0)
        self.frame = {0x20: (64, ("opaque", "entry:rbx")),
                      FRAME_RET: (64, ("opaque", "dispatcher-return"))}
        self.flags = None
        self.calls = []
        self.loc = entry
        self.seen = set()
        self.args = set()
        self.events = []

    def fork(self):
        other = copy.copy(self)
        other.regs, other.frame = dict(self.regs), dict(self.frame)
        other.calls, other.seen, other.args = list(self.calls), set(self.seen), set(self.args)
        other.events = []
        return other


class Lifter:
    def __init__(self, graph, context, mappings, summaries, budget=None):
        pc.validate(graph)
        if graph["arch"] != "x64":
            raise ValueError("M1 lifter supports x64 only")
        self.graph, self.sha = graph, graph["binary_sha256"]
        side = context.get(self.sha)
        evidence_require(side is not None, "no context for binary")
        evidence_require(int(side["image_base"], 16) == int(graph["image_base"], 16)
                         and int(side["table_rva"], 16) == int(graph["table_rva"], 16),
                         "context/export identity")
        self.sp_off = int(side["vm_stack"]["sp_offset"], 16)
        self.base_off = int(side["vm_stack"]["base_offset"], 16)
        self.mappings, self.summaries = mappings, summaries
        self.budget = dict(BUDGET, **(budget or {}))
        self.where = (None, None)

    def block(self, issue, detail=""):
        raise Blocked(issue, self.where, detail)

    def event(self, state, **fields):
        fields["id"] = "e%d" % next(self.ids)
        state.events.append(fields)
        return fields["id"]

    def read_reg(self, state, name, width):
        v = state.regs[name]
        if v[0] == "bv":
            if width <= v[1] or v[3]:
                return bv(width, resize(v[2], v[1], width))
            self.block("unsupported-expression", "read of partially defined register " + name)
        if width == 64 and v[0] in ("ptr", "sp", "opaque"):
            return v
        if width == 32 and v[0] == "spraw":
            return v
        self.block("unsupported-expression", "%s used as %d-bit value" % (v[0], width))

    def write_reg(self, state, name, width, value):
        if width == 64:
            if value[0] == "bv" and value[1] != 64:
                self.block("unsupported-expression", "width mismatch")
            state.regs[name] = value
        elif width == 32:
            if value[0] == "spraw":
                state.regs[name] = value
            elif value[0] == "bv":
                state.regs[name] = bv(32, value[2], True)
            else:
                self.block("unsupported-expression", "pointer truncated to 32 bits")
        else:
            if value[0] != "bv":
                self.block("unsupported-expression", "pointer in partial register")
            old = state.regs[name]
            if old[0] == "bv" and constant_of(old[2]) == 0 and (old[3] or old[1] == 64):
                state.regs[name] = bv(old[1], resize(value[2], width, old[1]), old[3])
            else:
                # Only the low bits are known; any wider read blocks.
                state.regs[name] = bv(width, value[2], False)

    def address(self, state, op):
        _, base, index, scale, disp, _ = op
        if base == "rip" or base is None:
            self.block("unknown-memory", "global or absolute address (M2 region model)")
        parts = [self.read_reg(state, base, 64)]
        if index is not None:
            if scale != 1:
                self.block("unknown-memory", "scaled dynamic index")
            parts.append(self.read_reg(state, index, 64))
        if len(parts) == 1 and parts[0][0] == "ptr":
            return ("ptr", parts[0][1], parts[0][2] + disp)
        if sorted(p[0] for p in parts) == ["ptr", "sp"]:
            p = next(x for x in parts if x[0] == "ptr")
            if p[1:] == ("stackbase", 0):
                return ("ptr", "pstack", disp)
        self.block("unknown-memory", "unresolved address")

    def mapping(self, obj, offset, width):
        entry = self.mappings.get((self.sha, obj, offset, width))
        if entry is None:
            self.block("unproven-mapping", "%s+0x%x/%d" % (obj, offset, width))
        self.used_mappings.add(entry["id"])
        t = entry["logical_target"]
        return {"region": t["region"], "object": t["object"], "field": t["field"]}, t.get("points_to")

    def load(self, state, ptr, width):
        _, root, off = ptr
        if root == "frame":
            slot = state.frame.get(off)
            if slot is None or slot[0] != width:
                self.block("unknown-memory", "native stack slot 0x%x not written at width %d" % (off, width))
            return slot[1]
        if root == "pstack":
            key = (off, width)
            for other in self.params:
                if other != key and other[0] < off + width // 8 and off < other[0] + other[1] // 8:
                    self.block("unknown-memory", "overlapping parameter reads")
            if off + width // 8 > 0:
                self.block("unknown-memory", "parameter read at or above SP")
            pid = self.params.setdefault(key, "s%d_%d" % (-off, width))
            if key not in state.args:
                state.args.add(key)
                self.event(state, op="read_parameter", parameter=pid)
            return bv(width, ["arg", pid])
        if root.startswith("obj:"):
            obj = root[4:]
            if obj == "vm" and off == self.sp_off and width == 32:
                return ("spraw", 0)
            if obj == "vm" and off == self.base_off and width == 64:
                return ("ptr", "stackbase", 0)
            target, points = self.mapping(obj, off, width)
            rid = self.event(state, op="read_state", target=target, width=width)
            if points is not None:
                return ("ptr", "obj:" + points, 0)
            return bv(width, ["ref", rid])
        self.block("unknown-memory", "raw parameter stack base access")

    def store(self, state, ptr, width, value):
        _, root, off = ptr
        if root == "frame":
            end = off + width // 8
            # Return address and anything past the dispatcher's 32-byte home space are
            # outside this command; a clobbered saved rbx fails the exit check instead.
            if (off < FRAME_RET + 8 and end > FRAME_RET) or end > FRAME_RET + 8 + 0x20:
                self.block("unsupported-effect", "write to dispatcher return slot or caller frame")
            for k in [k for k, (w, _) in state.frame.items() if k < end and off < k + w // 8]:
                del state.frame[k]
            state.frame[off] = (width, value)
            return
        if root.startswith("obj:"):
            obj = root[4:]
            if obj == "vm" and off in (self.sp_off, self.base_off):
                self.block("unsupported-effect", "parameter stack pointer write")
            target, points = self.mapping(obj, off, width)
            if points is not None or value[0] != "bv" or value[1] != width:
                self.block("unsupported-effect", "pointer or mismatched write")
            self.event(state, op="write_state", target=target, width=width, value=value[2])
            return
        self.block("unsupported-effect", "parameter stack write")

    def read(self, state, op, width):
        if op[0] == "reg":
            return self.read_reg(state, op[1], width)
        if op[0] == "imm":
            return bv(width, const(width, op[1]))
        return self.load(state, self.address(state, op), width)

    def write(self, state, op, width, value):
        if op[0] == "reg":
            self.write_reg(state, op[1], width, value)
        else:
            self.store(state, self.address(state, op), width, value)

    def condition(self, state, code):
        """(bool expression, invert) for a condition code; blocks unmodelled flags."""
        if state.flags is None:
            self.block("unsupported-expression", "flags not modelled")
        kind, a, b, w = state.flags
        zero = const(w, 0)
        if kind == "res":
            table = {"e": (["eq", a, zero], False), "ne": (["eq", a, zero], True),
                     "s": (["slt", a, zero], False), "ns": (["slt", a, zero], True)}
        elif kind == "and":
            r = a if a == b else fold("and", w, a, b)
            table = {"e": (["eq", r, zero], False), "ne": (["eq", r, zero], True),
                     "be": (["eq", r, zero], False), "a": (["eq", r, zero], True),
                     "s": (["slt", r, zero], False), "ns": (["slt", r, zero], True),
                     "l": (["slt", r, zero], False), "ge": (["slt", r, zero], True),
                     "g": (["slt", zero, r], False), "le": (["slt", zero, r], True)}
        else:
            table = {"e": (["eq", a, b], False), "ne": (["eq", a, b], True),
                     "b": (["ult", a, b], False), "ae": (["ult", a, b], True),
                     "a": (["ult", b, a], False), "be": (["ult", b, a], True),
                     "l": (["slt", a, b], False), "ge": (["slt", a, b], True),
                     "g": (["slt", b, a], False), "le": (["slt", b, a], True),
                     "s": (["slt", fold("sub", w, a, b), zero], False),
                     "ns": (["slt", fold("sub", w, a, b), zero], True)}
        if code not in table:
            self.block("unsupported-expression", "condition " + code)
        return table[code]

    def branch(self, state, code, apply):
        """Fork on a condition code; apply(state, holds) mutates each successor."""
        expr, invert = self.condition(state, code)
        try:
            static = cc.evaluate(expr, {}, {})[0]
        except (KeyError, TypeError):
            static = None
        if static is not None:
            apply(state, static != invert)
            return None
        yes, no = state.fork(), state.fork()
        apply(yes, not invert)
        apply(no, invert)
        return {"kind": "branch", "events": state.events, "condition": expr,
                "then": self.run(yes), "else": self.run(no)}

    def width_of(self, op):
        return op[2] if op[0] == "reg" else op[5]

    def frame_ptr(self, state):
        rsp = state.regs["rsp"]
        if rsp[0] != "ptr" or rsp[1] != "frame":
            self.block("unsupported-expression", "native stack pointer lost")
        return rsp[2]

    def exit(self, state):
        off = self.frame_ptr(state)
        saved = all(state.regs[r] == ("opaque", "entry:" + r) for r in CALLEE_SAVED)
        if off != FRAME_RET or state.frame.get(FRAME_RET) != (64, ("opaque", "dispatcher-return")) or not saved:
            self.block("cfg-gap", "case exit violates dispatcher frame or callee-saved registers")
        code = constant_of(self.read_reg(state, "rax", 32)[2])
        if code is None:
            self.block("unsupported-expression", "non-constant return code")
        if code & 0xa000 or not code & 0x4000 or code >> 16:
            self.block("unsupported-effect", "yield/retry/transfer return 0x%x" % code)
        self.leaves += 1
        if self.leaves > self.budget["leaves"]:
            self.block("budget-exceeded", "leaf paths")
        # Dispatcher rule: bit 14 advances PC and releases the low byte of parameter stack.
        self.event(state, op="release_stack", bytes=code & 0xff)
        return {"kind": "leaf", "events": state.events, "exit": {"kind": "return", "code": const(32, code)}}

    def apply_summary(self, state, entry, target, insn):
        evidence_require(entry["body_sha256"] == body_digest(self.graph, target), "stale summary body " + entry["id"])
        for dep in entry["dependencies"]:
            cc.keys(dep, "function_rva body_sha256")
            evidence_require(dep["function_rva"] in self.graph["functions"]
                             and dep["body_sha256"] == body_digest(self.graph, dep["function_rva"]),
                             "stale summary dependency " + entry["id"])
        values = {}
        for arg in entry["arguments"]:
            cc.keys(arg, "id register width")
            v = self.read_reg(state, arg["register"], arg["width"])
            if v[0] != "bv":
                self.block("unsupported-expression", "pointer argument to summary " + entry["id"])
            values[arg["id"]] = v[2]
        renamed = {}

        def subst(e):
            if e[0] == "arg":
                return values[e[1]]
            if e[0] == "ref":
                return ["ref", renamed[e[1]]]
            return [subst(x) if isinstance(x, list) else x for x in e]
        for ev in entry["tree"]["events"]:
            evidence_require(ev.get("op") in ("read_state", "write_state"), "M1 summary events are state only")
            fields = {k: copy.deepcopy(v) for k, v in ev.items() if k != "id"}
            if "value" in fields:
                fields["value"] = subst(fields["value"])
            renamed[ev["id"]] = self.event(state, **fields)
        state.regs["rax"] = bv(32, subst(entry["tree"]["exit"]["code"]), True)
        for r in VOLATILE[1:]:
            state.regs[r] = ("opaque", "clobbered")
        state.flags = None
        self.used_summaries.add(entry["id"])
        state.loc = (state.loc[0], insn["next"])

    def run(self, state):
        while True:
            fn, idx = state.loc
            self.where = (fn, idx)
            node = self.graph["functions"].get(fn)
            if node is None or node.get("library") is not None:
                self.block("unresolved-call", "library or missing function")
            # Function issues explained by a flagged instruction block only on that
            # instruction's path; unexplained ones (CFG gaps) block the whole function.
            local_issues = {i for x in node["instructions"] for i in x.get("issues", [])}
            unexplained = set(node["issues"]) - local_issues
            if unexplained or type(idx) is not int or not 0 <= idx < len(node["instructions"]):
                self.block("cfg-gap", ",".join(sorted(unexplained)) or "bad instruction index")
            key = (fn, idx, len(state.calls))
            if key in state.seen:
                self.block("loop")
            state.seen.add(key)
            self.count += 1
            if self.count > self.budget["instructions"]:
                self.block("budget-exceeded", "instructions")
            insn = node["instructions"][idx]
            if insn.get("issues"):
                kinds = set(insn["issues"])
                self.block("unknown-memory" if kinds == {"unresolved-or-mutable-data"}
                           else "unresolved-call" if kinds == {"indirect-control-flow"} else "cfg-gap",
                           ",".join(sorted(kinds)))
            # Exports zero branch displacements; record the target the original must encode.
            branch_target = None
            for ref in insn["refs"]:
                if ref["kind"] == "function":
                    branch_target = int(ref["target"], 16)
                elif ref["kind"] == "local" and 0 <= ref["target"] < len(node["instructions"]):
                    branch_target = int(fn, 16) + node["instructions"][ref["target"]]["offset"]
            self.executed.add((fn, insn["offset"], insn["bytes"], branch_target))
            try:
                name, ops, code = x64.decode(bytes.fromhex(insn["bytes"]))
            except x64.Unsupported as error:
                self.block("unsupported-instruction", str(error))
            refs = insn["refs"]
            if any(r["kind"] not in ("local", "function") for r in refs):
                self.block("unknown-memory" if name not in ("call", "jmp") else "unresolved-call",
                           ",".join(sorted({r["kind"] for r in refs})))
            local = [r["target"] for r in refs if r["kind"] == "local"]
            func = [r["target"] for r in refs if r["kind"] == "function"]
            nxt = insn["next"]
            if name == "ret":
                if not state.calls:
                    return self.exit(state)
                caller, ret_index, marker = state.calls.pop()
                off = self.frame_ptr(state)
                if state.frame.get(off) != (64, marker):
                    self.block("cfg-gap", "return address mismatch")
                del state.frame[off]
                state.regs["rsp"] = ("ptr", "frame", off + 8)
                state.loc = (caller, ret_index)
                continue
            if name == "jcc":
                if len(local) != 1:
                    self.block("cfg-gap", "conditional target")
                t = local[0]
                result = self.branch(state, code, lambda s, holds: setattr(s, "loc", (fn, t if holds else nxt)))
                if result is not None:
                    return result
                continue
            if name in ("jmp", "call"):
                if name == "jmp" and len(local) == 1 and not func:
                    state.loc = (fn, local[0])
                    continue
                if len(func) != 1 or local:
                    self.block("unresolved-call", "indirect or unresolved target")
                target = func[0]
                summary = self.summaries.get((self.sha, target))
                if name == "call" and summary is not None:
                    self.apply_summary(state, summary, target, insn)
                    continue
                active = {c[0] for c in state.calls} | {fn}
                if target in active:
                    self.block("recursion", target)
                callee = self.graph["functions"][target]
                if name == "call":
                    if len(state.calls) >= self.budget["call_depth"]:
                        self.block("budget-exceeded", "call depth")
                    marker = ("opaque", "ret:%d" % next(self.ids))
                    off = self.frame_ptr(state) - 8
                    state.frame[off] = (64, marker)
                    state.regs["rsp"] = ("ptr", "frame", off)
                    state.calls.append((fn, nxt, marker))
                elif summary is not None:
                    self.block("unresolved-call", "summary reached through tail jump")
                state.loc = (target, callee.get("entry_index"))
                continue
            result = self.step(state, name, ops, code, fn, nxt)
            if result is not None:
                return result
            if nxt is None:
                self.block("cfg-gap", "missing fallthrough")
            state.loc = (fn, nxt)

    def step(self, state, name, ops, code, fn, nxt):
        if name == "nop":
            return None
        if name in ("add", "sub", "and", "or", "xor", "cmp", "test"):
            w = self.width_of(ops[0])
            if name in ("xor", "sub") and ops[0] == ops[1] and ops[0][0] == "reg":
                zero = bv(w, const(w, 0))
                self.write(state, ops[0], w, zero)
                state.flags = ("res", zero[2], None, w)
                return None
            a, b = self.read(state, ops[0], w), self.read(state, ops[1], w)
            if a[0] == "ptr" and b[0] == "bv" and name in ("add", "sub") and constant_of(b[2]) is not None:
                delta = constant_of(b[2])
                delta = delta - (1 << 64) if delta >> 63 else delta
                self.write(state, ops[0], w, ("ptr", a[1], a[2] + (delta if name == "add" else -delta)))
                state.flags = None
                return None
            if a[0] != "bv" or b[0] != "bv":
                self.block("unsupported-expression", "pointer arithmetic or comparison")
            op = {"cmp": "sub", "test": "and"}.get(name, name)
            result = fold(op, w, a[2], b[2])
            state.flags = (("sub", a[2], b[2], w) if op == "sub" else ("and", a[2], b[2], w) if op == "and"
                           else ("res", result, None, w))
            if name not in ("cmp", "test"):
                self.write(state, ops[0], w, bv(w, result))
            return None
        if name in ("inc", "dec", "neg", "not"):
            w = self.width_of(ops[0])
            a = self.read(state, ops[0], w)
            if a[0] != "bv":
                self.block("unsupported-expression", "pointer arithmetic")
            result = {"inc": lambda: fold("add", w, a[2], const(w, 1)),
                      "dec": lambda: fold("sub", w, a[2], const(w, 1)),
                      "neg": lambda: fold("sub", w, const(w, 0), a[2]),
                      "not": lambda: fold("not", w, a[2])}[name]()
            if name != "not":
                state.flags = ("res", result, None, w)
            self.write(state, ops[0], w, bv(w, result))
            return None
        if name in ("shl", "shr", "sar"):
            w = self.width_of(ops[0])
            a = self.read(state, ops[0], w)
            value = constant_of(a[2]) if a[0] == "bv" else None
            if value is None:
                self.block("unsupported-expression", "symbolic shift")
            n = ops[1][1] & (63 if w == 64 else 31)
            if name == "sar" and value >> (w - 1):
                value -= 1 << w
            result = const(w, value << n if name == "shl" else value >> n)
            state.flags = ("res", result, None, w)
            self.write(state, ops[0], w, bv(w, result))
            return None
        if name == "mov":
            w = self.width_of(ops[0])
            self.write(state, ops[0], w, self.read(state, ops[1], w))
            return None
        if name in ("movzx", "movsx", "movsxd"):
            w, sw = ops[0][2], self.width_of(ops[1])
            v = self.read(state, ops[1], sw)
            if v[0] == "spraw" and name == "movsxd":
                self.write(state, ops[0], 64, ("sp", 0))
                return None
            if v[0] != "bv":
                self.block("unsupported-expression", "extension of pointer")
            self.write(state, ops[0], w, bv(w, resize(v[2], sw, w, name != "movzx")))
            return None
        if name == "lea":
            if ops[0][2] != 64:
                self.block("unsupported-expression", "narrow lea")
            self.write_reg(state, ops[0][1], 64, self.address(state, ops[1]))
            return None
        if name == "push":
            v = self.read_reg(state, ops[0][1], 64)
            off = self.frame_ptr(state) - 8
            self.store(state, ("ptr", "frame", off), 64, v)
            state.regs["rsp"] = ("ptr", "frame", off)
            return None
        if name == "pop":
            off = self.frame_ptr(state)
            v = self.load(state, ("ptr", "frame", off), 64)
            state.regs["rsp"] = ("ptr", "frame", off + 8)
            self.write_reg(state, ops[0][1], 64, v)
            return None
        if name == "setcc":
            def apply(s, holds):
                self.write(s, ops[0], 8, bv(8, const(8, int(holds))))
                s.loc = (fn, nxt)
            return self.branch(state, code, apply)
        if name == "cmovcc":
            w = ops[0][2]
            src, old = self.read(state, ops[1], w), self.read_reg(state, ops[0][1], w)

            def apply(s, holds):
                self.write_reg(s, ops[0][1], w, src if holds else old)
                s.loc = (fn, nxt)
            return self.branch(state, code, apply)
        self.block("unsupported-instruction", name)

    def lift(self, opcode):
        self.ids = itertools.count()
        self.params, self.used_mappings, self.used_summaries = {}, set(), set()
        self.executed, self.count, self.leaves = set(), 0, 0
        command = self.graph["commands"][opcode - 0x2000]
        base = {"opcode": "0x%x" % opcode, "binary_sha256": self.sha}
        if command["kind"] != "handler":
            return {**base, "status": "unknown", "issues": [{"issue": "not-handler", "detail": command["kind"]}]}
        entry = command["entry"]
        base["case_rva"] = entry
        try:
            tree = self.run(State((entry, self.graph["functions"][entry].get("entry_index"))))
        except Blocked as error:
            return {**base, "status": "unknown", "issues": [error.issue], "instructions": self.count}
        params = sorted(self.params.items(), key=lambda kv: (-kv[0][0], kv[0][1]))
        contract = {"precondition": ["bool", True],
                    "parameters": [{"id": pid, "stack_offset": off, "width": w, "interpretation": "bits"}
                                   for (off, w), pid in params],
                    "tree": tree,
                    "coverage": {**{k: True for k in cc.CLOSED}, "issues": []}}
        try:
            cc.validate(contract)
        except cc.Unsupported as error:
            return {**base, "status": "unknown", "issues": [{"issue": str(error), "detail": "IR subset"}]}
        return {**base, "status": "extracted", "contract": contract, "instructions": self.count,
                "mappings": sorted(self.used_mappings), "summaries": sorted(self.used_summaries),
                "executed": sorted(self.executed)}


# ---------------------------------------------------------------- binary binding

class Image:
    """Read-only PE32+ view used to bind exported instructions to the original bytes."""

    def __init__(self, raw):
        self.raw = raw
        self.sha256 = hashlib.sha256(raw).hexdigest()
        evidence_require(raw[:2] == b"MZ", "not a PE file")
        pe = struct.unpack_from("<I", raw, 0x3c)[0]
        evidence_require(raw[pe:pe + 4] == b"PE\0\0", "not a PE file")
        count, opt_size = struct.unpack_from("<H", raw, pe + 6)[0], struct.unpack_from("<H", raw, pe + 20)[0]
        opt = pe + 24
        evidence_require(struct.unpack_from("<H", raw, opt)[0] == 0x20b, "PE32+ required")
        self.image_base = struct.unpack_from("<Q", raw, opt + 24)[0]
        self.sections = []
        for i in range(count):
            s = opt + opt_size + 40 * i
            vsize, va, rsize, rptr = struct.unpack_from("<IIII", raw, s + 8)
            flags = struct.unpack_from("<I", raw, s + 36)[0]
            self.sections.append((va, max(vsize, rsize), rptr, rsize, flags))

    def read(self, rva, size):
        for va, vsize, rptr, rsize, _ in self.sections:
            if va <= rva and rva + size <= va + min(vsize, rsize):
                return self.raw[rptr + rva - va:rptr + rva - va + size]
        raise InvalidEvidence("rva 0x%x outside file-backed sections" % rva)


def verify_binary(image, graph, opcode, executed):
    """Bind dispatcher, case table entry and every executed instruction to the image."""
    evidence_require(image.sha256 == graph["binary_sha256"], "executable hash differs from export")
    evidence_require(image.image_base == int(graph["image_base"], 16), "image base differs")
    table = int(graph["table_rva"], 16)
    cases = len(graph["commands"])
    pattern = DISPATCH_PROLOGUE + b"\x3d" + struct.pack("<I", cases - 1) + b"\x77"
    hits = []
    for va, _, rptr, rsize, flags in image.sections:
        if not flags & 0x20000000:
            continue
        data = image.raw[rptr:rptr + rsize]
        pos = data.find(pattern)
        while pos >= 0:
            hits.append((va + pos, data[pos:pos + 48]))
            pos = data.find(pattern, pos + 1)
    evidence_require(len(hits) == 1, "expected exactly one dispatcher prologue")
    start, code = hits[0]
    at = len(pattern) + 1
    lea = start + at + 7 + struct.unpack_from("<i", code, at + 3)[0]
    evidence_require(code[at:at + 3] == bytes.fromhex("488d0d") and lea == 0
                     and code[at + 7:at + 10] == bytes.fromhex("8b9481")
                     and struct.unpack_from("<I", code, at + 10)[0] == table
                     and code[at + 14:at + 19] == bytes.fromhex("4803d1ffe2"), "dispatcher switch shape")
    command = graph["commands"][opcode - 0x2000]
    if command["kind"] == "handler":
        entry = struct.unpack_from("<I", image.read(table + 4 * (opcode - 0x2000), 4))[0]
        evidence_require(entry == int(command["entry"], 16), "case table entry differs")
    for fn, offset, raw, target in executed:
        rva = int(fn, 16) + offset
        real = image.read(rva, len(raw) // 2)
        if target is not None:
            try:
                _, ops, _ = x64.decode(real)
            except x64.Unsupported:
                raise InvalidEvidence("undecodable branch at %s+0x%x" % (fn, offset)) from None
            rel = [o[1] for o in ops if o[0] == "rel"]
            size = 1 if real[0] == 0xeb or 0x70 <= real[0] <= 0x7f else 4
            evidence_require(len(rel) == 1 and rva + len(real) + rel[0] == target,
                             "branch target differs at %s+0x%x" % (fn, offset))
            real = real[:-size] + bytes(size)
        evidence_require(real.hex() == raw, "instruction bytes differ at %s+0x%x" % (fn, offset))
    return {"dispatcher_rva": hex(start), "instructions": len(executed)}


# ---------------------------------------------------------------- envelopes and CLI

def envelope(graph, export_sha, rules, side):
    identity = {"profile_id": graph["profile"], "binary_sha256": graph["binary_sha256"], "arch": "x64",
                "image_base": "0x%x" % int(graph["image_base"], 16), "opcode": side["opcode"],
                "case_rva": "0x%x" % int(side["case_rva"], 16)}
    evidence = {"export_sha256": export_sha, "extractor_revision": rules["extractor_revision"],
                "rules_sha256": rules["rules_sha256"], "mapping_sha256": rules["mapping_sha256"],
                "summaries_sha256": rules["summaries_sha256"], "binary_checked": side.get("binary") is not None,
                "mappings_used": side["mappings"], "summaries_used": side["summaries"]}
    return {"schema": ENVELOPE, "identity": identity, "evidence": evidence, "contract": side["contract"]}


def load_json(path, limit=256 * 1024 * 1024):
    raw = Path(path).read_bytes()
    if len(raw) > limit:
        raise ValueError("input size limit")

    def unique(items):
        obj = {}
        for key, value in items:
            if key in obj:
                raise ValueError("duplicate JSON key")
            obj[key] = value
        return obj
    return raw, json.loads(raw, object_pairs_hook=unique)


def compare_pair(base, derived, rules, opcode, images=(None, None)):
    """Lift one opcode on both sides and compare. Returns a report row (never a profile)."""
    row = {"opcode": "0x%x" % opcode}
    sides = []
    try:
        for name, (lifter, graph, export_sha), image in zip(("base", "derived"), (base, derived), images):
            side = lifter.lift(opcode)
            if side["status"] == "extracted" and image is not None:
                side["binary"] = verify_binary(image, graph, opcode, side["executed"])
            row[name] = {k: v for k, v in side.items() if k not in ("contract", "executed")}
            sides.append((graph, export_sha, side))
        if any(s[2]["status"] != "extracted" for s in sides):
            row["status"] = "unknown"
            row["reason"] = "extraction-incomplete"
            return row
        envs = [envelope(g, e, rules, s) for g, e, s in sides]
        result = cc.compare_envelopes(envs[0], envs[1], rules)
    except (InvalidEvidence, cc.Invalid) as error:
        row.update(status="invalid-evidence", reason=str(error))
        return row
    row.update(result)
    if result["status"] == "equivalent":
        row["contract"] = envs[0]["contract"]
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base_export")
    parser.add_argument("derived_export")
    parser.add_argument("--context", required=True)
    parser.add_argument("--mappings", required=True)
    parser.add_argument("--summaries", required=True)
    parser.add_argument("--base-exe")
    parser.add_argument("--derived-exe")
    parser.add_argument("opcodes", nargs="*", type=lambda v: int(v, 0),
                        help="default: every derived handler")
    args = parser.parse_args()
    try:
        docs = {}
        for key in ("context", "mappings", "summaries"):
            raw, docs[key] = load_json(getattr(args, key), 8 * 1024 * 1024)
            docs[key + "_sha256"] = hashlib.sha256(raw).hexdigest()
        context = check_context(docs["context"])
        mappings = check_mappings(docs["mappings"])
        summaries = check_summaries(docs["summaries"])
        rules = {"extractor_revision": revision(), "rules_sha256": docs["context_sha256"],
                 "mapping_sha256": docs["mappings_sha256"], "summaries_sha256": docs["summaries_sha256"]}
        loaded = []
        for path in (args.base_export, args.derived_export):
            raw, graph = load_json(path)
            loaded.append((Lifter(graph, context, mappings, summaries), graph, hashlib.sha256(raw).hexdigest()))
        if loaded[0][1]["binary_sha256"] == loaded[1][1]["binary_sha256"]:
            raise ValueError("expected distinct base and derived builds")
        images = tuple(Image(Path(p).read_bytes()) if p else None for p in (args.base_exe, args.derived_exe))
        opcodes = args.opcodes or [c["opcode"] for c in loaded[1][1]["commands"] if c["kind"] == "handler"]
        rows = [compare_pair(loaded[0], loaded[1], rules, op, images) for op in opcodes]
        totals = {}
        blockers = {}
        for row in rows:
            totals[row["status"]] = totals.get(row["status"], 0) + 1
            for side in ("base", "derived"):
                for issue in row.get(side, {}).get("issues", []):
                    blockers[issue["issue"]] = blockers.get(issue["issue"], 0) + 1
        result = {"status": "report", "scope": "offline report; no profile write or runtime authorization",
                  "rules": rules, "totals": totals, "blockers": blockers, "rows": rows}
        code = 0
    except (InvalidEvidence, cc.Invalid) as error:
        result, code = {"status": "invalid-evidence", "reason": str(error)}, 4
    except (ValueError, TypeError, KeyError, IndexError, RecursionError, OSError) as error:
        result, code = {"status": "invalid-input", "reason": str(error)}, 3
    print(json.dumps(result, ensure_ascii=False, indent=1))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
