"""Prototype of the app's number guard (ported to src-tauri/src/llm.rs).

Every number FlowScribe writes must be traceable to what was said: a digit
string in the transcript, a spoken number ("two thousand three hundred and
forty five"), a time ("three thirty" -> 3, 30) or a year ("twenty twenty
five" -> 2025). Otherwise the output is rejected.
"""

import re

UNITS = {w: i for i, w in enumerate("zero one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen".split())}
TENS = {w: 10 * i for i, w in enumerate("_ _ twenty thirty forty fifty sixty seventy eighty ninety".split()) if w != "_"}
ORD_UNITS = {w: i for i, w in enumerate("zeroth first second third fourth fifth sixth seventh eighth ninth tenth eleventh twelfth thirteenth fourteenth fifteenth sixteenth seventeenth eighteenth nineteenth".split())}
ORD_TENS = {"twentieth": 20, "thirtieth": 30, "fortieth": 40, "fiftieth": 50, "sixtieth": 60, "seventieth": 70, "eightieth": 80, "ninetieth": 90}
SCALES = {"hundred": 100, "thousand": 1000, "million": 1_000_000, "billion": 1_000_000_000}


def ordered_numbers(text: str) -> list[int]:
    """Numbers in `text` in order: digit runs as written, number phrases by
    their standard reading ("two thousand three hundred and forty five" ->
    2345, "three thirty" -> 3, 30, "twenty twenty five" -> 20, 25)."""
    tokens = re.findall(r"\d[\d,]*(?:\.\d+)?|[a-z]+", text.lower().replace("-", " "))
    values: list[int] = []
    cur = total = 0
    last = None  # "unit" | "teen" | "tens" | "scale"
    active = False

    def flush():
        nonlocal cur, total, active, last
        if active:
            values.append(total + cur)
        cur, total, active, last = 0, 0, False, None

    for i, w in enumerate(tokens):
        nxt = tokens[i + 1] if i + 1 < len(tokens) else ""
        if w[0].isdigit():
            flush()
            intpart, _, frac = w.rstrip(",").replace(",", "").partition(".")
            values.append(int(intpart))
            if frac:
                values.append(int(frac))
            continue
        if w == "a" and nxt in SCALES and not active:
            cur, active, last = 1, True, "unit"
            continue
        if w == "and" and active and (nxt in UNITS or nxt in TENS or nxt in ORD_UNITS or nxt in ORD_TENS):
            continue
        if w in SCALES and active:
            if SCALES[w] == 100:
                cur = (cur or 1) * 100
            else:
                total += (cur or 1) * SCALES[w]
                cur = 0
            last = "scale"
            continue
        if w in UNITS or w in ORD_UNITS:
            val = UNITS.get(w, ORD_UNITS.get(w))
            kind = "teen" if val >= 10 else "unit"
        elif w in TENS or w in ORD_TENS:
            val, kind = TENS.get(w, ORD_TENS.get(w)), "tens"
        else:
            flush()
            continue
        continues = active and ((kind == "unit" and last == "tens") or last == "scale")
        if not continues:
            flush()
            active = True
        cur += val
        last = kind
    flush()
    return values


def allowed_numbers(inp: str) -> set[str]:
    seq = [str(v) for v in ordered_numbers(inp)]
    allowed = set(seq) | {"0"}  # "4:00" from "four"
    # Adjacent concatenations: years ("twenty twenty five"), codes ("nine two").
    for n in (2, 3, 4):
        for i in range(len(seq) - n + 1):
            allowed.add("".join(seq[i : i + n]))
    return allowed


def output_numbers(out: str) -> list[str]:
    nums = []
    for m in re.finditer(r"\d[\d,]*(?:\.\d+)?", out):
        s = m.group().rstrip(",.")
        intpart, _, frac = s.replace(",", "").partition(".")
        nums.append(intpart.lstrip("0") or "0")
        if frac:
            nums.append(frac)
    return nums


def numbers_supported(out: str, inp: str) -> bool:
    allowed = allowed_numbers(inp)
    allowed |= {a.lstrip("0") or "0" for a in allowed}
    for n in output_numbers(out):
        if n in allowed:
            continue
        return False
    return True


if __name__ == "__main__":
    cases = [
        ("25% reduction", "twenty five percent reduction", True),
        ("20% reduction", "twenty five percent reduction", False),
        ("$2,345.60 by the 28th", "two thousand three hundred and forty five dollars and sixty cents by the twenty eighth", True),
        ("$2,300 by the 28th", "two thousand three hundred and forty five dollars and sixty cents by the twenty eighth", False),
        ("October 2025", "october twenty twenty five", True),
        ("October 2020", "october twenty twenty five", False),
        ("at 3:30", "at three thirty", True),
        ("api-7f92b", "api dash seven f nine two b", True),
        ("5-minute warm-up, 3 sets of 12", "five minute warm up three sets of twelve", True),
        ("1,500 dollars", "fifteen hundred dollars", True),
        ("a 100 people", "a hundred people", True),
        ("from $5,000 to $7,000", "from $5,000 to $7,000", True),
        ("Friday at 4:00", "friday at four", True),
    ]
    for out, inp, want in cases:
        got = numbers_supported(out, inp)
        print(("ok " if got == want else "BAD"), want, "|", out, "|", inp, allowed_numbers(inp))
