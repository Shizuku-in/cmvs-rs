"""Minimal x64 decoder for the contract lifter; Python standard library only.

Decodes a deliberately small integer subset. Anything else raises Unsupported, so the
lifter reports `unsupported-instruction` instead of guessing. Each call must consume the
whole exported instruction encoding.
"""
import struct

REG64 = ("rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi",
         "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15")
ALU = {0: "add", 1: "or", 4: "and", 5: "sub", 6: "xor", 7: "cmp"}
CC = ("o", "no", "b", "ae", "e", "ne", "be", "a", "s", "ns", "p", "np", "l", "ge", "le", "g")


class Unsupported(ValueError):
    pass


def reg(index, width, rex):
    if width == 8 and not rex and 4 <= index <= 7:
        raise Unsupported("high-byte register")
    return ("reg", REG64[index], width)


class Reader:
    def __init__(self, raw):
        self.raw, self.pos = raw, 0

    def take(self, fmt):
        size = struct.calcsize(fmt)
        if self.pos + size > len(self.raw):
            raise Unsupported("truncated instruction")
        value = struct.unpack_from(fmt, self.raw, self.pos)[0]
        self.pos += size
        return value


def decode(raw):
    return decode_ex(raw)[:3]


def decode_ex(raw):
    """Return (mnemonic, operands, extra, rip_disp_pos) with operands as tuples:
    ("reg", name, width), ("imm", value, width), ("mem", base, index, scale, disp, width),
    ("rel", displacement). `extra` holds a condition code for jcc/setcc/cmovcc."""
    r = Reader(raw)
    rip = []
    opsize16 = rex = 0
    while True:
        b = r.take("B")
        if b == 0x66:
            opsize16 = 1
        elif b == 0xf3 and raw[r.pos:] == bytes((0xc3,)):
            r.pos += 1
            return "ret", [], None, None
        elif b in (0xf0, 0xf2, 0xf3, 0x2e, 0x3e, 0x26, 0x36, 0x64, 0x65, 0x67):
            raise Unsupported("unsupported prefix %02x" % b)
        else:
            break
    if 0x40 <= b <= 0x4f:
        rex = b
        b = r.take("B")
    w = 64 if rex & 8 else 16 if opsize16 else 32
    rexr, rexx, rexb = (rex >> 2) & 1, (rex >> 1) & 1, rex & 1

    def modrm(width, regwidth=None):
        m = r.take("B")
        mod, regf, rm = m >> 6, (m >> 3) & 7, m & 7
        regop = regf | (rexr << 3)
        if mod == 3:
            return regop, reg(rm | (rexb << 3), width, rex), regf
        base = index = None
        scale = 1
        if rm == 4:
            sib = r.take("B")
            scale, idx, bas = 1 << (sib >> 6), ((sib >> 3) & 7) | (rexx << 3), sib & 7
            if idx != 4:
                index = REG64[idx]
            if bas == 5 and mod == 0:
                disp = r.take("<i")
            else:
                base = REG64[bas | (rexb << 3)]
                disp = 0
        elif rm == 5 and mod == 0:
            rip.append(r.pos)
            base, disp = "rip", r.take("<i")
        else:
            base, disp = REG64[rm | (rexb << 3)], 0
        if mod == 1:
            disp = r.take("<b")
        elif mod == 2:
            disp = r.take("<i")
        return regop, ("mem", base, index, scale, disp, width), regf

    def imm(width):
        return r.take({8: "<b", 16: "<h", 32: "<i"}[min(width, 32)])

    out = None
    if b < 0x40 and b & 7 < 6 and (b >> 3) in ALU:
        name, kind = ALU[b >> 3], b & 7
        width = 8 if kind in (0, 2, 4) else w
        if kind < 4:
            regop, rm, _ = modrm(width)
            regs = reg(regop, width, rex)
            out = (name, [rm, regs] if kind < 2 else [regs, rm])
        else:
            out = (name, [reg(0, width, rex), ("imm", imm(width), width)])
    elif b == 0x63 and rex & 8:
        regop, rm, _ = modrm(32)
        out = ("movsxd", [reg(regop, 64, rex), rm])
    elif 0x50 <= b <= 0x5f and not opsize16:
        out = ("push" if b < 0x58 else "pop", [reg((b & 7) | (rexb << 3), 64, rex)])
    elif 0x70 <= b <= 0x7f:
        out = ("jcc", [("rel", r.take("<b"))], CC[b & 15])
    elif b in (0x80, 0x81, 0x83):
        width = 8 if b == 0x80 else w
        _, rm, sub = modrm(width)
        if sub not in ALU:
            raise Unsupported("group1 /%d" % sub)
        value = imm(8 if b != 0x81 else width)
        out = (ALU[sub], [rm, ("imm", value, width)])
    elif b in (0x84, 0x85, 0x88, 0x89, 0x8a, 0x8b):
        width = 8 if b in (0x84, 0x88, 0x8a) else w
        regop, rm, _ = modrm(width)
        regs = reg(regop, width, rex)
        name = "test" if b in (0x84, 0x85) else "mov"
        out = (name, [rm, regs] if b in (0x84, 0x85, 0x88, 0x89) else [regs, rm])
    elif b in (0xa8, 0xa9):
        width = 8 if b == 0xa8 else w
        out = ("test", [reg(0, width, rex), ("imm", imm(width), width)])
    elif b == 0x8d:
        regop, rm, _ = modrm(w)
        if rm[0] != "mem":
            raise Unsupported("lea register")
        out = ("lea", [reg(regop, w, rex), rm])
    elif b == 0x90 and not rex & 1:
        out = ("nop", [])
    elif 0xb0 <= b <= 0xbf:
        width = 8 if b < 0xb8 else w
        value = r.take("<q") if width == 64 else imm(width)
        out = ("mov", [reg((b & 7) | (rexb << 3), width, rex), ("imm", value, width)])
    elif b in (0xc0, 0xc1, 0xd0, 0xd1):
        width = 8 if b in (0xc0, 0xd0) else w
        _, rm, sub = modrm(width)
        names = {4: "shl", 5: "shr", 7: "sar"}
        if sub not in names:
            raise Unsupported("shift /%d" % sub)
        count = r.take("B") if b in (0xc0, 0xc1) else 1
        out = (names[sub], [rm, ("imm", count, 8)])
    elif b == 0xc3:
        out = ("ret", [])
    elif b in (0xc6, 0xc7):
        width = 8 if b == 0xc6 else w
        _, rm, sub = modrm(width)
        if sub:
            raise Unsupported("c6/c7 /%d" % sub)
        out = ("mov", [rm, ("imm", imm(width), width)])
    elif b in (0xe8, 0xe9):
        out = ("call" if b == 0xe8 else "jmp", [("rel", r.take("<i"))])
    elif b == 0xeb:
        out = ("jmp", [("rel", r.take("<b"))])
    elif b in (0xf6, 0xf7):
        width = 8 if b == 0xf6 else w
        _, rm, sub = modrm(width)
        if sub == 0:
            out = ("test", [rm, ("imm", imm(width), width)])
        elif sub in (2, 3):
            out = ("not" if sub == 2 else "neg", [rm])
        else:
            raise Unsupported("f6/f7 /%d" % sub)
    elif b in (0xfe, 0xff):
        width = 8 if b == 0xfe else w
        _, rm, sub = modrm(width)
        if sub not in (0, 1):
            raise Unsupported("indirect or unsupported fe/ff /%d" % sub)
        out = ("inc" if sub == 0 else "dec", [rm])
    elif b == 0x0f:
        b2 = r.take("B")
        if 0x80 <= b2 <= 0x8f:
            out = ("jcc", [("rel", r.take("<i"))], CC[b2 & 15])
        elif 0x90 <= b2 <= 0x9f:
            _, rm, _ = modrm(8)
            out = ("setcc", [rm], CC[b2 & 15])
        elif 0x40 <= b2 <= 0x4f:
            regop, rm, _ = modrm(w)
            out = ("cmovcc", [reg(regop, w, rex), rm], CC[b2 & 15])
        elif b2 in (0xb6, 0xb7, 0xbe, 0xbf):
            regop, rm, _ = modrm(8 if b2 in (0xb6, 0xbe) else 16)
            out = ("movzx" if b2 < 0xb8 else "movsx", [reg(regop, w, rex), rm])
        elif b2 == 0x1f:
            modrm(w)
            out = ("nop", [])
        else:
            raise Unsupported("0f %02x" % b2)
    else:
        raise Unsupported("opcode %02x" % b)
    if r.pos != len(raw):
        raise Unsupported("encoding length mismatch")
    return out[0], out[1], out[2] if len(out) > 2 else None, rip[0] if rip else None
