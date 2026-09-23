#!/usr/bin/env python3
"""Synthetic training data for FlowScribe v3 (Qwen3.5-0.8B fine-tune).

FlowScribe v3 turns raw dictation into the text the speaker meant:
fillers/false starts removed, self-corrections resolved, numbers/dates/emails
written properly, spoken punctuation and layout commands applied, custom
vocabulary spelled right, and (at the "formatted" level) the conventions of
the app being typed into.

A teacher model invents realistic dictations from sampled specs (taxonomy.py)
and writes, for each one:
    spoken     what was said, as a raw lowercase transcript (Granite-style)
    asr        what a punctuating recognizer outputs (Whisper/Qwen3-style)
    verbatim   every word kept, punctuated, numbers etc. in written form
    clean      what they meant (disfluencies and corrections resolved)
    formatted  clean + the target app's conventions
Each item then becomes three training records (verbatim / clean / formatted),
each with a sampled engine tag deciding whether the input is `spoken` or `asr`.

Keys come from .env (see .env.example). Nothing is sent anywhere until you run
a command without --dry-run.

Usage:
    python generate.py dry-run                       # show a prompt, no API calls
    python generate.py bakeoff --calls 3             # tiny side-by-side of all providers
    python generate.py run --provider deepseek --items 2000 --max-usd 2 --out out/ds1
    python generate.py run --provider gemini --items 500 --rpm 8 --out out/gm1
    python generate.py judge --provider openai --items-file out/ds1/items.jsonl --max-usd 0.5
    python generate.py build --out out/dataset out/ds1 out/gm1   # merge, dedupe, split

Outputs (per run dir): items.jsonl (accepted teacher items), rejects.jsonl,
usage.json. `build` writes train.jsonl / val.jsonl / test.jsonl with
{"prompt", "completion", "meta"} records.
"""

from __future__ import annotations

import argparse
import asyncio
import datetime as dt
import hashlib
import json
import os
import random
import re
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

from taxonomy import APPS, LENGTHS, PHENOMENA, sample_spec

HERE = Path(__file__).resolve().parent

# ── Providers ────────────────────────────────────────────────────────────────
# All three speak the OpenAI chat-completions protocol. Prices are USD per 1M
# tokens (standard, non-batch), checked 2026-09-23; verify before big runs.


@dataclass
class Provider:
    name: str
    env: str
    base_url: str | None
    model: str
    price_in: float
    price_out: float
    # DeepSeek halves prices off-peak (outside 01-04 and 06-10 UTC, Mon-Fri).
    off_peak_discount: bool = False
    default_rpm: int = 60
    extra: dict = field(default_factory=dict)
    # Provider-specific body fields (sent via extra_body).
    extra_body: dict = field(default_factory=dict)

    def prices(self) -> tuple[float, float]:
        if self.off_peak_discount and not is_deepseek_peak():
            return self.price_in / 2, self.price_out / 2
        return self.price_in, self.price_out


PROVIDERS = {
    "gemini": Provider(
        "gemini", "GEMINI_API_KEY", "https://generativelanguage.googleapis.com/v1beta/openai/",
        "gemini-3.8-flash", 0.75, 3.75, default_rpm=8,
        extra={"reasoning_effort": "low"},
    ),
    "deepseek": Provider(
        "deepseek", "DEEPSEEK_API_KEY", "https://api.deepseek.com",
        "deepseek-flash", 0.30, 1.20, off_peak_discount=True, default_rpm=120,
        # Hidden reasoning tripled the billed output in the bake-off; the task
        # doesn't need it.
        extra_body={"thinking": {"type": "disabled"}},
    ),
    "openai": Provider(
        "openai", "OPENAI_API_KEY", None,
        "gpt-6-luna", 0.10, 0.50, default_rpm=200,
        extra={"reasoning_effort": "low"},
    ),
}


