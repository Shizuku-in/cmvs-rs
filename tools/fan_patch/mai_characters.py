"""Shared offline character allocation for mai fan patches."""


def character_map(flags: bytes, sjis: tuple[int, ...]) -> dict[int, int]:
    """Invert the mai DLL's Unicode -> custom SJIS allocation.

    Callers supply tables from their digest-pinned patch. Retain the allocator's
    deliberately excluded 0x9f trail and its ordering of duplicate mappings.
    """
    if len(flags) != 65536 or len(sjis) != 65536 or any(v > 1 for v in flags):
        raise ValueError("Invalid character tables")
    used = bytearray(flags)
    available = bytearray(65536)
    for high in (*range(129, 160), *range(224, 240)):
        for low in (*range(64, 127), *range(128, 159), *range(160, 253)):
            available[high * 256 + low] = 1
    available[:34975] = bytes(34975)
    forward = [63] * 65536
    forward[:127] = range(127)
    for code in range(128, 65536):
        if used[code] and sjis[code] != 63:
            forward[code] = sjis[code]
            available[sjis[code]] = 0
            used[code] = 0
    free = iter(i for i, value in enumerate(available) if value)
    for code in range(128, 65536):
        if used[code]:
            try:
                forward[code] = next(free)
            except StopIteration as exc:
                raise ValueError("Character map exceeds available codes") from exc
    inverse = {i: i for i in range(128)}
    for code in range(128, 65536):
        if forward[code] & 0xFF00:
            inverse[forward[code]] = code
    return inverse
