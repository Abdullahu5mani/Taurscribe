import unittest
from asr_quality import word_error_rate


class WordErrorRateTests(unittest.TestCase):
    def test_case_punctuation_and_curly_apostrophes_are_normalized(self):
        self.assertEqual(word_error_rate("I'm ready, José!", "I’M READY JOSÉ."), 0)

    def test_missing_word_counts_as_one_deletion(self):
        self.assertEqual(word_error_rate("one two three four", "one three four"), 0.25)

    def test_extra_word_counts_as_one_insertion(self):
        self.assertEqual(word_error_rate("one two three four", "one extra two three four"), 0.25)

    def test_changed_word_counts_as_one_substitution(self):
        self.assertEqual(word_error_rate("one two three four", "one wrong three four"), 0.25)

    def test_reordered_words_fail_despite_complete_word_presence(self):
        self.assertGreater(word_error_rate("one two three four", "four three two one"), 0.5)

    def test_repeated_hallucinations_can_exceed_one_hundred_percent(self):
        self.assertEqual(word_error_rate("one", "one one one"), 2)

    def test_empty_and_unrelated_output_cannot_pass_a_spoken_reference(self):
        self.assertEqual(word_error_rate("one two three", ""), 1)
        self.assertEqual(word_error_rate("one two three", "other unrelated words"), 1)

    def test_silence_reference_still_penalizes_hallucinated_speech(self):
        self.assertEqual(word_error_rate("", ""), 0)
        self.assertEqual(word_error_rate("", "invented words"), 2)