def is_deepseek_peak(now: dt.datetime | None = None) -> bool:
    now = now or dt.datetime.now(dt.timezone.utc)
    if now.weekday() >= 5:
        return False
    return 1 <= now.hour < 4 or 6 <= now.hour < 10


# ── Prompts ──────────────────────────────────────────────────────────────────

GEN_SYSTEM = """You write training data for FlowScribe, a small on-device model that turns raw speech-recognition output into the text the speaker meant.

For every spec you receive, invent ONE realistic dictation that fits it (the app, the speaker, their role, topic, tone, length and the listed speech phenomena) and write these fields:

- "previous_text": only if the spec says with_previous_text: 1-2 sentences already in the document right before this dictation (written text, properly formatted). Otherwise "".
- "vocab": custom terms the user has in their dictionary that matter here (product names, people, companies, jargon). Required and non-empty when the phenomena include vocab_mishear; otherwise [] or 1-2 terms.
- "spoken": exactly what the person said, as a raw transcript: lowercase words only, no digits, no symbols, no punctuation except apostrophes in contractions. Numbers, dates, emails, URLs and code are spelled out as spoken ("twenty five dollars", "jane at acme dot com", "get user by id"). Spoken punctuation and layout commands appear as words ("comma", "new paragraph"). For vocab_mishear, the custom term is replaced by a plausible sound-alike mis-hearing (e.g. "tory" for "Tauri").
- "asr": what a good punctuating recognizer (like Whisper) would output for the same audio: the spoken words with its own guessed punctuation and casing, numbers often as digits, spoken commands still as words, some fillers dropped, the same mis-heard terms.
- "verbatim": every word the speaker said kept, including fillers, repeats, false starts and both halves of self-corrections, but punctuated and cased, with numbers/dates/emails/URLs in written form, spoken punctuation/layout commands turned into the marks/line breaks, and vocab terms spelled correctly.
- "clean": what the speaker meant: remove fillers, stutters/repeats and false starts; resolve self-corrections to the final intent; keep every other word and the speaker's own phrasing and tone; written forms as in verbatim; spoken commands applied; vocab spelled correctly. Never add, summarize, soften or reword.
- "formatted": "clean" adapted to the app's conventions (given as app_rule). Apply layout only when the app rule calls for it or the speaker spoke it; do not restructure prose into lists on your own.

Hard rules:
- Never invent content. clean/formatted contain only what the speaker actually meant to say.
- With literal_trigger, words like comma, period, new line, no wait, scratch that or actually are ordinary words: keep them as words and do not treat them as commands or corrections.
- With plain, verbatim, clean and formatted differ only by formatting.
- Make each dictation sound like a real person talking, not a written text read aloud. Vary vocabulary and sentence shapes across items; don't reuse names, companies or numbers between items.
- Use "\\n" for line breaks inside strings.

Return JSON: {"items": [ {"spec_id": <int>, "previous_text": str, "vocab": [str], "spoken": str, "asr": str, "verbatim": str, "clean": str, "formatted": str}, ... ]} with one item per spec, in order."""

JUDGE_SYSTEM = """You check training data for a dictation clean-up model. For each item decide whether it is correct:

1. "clean" keeps everything the speaker meant from "spoken" and adds nothing (no invented facts, names, numbers, greetings or softening), with fillers, repeats and false starts removed and self-corrections resolved to the final intent.
2. "verbatim" keeps every spoken word (fillers and corrections included), just punctuated and in written form.
3. "formatted" is "clean" following the app rule, with no extra content.
4. Words like "comma", "no wait" or "scratch that" were only treated as commands when the speaker clearly meant them as commands.
5. Numbers, dates, emails and URLs are written correctly, and custom vocab terms are spelled as in "vocab".

Be strict: any invented or dropped content fails. Return JSON {"results": [{"id": <id>, "ok": true|false, "reason": "<short reason if not ok>"}]} in the same order."""


