"""Diversity axes for FlowScribe v3 synthetic data.

Every generation call samples a handful of *specs* from these lists, so the
teacher model is steered toward varied apps, people, topics and speech
phenomena instead of drifting into its favourite examples.
"""

import random

# Where the text is going. `rule` is what "formatted" output must follow; it
# is shown to the teacher and becomes the app tag the student model sees.
APPS = {
    "email": "An email body. Greeting on its own line, a blank line, body paragraphs, sign-off on its own line only if spoken. No subject line.",
    "chat": "A chat message (Slack, Teams, iMessage, Discord). Casual and short; normal sentence casing; no sign-off; a one-line message may drop its final period.",
    "notes": "Personal notes. Concise; spoken list commands become '- ' bullets; headings only if spoken.",
    "document": "Long-form writing (a doc, report or essay). Full sentences in paragraphs.",
    "code_editor": "An IDE, code comment or commit message. Identifiers, file names, symbols and operators in code form (getUserById, user_id, index.ts, !=); prose stays prose. Never add comment markers (//, #) or labels like TODO unless spoken.",
    "terminal": "A shell command line. The speaker dictates one command word by word (e.g. 'git commit dash m quote fix login quote'); formatted is that exact command (git commit -m \"fix login\"), no trailing period. Never shorten or invent commands from a description.",
    "ai_prompt": "A prompt typed to an AI assistant. Clear prose; keep spoken lists as lists.",
    "search": "A search or address bar query. Short, no trailing punctuation; URLs written as URLs.",
    "calendar_task": "A calendar event or to-do title. Short, title-like, dates and times in written form.",
    "generic": "Unknown app. Plain, well-punctuated prose.",
}
APP_WEIGHTS = {
    "email": 14, "chat": 18, "notes": 10, "document": 10, "code_editor": 9,
    "terminal": 4, "ai_prompt": 12, "search": 5, "calendar_task": 5, "generic": 13,
}

# Speech phenomena. Each spec gets 1-3 of these (or "plain").
PHENOMENA = {
    "fillers": "um, uh, like, you know, I mean used as filler",
    "false_start": "starts a phrase, abandons it and restarts differently",
    "repetition": "stutters or repeats words (the the, I I think)",
    "self_correction": "corrects themself mid-sentence (no wait, sorry, I mean, actually make that, scratch that) — the output keeps only the corrected version",
    "numbers": "quantities, prices, percentages, phone numbers or versions said out loud",
    "dates_times": "dates, times, days or durations said out loud",
    "email_url": "an email address, URL, @handle or file path said out loud (at, dot, slash)",
    "spoken_punctuation": "deliberately says punctuation (comma, period, question mark, open quote, colon)",
    "format_commands": "says layout commands (new line, new paragraph, bullet point, numbered list, next item)",
    "vocab_mishear": "uses a custom term (product, person, company or jargon from `vocab`) that a speech recognizer mis-hears as a sound-alike",
    "literal_trigger": "uses words like comma, period, new line, no wait, scratch that or actually in their ordinary meaning — they must NOT be treated as commands or corrections",
    "plain": "clean, fluent speech that needs almost no fixing (only punctuation and casing)",
}
PHENOMENA_WEIGHTS = {
    "fillers": 14, "false_start": 8, "repetition": 7, "self_correction": 14,
    "numbers": 11, "dates_times": 8, "email_url": 6, "spoken_punctuation": 6,
    "format_commands": 7, "vocab_mishear": 8, "literal_trigger": 5, "plain": 9,
}

LENGTHS = {
    "short": "4-15 spoken words",
    "medium": "15-45 spoken words",
    "long": "45-110 spoken words",
}
LENGTH_WEIGHTS = {"short": 35, "medium": 45, "long": 20}

SPEAKERS = [
    "US English", "UK English", "Indian English", "Canadian English", "Australian English",
    "Irish English", "South African English", "Nigerian English", "Filipino English",
    "non-native speaker (first language Spanish)", "non-native speaker (first language Mandarin)",
    "non-native speaker (first language Arabic)", "non-native speaker (first language Hindi)",
    "non-native speaker (first language French)", "non-native speaker (first language Urdu)",
]

ROLES = [
    "software engineer", "nurse", "high school teacher", "sales rep", "university student",
    "startup founder", "lawyer", "accountant", "construction project manager", "product designer",
    "real estate agent", "customer support agent", "researcher", "journalist", "retired person",
    "small restaurant owner", "data scientist", "HR manager", "doctor", "pharmacist",
    "mechanic", "parent organizing family logistics", "gamer", "marketing manager", "DevOps engineer",
    "freelance writer", "logistics coordinator", "electrician", "therapist", "PhD student",
    "insurance adjuster", "musician", "event planner", "social media manager", "IT admin",
]

TOPICS = [
    "scheduling a meeting", "a bug report", "a code review comment", "a shopping list",
    "travel plans", "a customer complaint reply", "a project status update", "a recipe",
    "an invoice or payment", "a doctor's appointment", "a school assignment", "a job application",
    "a birthday party", "a lease or rental issue", "a product launch", "a server outage",
    "a workout plan", "a book or film opinion", "a sales follow-up", "a lab experiment",
    "a legal deadline", "a home repair", "a budget spreadsheet", "a team offsite",
    "a pull request description", "a support ticket", "a car problem", "a wedding",
    "a research summary request", "a grocery order", "a flight delay", "a hiring decision",
    "a database migration", "a marketing email", "a thank-you note", "a dentist reminder",
    "a conference talk", "a vacation request", "a lost package", "a vet visit",
]

# Code editors and terminals get technical subjects most of the time.
TECH_TOPICS = [
    "a failing unit test", "renaming a function", "a git branch cleanup", "a Docker build error",
    "an API endpoint change", "a TODO comment", "a SQL query", "installing a package",
    "a React component bug", "a CI pipeline failure", "an environment variable", "a Python script",
    "a regex fix", "a log file search", "a config file edit", "deploying to staging",
    "a type error", "a performance fix", "a code review comment", "a Kubernetes pod crash",
]

TONES = ["neutral", "casual", "friendly", "formal", "terse", "rambling", "hurried", "polite"]


def _weighted(rng: random.Random, weights: dict) -> str:
    keys = list(weights)
    return rng.choices(keys, weights=[weights[k] for k in keys], k=1)[0]


def sample_spec(rng: random.Random) -> dict:
    app = _weighted(rng, APP_WEIGHTS)
    n = rng.choices([1, 2, 3], weights=[45, 40, 15], k=1)[0]
    phenomena: list[str] = []
    while len(phenomena) < n:
        p = _weighted(rng, PHENOMENA_WEIGHTS)
        if p not in phenomena:
            phenomena.append(p)
    if "plain" in phenomena:
        phenomena = ["plain"]
    # Terminal/search inputs are short by nature.
    length = "short" if app in ("terminal", "search", "calendar_task") else _weighted(rng, LENGTH_WEIGHTS)
    return {
        "app": app,
        "phenomena": phenomena,
        "length": length,
        "speaker": rng.choice(SPEAKERS),
        "role": rng.choice(ROLES),
        "topic": rng.choice(TECH_TOPICS if app in ("code_editor", "terminal") and rng.random() < 0.8 else TOPICS),
        "tone": rng.choice(TONES),
        "with_previous_text": rng.random() < 0.25,
    }
