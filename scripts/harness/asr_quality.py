"""Reference-based accuracy checks; independent of models and UI navigation."""
import re


def word_error_rate(reference: str, actual: str) -> float:
    def words(text):
        return re.findall(r"[^\W_]+(?:'[^\W_]+)*", text.casefold().replace("’", "'"))

    expected, found = words(reference), words(actual)
    previous = list(range(len(found) + 1))
    for i, word in enumerate(expected):
        row = [i + 1]
        for j, actual_word in enumerate(found):
            row.append(min(previous[j] + (word != actual_word), previous[j + 1] + 1, row[j] + 1))
        previous = row
    return previous[-1] / max(1, len(expected))