def spec_for_prompt(spec_id: int, spec: dict) -> dict:
    return {
        "spec_id": spec_id,
        "app": spec["app"],
        "app_rule": APPS[spec["app"]],
        "phenomena": {p: PHENOMENA[p] for p in spec["phenomena"]},
        "length": LENGTHS[spec["length"]],
        "speaker": spec["speaker"],
        "role": spec["role"],
        "topic": spec["topic"],
        "tone": spec["tone"],
        "with_previous_text": spec["with_previous_text"],
    }


def gen_user_prompt(specs: list[dict]) -> str:
    body = [spec_for_prompt(i, s) for i, s in enumerate(specs)]
    return "Specs:\n" + json.dumps(body, ensure_ascii=False, indent=1)


# ── Validation ───────────────────────────────────────────────────────────────

FILLERS = {"um", "uh", "erm", "er", "uhm", "umm", "hmm", "mm"}
SPOKEN_FORBIDDEN = re.compile(r"[0-9@$%#&*/\\:;,.!?\"()\[\]{}<>=+_|~^`]")
WORD = re.compile(r"[a-z']+")


def words(text: str) -> list[str]:
    return WORD.findall(text.lower())


def validate(item: dict, spec: dict) -> str | None:
    """Return a reason string if the item is unusable, else None."""
    for key in ("spoken", "asr", "verbatim", "clean", "formatted"):
        if not isinstance(item.get(key), str) or not item[key].strip():
            return f"missing {key}"
    if not isinstance(item.get("vocab", []), list) or not isinstance(item.get("previous_text", ""), str):
        return "bad vocab/previous_text type"
    spoken = item["spoken"]
    if spoken != spoken.lower():
        return "spoken not lowercase"
    if SPOKEN_FORBIDDEN.search(spoken):
        return "spoken has digits/symbols/punctuation"
    n_spoken = len(words(spoken))
    if n_spoken < 2:
        return "spoken too short"
    if set(words(item["clean"])) & FILLERS:
        return "filler left in clean"
    if has_spelled_written_forms(item):
        return "date/time/amount left as words in a target"
    phen = spec["phenomena"]
    if any(p in phen for p in ("fillers", "false_start", "repetition", "self_correction")):
        if item["clean"].strip() == item["verbatim"].strip():
            return "clean identical to verbatim despite disfluencies"
    if len(words(item["clean"])) > n_spoken + 6:
        return "clean longer than spoken (invented content?)"
    if len(words(item["formatted"])) > n_spoken + 10:
        return "formatted much longer than spoken"
    # Formatting may merge words into identifiers (getUserById) but must not
    # drop what the speaker said.
    if spec["app"] not in ("code_editor", "terminal"):
        clean_w, fmt_w = words(item["clean"]), set(words(item["formatted"]))
        if clean_w and sum(w in fmt_w for w in clean_w) / len(clean_w) < 0.9:
            return "formatted dropped words from clean"
    elif spec["app"] == "terminal" and len(words(item["formatted"])) < 0.5 * len(words(item["clean"])):
        return "terminal command much shorter than what was said"
    if "vocab_mishear" in phen:
        vocab = [v for v in item.get("vocab", []) if isinstance(v, str) and v.strip()]
        if not vocab:
            return "vocab_mishear without vocab"
        low_clean = item["clean"].lower()
        if not any(v.lower() in low_clean for v in vocab):
            return "vocab term missing from clean"
    return None


# ── API plumbing ─────────────────────────────────────────────────────────────


class Budget:
    def __init__(self, max_usd: float | None):
        self.max_usd = max_usd
        self.usd = 0.0
        self.tokens_in = 0
        self.tokens_out = 0
        self.calls = 0
        self.reasoning_tokens = 0

    def add(self, provider: Provider, usage) -> None:
        pin, pout = provider.prices()
        tin = getattr(usage, "prompt_tokens", 0) or 0
        tout = getattr(usage, "completion_tokens", 0) or 0
        self.tokens_in += tin
        self.tokens_out += tout
        self.calls += 1
        self.usd += tin / 1e6 * pin + tout / 1e6 * pout

    @property
    def exhausted(self) -> bool:
        return self.max_usd is not None and self.usd >= self.max_usd

    def as_dict(self) -> dict:
        return {"usd": round(self.usd, 4), "tokens_in": self.tokens_in, "tokens_out": self.tokens_out, "calls": self.calls}


class RateLimiter:
    def __init__(self, rpm: int):
        self.interval = 60.0 / max(rpm, 1)
        self.next_at = 0.0
        self.lock = asyncio.Lock()

    async def wait(self) -> None:
        async with self.lock:
            now = time.monotonic()
            if self.next_at > now:
                await asyncio.sleep(self.next_at - now)
            self.next_at = max(now, self.next_at) + self.interval


class Client:
    """One provider: request params the endpoint rejects are dropped and retried."""

    def __init__(self, provider: Provider, rpm: int | None, model: str | None = None):
        from openai import AsyncOpenAI

        key = os.environ.get(provider.env, "").strip()
        if not key:
            sys.exit(f"{provider.env} is not set. Put it in {HERE / '.env'}.")
        self.p = provider
        self.model = model or provider.model
        self.client = AsyncOpenAI(api_key=key, base_url=provider.base_url, timeout=180, max_retries=0)
        self.limiter = RateLimiter(rpm or provider.default_rpm)
        self.params: dict = {"temperature": 1.0, "response_format": {"type": "json_object"}, **provider.extra}
        if provider.extra_body:
            self.params["extra_body"] = dict(provider.extra_body)

    async def chat_json(self, system: str, user: str, budget: Budget) -> dict:
        import openai

        delay = 5.0
        for attempt in range(8):
            await self.limiter.wait()
            try:
                resp = await self.client.chat.completions.create(
                    model=self.model,
                    messages=[{"role": "system", "content": system}, {"role": "user", "content": user}],
                    **self.params,
                )
            except openai.BadRequestError as e:
                msg = str(e).lower()
                dropped = [k for k in list(self.params) if k in msg or k.replace("_", " ") in msg]
                body = self.params.get("extra_body", {})
                if any(k in msg for k in body):
                    dropped.append("extra_body")
                if dropped:
                    for k in dropped:
                        self.params.pop(k, None)
                    print(f"  [{self.p.name}] endpoint rejected {dropped}; retrying without", file=sys.stderr)
                    continue
                raise
            except (openai.RateLimitError, openai.APIConnectionError, openai.APITimeoutError, openai.InternalServerError) as e:
                print(f"  [{self.p.name}] {type(e).__name__}; retry in {delay:.0f}s", file=sys.stderr)
                await asyncio.sleep(delay)
                delay = min(delay * 2, 120)
                continue
            if resp.usage:
                budget.add(self.p, resp.usage)
                details = getattr(resp.usage, "completion_tokens_details", None)
                reasoning = getattr(details, "reasoning_tokens", 0) or 0
                if reasoning:
                    budget.reasoning_tokens += reasoning
            text = resp.choices[0].message.content or ""
            try:
                return parse_json(text)
            except ValueError:
                print(f"  [{self.p.name}] unparseable JSON (attempt {attempt + 1})", file=sys.stderr)
                continue
        raise RuntimeError(f"{self.p.name}: giving up after retries")


def parse_json(text: str) -> dict:
    text = text.strip()
    if text.startswith("```"):
        text = re.sub(r"^```[a-z]*\n?|```$", "", text, flags=re.M).strip()
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        start, end = text.find("{"), text.rfind("}")
        if start >= 0 and end > start:
            return json.loads(text[start : end + 1])
        raise ValueError("no JSON object")


# ── Generation ───────────────────────────────────────────────────────────────


def append_jsonl(path: Path, rows: list[dict]) -> None:
    if not rows:
        return
    with path.open("a", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")


def read_jsonl(path: Path) -> list[dict]:
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


async def generate(args, provider_name: str, out: Path, n_calls: int, seed: int, rpm: int | None, max_usd: float | None) -> Budget:
    provider = PROVIDERS[provider_name]
    client = Client(provider, rpm, args.model)
    budget = Budget(max_usd)
    out.mkdir(parents=True, exist_ok=True)
    items_path, rejects_path = out / "items.jsonl", out / "rejects.jsonl"
    done_calls = {r["call"] for r in read_jsonl(items_path)} | {r["call"] for r in read_jsonl(rejects_path)}
    sem = asyncio.Semaphore(args.concurrency)
    write_lock = asyncio.Lock()
    stats = {"ok": 0, "rejected": 0}

    async def one_call(call: int) -> None:
        if call in done_calls or budget.exhausted:
            return
        rng = random.Random(f"{seed}:{call}")
        specs = [sample_spec(rng) for _ in range(args.per_call)]
        async with sem:
            if budget.exhausted:
                return
            try:
                data = await client.chat_json(GEN_SYSTEM, gen_user_prompt(specs), budget)
            except Exception as e:  # keep the run going; the call is retried on resume
                print(f"  call {call} failed: {e}", file=sys.stderr)
                return
        items = data.get("items") if isinstance(data, dict) else None
        ok_rows, bad_rows = [], []
        if not isinstance(items, list):
            bad_rows.append({"call": call, "reason": "no items list", "raw": data})
        else:
            for i, spec in enumerate(specs):
                item = items[i] if i < len(items) and isinstance(items[i], dict) else {}
                reason = validate(item, spec)
                row = {
                    "call": call, "idx": i, "teacher": f"{provider.name}:{client.model}",
                    "spec": spec, **{k: item.get(k) for k in ("previous_text", "vocab", "spoken", "asr", "verbatim", "clean", "formatted")},
                }
                (bad_rows if reason else ok_rows).append({**row, "reason": reason} if reason else row)
        async with write_lock:
            append_jsonl(items_path, ok_rows)
            append_jsonl(rejects_path, bad_rows)
            stats["ok"] += len(ok_rows)
            stats["rejected"] += len(bad_rows)
            total = stats["ok"] + stats["rejected"]
            if total and (total // args.per_call) % 10 == 0:
                print(f"  [{provider.name}] {stats['ok']} ok / {stats['rejected']} rejected, ${budget.usd:.3f}", file=sys.stderr)

    await asyncio.gather(*(one_call(c) for c in range(n_calls)))
    usage_path = out / "usage.json"
    prev = json.loads(usage_path.read_text()) if usage_path.exists() else {"usd": 0, "tokens_in": 0, "tokens_out": 0, "calls": 0}
    this = budget.as_dict()
    usage_path.write_text(json.dumps({k: round(prev[k] + this[k], 4) for k in this}, indent=2))
    print(f"[{provider.name}] this session: {stats['ok']} ok, {stats['rejected']} rejected, "
          f"{this['tokens_in']} in / {this['tokens_out']} out tokens ({budget.reasoning_tokens} reasoning), ${this['usd']:.4f}"
          + ("  (budget reached)" if budget.exhausted else ""))
    return budget


async def judge(args) -> None:
    provider = PROVIDERS[args.provider]
    client = Client(provider, args.rpm, args.model)
    budget = Budget(args.max_usd)
    items_path = Path(args.items_file)
    items = read_jsonl(items_path)
    out_path = items_path.with_name("judged.jsonl")
    done = {(r["call"], r["idx"]) for r in read_jsonl(out_path)}
    todo = [it for it in items if (it["call"], it["idx"]) not in done]
    sem = asyncio.Semaphore(args.concurrency)
    lock = asyncio.Lock()
    batch = 8
    counts = {"ok": 0, "bad": 0}

    async def one(chunk: list[dict]) -> None:
        if budget.exhausted:
            return
        payload = [{
            "id": j, "app_rule": APPS[it["spec"]["app"]], "phenomena": it["spec"]["phenomena"], "vocab": it["vocab"],
            "spoken": it["spoken"], "verbatim": it["verbatim"], "clean": it["clean"], "formatted": it["formatted"],
        } for j, it in enumerate(chunk)]
        async with sem:
            if budget.exhausted:
                return
            try:
                data = await client.chat_json(JUDGE_SYSTEM, json.dumps(payload, ensure_ascii=False), budget)
            except Exception as e:
                print(f"  judge failed: {e}", file=sys.stderr)
                return
        results = {r.get("id"): r for r in data.get("results", []) if isinstance(r, dict)}
        rows = []
        for j, it in enumerate(chunk):
            r = results.get(j)
            if r is None:
                continue
            rows.append({**it, "judge": f"{provider.name}:{client.model}", "judge_ok": bool(r.get("ok")), "judge_reason": r.get("reason", "")})
            counts["ok" if r.get("ok") else "bad"] += 1
        async with lock:
            append_jsonl(out_path, rows)

    await asyncio.gather(*(one(todo[i : i + batch]) for i in range(0, len(todo), batch)))
    print(f"judged {counts['ok'] + counts['bad']}: {counts['ok']} ok, {counts['bad']} failed, ${budget.usd:.4f} -> {out_path}")


# ── Dataset build ────────────────────────────────────────────────────────────

# Dates, times, money and measured amounts must be in written form in every
# target; small counts ("two weeks") may stay as words.
_NW = r"(?:one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|forty|fifty|sixty|seventy|eighty|ninety)"
SPELLED_WRITTEN_FORM = re.compile(
    r"\b(?:" + _NW + r"(?:[ -]" + _NW + r")*\s+(?:hundred|thousand|million|percent|dollars?|euros?|pounds|rupees|milligrams?|mg|kilograms?|kg|miles|km|o'clock|a\.?m\.?|p\.?m\.?)"
    r"|(?:fifth|sixth|seventh|eighth|ninth|tenth|eleventh|twelfth|thirteenth|fourteenth|fifteenth|sixteenth|seventeenth|eighteenth|nineteenth|twentieth|thirtieth)\s+of\s+[A-Z]"
    r"|(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+(?:first|second|third|" + _NW + r"|\w+th)\b"
    r"|\b(?:one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve)\s+(?:thirty|fifteen|forty[ -]five|o'clock)\b)",
    re.I,
)


def has_spelled_written_forms(item: dict) -> bool:
    return any(SPELLED_WRITTEN_FORM.search(item.get(k) or "") for k in ("verbatim", "clean", "formatted"))


ENGINE_WEIGHTS = {"granite": 0.4, "whisper": 0.3, "qwen3": 0.3}
LEVELS = ("verbatim", "clean", "formatted")


def make_prompt(engine: str, level: str, app: str, vocab: list[str], prev: str, text: str) -> str:
    tags = f"<engine={engine}> <level={level}> <app={app}>"
    if vocab:
        tags += " <vocab=" + "; ".join(vocab) + ">"
    parts = [tags]
    if prev:
        parts.append(f"<prev>{prev}</prev>")
    parts.append(f"<text>{text}</text>")
    return "\n".join(parts)


def build(args) -> None:
    rng = random.Random(args.seed)
    items: list[dict] = []
    for run in args.runs:
        run = Path(run)
        judged = run / "judged.jsonl"
        if judged.exists() and not args.ignore_judge:
            rows = read_jsonl(judged)
            items += [r for r in rows if r.get("judge_ok")]
            print(f"{run}: {sum(r.get('judge_ok', False) for r in rows)}/{len(rows)} judged ok")
        else:
            rows = read_jsonl(run / "items.jsonl")
            items += rows
            print(f"{run}: {len(rows)} items (not judged)")

    before = len(items)
    items = [it for it in items if not has_spelled_written_forms(it)]
    print(f"dropped {before - len(items)} items with dates/times/amounts left as words")

    # Dedupe on the cleaned text.
    seen, unique = set(), []
    for it in items:
        h = hashlib.sha1(re.sub(r"\W+", " ", it["clean"].lower()).strip().encode()).hexdigest()
        if h not in seen:
            seen.add(h)
            unique.append(it)
    vocab_pool = sorted({v for it in unique for v in (it.get("vocab") or []) if isinstance(v, str)})

    rng.shuffle(unique)
    n = len(unique)
    n_test = max(1, int(n * args.test_frac)) if n > 20 else 0
    n_val = max(1, int(n * args.val_frac)) if n > 20 else 0
    splits = {"test": unique[:n_test], "val": unique[n_test : n_test + n_val], "train": unique[n_test + n_val :]}

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    for split, rows in splits.items():
        records = []
        for it in rows:
            vocab = [v for v in (it.get("vocab") or []) if isinstance(v, str)]
            # Distractor terms teach the model to leave unrelated words alone.
            if vocab_pool and rng.random() < 0.4:
                vocab = vocab + rng.sample(vocab_pool, k=min(len(vocab_pool), rng.randint(1, 3)))
                rng.shuffle(vocab)
            for level in LEVELS:
                engine = rng.choices(list(ENGINE_WEIGHTS), weights=list(ENGINE_WEIGHTS.values()), k=1)[0]
                # Verbatim keeps every spoken word, but punctuating recognizers
                # drop fillers; from their output the words can't be recovered,
                # so verbatim always trains on the raw (Granite-style) input.
                if level == "verbatim":
                    engine = "granite"
                src = it["spoken"] if engine == "granite" else it["asr"]
                records.append({
                    "prompt": make_prompt(engine, level, it["spec"]["app"], vocab, it.get("previous_text") or "", src),
                    "completion": it[level],
                    "meta": {"teacher": it["teacher"], "engine": engine, "level": level, **{k: it["spec"][k] for k in ("app", "phenomena", "length")}},
                })
        path = out / f"{split}.jsonl"
        path.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in records), encoding="utf-8")
        print(f"{split}: {len(rows)} items -> {len(records)} records -> {path}")


# ── Bake-off ─────────────────────────────────────────────────────────────────


def distinct2(texts: list[str]) -> float:
    grams, total = set(), 0
    for t in texts:
        w = words(t)
        pairs = list(zip(w, w[1:]))
        grams.update(pairs)
        total += len(pairs)
    return len(grams) / total if total else 0.0


async def bakeoff(args) -> None:
    out_root = Path(args.out)
    names = [n.strip() for n in args.providers.split(",") if n.strip()]
    rows = []
    for name in names:
        if not os.environ.get(PROVIDERS[name].env):
            print(f"skipping {name}: {PROVIDERS[name].env} not set")
            continue
        out = out_root / name
        t0 = time.monotonic()
        budget = await generate(args, name, out, args.calls, args.seed, None, args.max_usd)
        items = read_jsonl(out / "items.jsonl")
        rejects = read_jsonl(out / "rejects.jsonl")
        total = len(items) + len(rejects)
        rows.append({
            "provider": f"{name}:{args.model or PROVIDERS[name].model}",
            "accepted": f"{len(items)}/{total}",
            "usd_per_1k_items": round(budget.usd / max(len(items), 1) * 1000, 3),
            "seconds": round(time.monotonic() - t0, 1),
            "distinct2_spoken": round(distinct2([i["spoken"] for i in items]), 3),
            "top_reject": max({r.get("reason") for r in rejects}, key=[r.get("reason") for r in rejects].count) if rejects else "",
        })
    print("\nprovider | accepted | $/1k items | seconds | distinct-2 | most common reject")
    for r in rows:
        print(" | ".join(str(v) for v in r.values()))
    # Same specs for every provider (same seed), so the review file lines up.
    review = ["# FlowScribe v3 teacher bake-off\n"]
    per = {n: read_jsonl(out_root / n / "items.jsonl") for n in names if (out_root / n / "items.jsonl").exists()}
    keys = sorted({(i["call"], i["idx"]) for items in per.values() for i in items})[: args.review]
    for key in keys:
        any_item = next(i for items in per.values() for i in items if (i["call"], i["idx"]) == key)
        review.append(f"\n## spec {key}: {any_item['spec']['app']} · {', '.join(any_item['spec']['phenomena'])}\n")
        for n, items in per.items():
            it = next((i for i in items if (i["call"], i["idx"]) == key), None)
            if it:
                review.append(f"\n**{n}**\n- spoken: {it['spoken']}\n- clean: {it['clean']}\n- formatted: {it['formatted']!r}\n")
    (out_root / "review.md").write_text("".join(review), encoding="utf-8")
    print(f"\nside-by-side samples: {out_root / 'review.md'}")


# ── CLI ──────────────────────────────────────────────────────────────────────


def main() -> None:
    try:
        from dotenv import load_dotenv

        load_dotenv(HERE / ".env")
    except ImportError:
        pass

    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    def common(p):
        p.add_argument("--per-call", type=int, default=8, help="dictations per API call")
        p.add_argument("--concurrency", type=int, default=4)
        p.add_argument("--seed", type=int, default=1)
        p.add_argument("--model", default=None, help="override the provider's default model id")

    sub.add_parser("dry-run", help="print one generation prompt; no API calls")

    p = sub.add_parser("run", help="generate items with one provider")
    p.add_argument("--provider", choices=PROVIDERS, required=True)
    p.add_argument("--items", type=int, required=True, help="target number of items (before rejects)")
    p.add_argument("--max-usd", type=float, default=None, help="stop once this much is spent this session")
    p.add_argument("--rpm", type=int, default=None, help="requests per minute cap")
    p.add_argument("--out", required=True)
    common(p)

    p = sub.add_parser("bakeoff", help="same specs through several providers, side by side")
    p.add_argument("--providers", default="gemini,deepseek,openai")
    p.add_argument("--calls", type=int, default=3)
    p.add_argument("--max-usd", type=float, default=0.25, help="per provider")
    p.add_argument("--review", type=int, default=12, help="specs to show in review.md")
    p.add_argument("--out", default=str(HERE / "out" / "bakeoff"))
    common(p)

    p = sub.add_parser("judge", help="second-opinion check of generated items")
    p.add_argument("--provider", choices=PROVIDERS, required=True)
    p.add_argument("--items-file", required=True)
    p.add_argument("--max-usd", type=float, default=None)
    p.add_argument("--rpm", type=int, default=None)
    p.add_argument("--concurrency", type=int, default=4)
    p.add_argument("--model", default=None)

    p = sub.add_parser("build", help="merge runs into train/val/test records")
    p.add_argument("runs", nargs="+")
    p.add_argument("--out", required=True)
    p.add_argument("--val-frac", type=float, default=0.03)
    p.add_argument("--test-frac", type=float, default=0.03)
    p.add_argument("--seed", type=int, default=7)
    p.add_argument("--ignore-judge", action="store_true")

    args = ap.parse_args()
    if args.cmd == "dry-run":
        rng = random.Random(0)
        specs = [sample_spec(rng) for _ in range(3)]
        print("=== system ===\n" + GEN_SYSTEM + "\n\n=== user ===\n" + gen_user_prompt(specs))
    elif args.cmd == "run":
        calls = -(-args.items // args.per_call)
        asyncio.run(generate(args, args.provider, Path(args.out), calls, args.seed, args.rpm, args.max_usd))
    elif args.cmd == "bakeoff":
        asyncio.run(bakeoff(args))
    elif args.cmd == "judge":
        asyncio.run(judge(args))
    elif args.cmd == "build":
        build(args)


if __name__ == "__main__":
    main()
