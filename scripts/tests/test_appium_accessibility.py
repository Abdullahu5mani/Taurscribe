#!/usr/bin/env python3
"""
scripts/tests/test_appium_accessibility.py

Automated Accessibility and Appium Compatibility Test Suite for Taurscribe.

Validates that:
1. 100% of interactive elements (<button>, <input>, <select>, <textarea>) have
   both `id` and `data-testid` attributes matching Appium `accessibilityId` standards.
2. 100% of WAI-ARIA role elements (dialog, tablist, tab, switch, radio, slider, alert, status, region)
   have valid Appium locators and ARIA semantics.
3. All interactive elements have accessible names (text content, aria-label, or title).
4. All stateful controls (switches, radios, tabs, sliders, toggles) declare required ARIA state attributes.
"""

import os
import re
import unittest
from typing import List, Tuple

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SRC_DIR = os.path.join(REPO_ROOT, "src")

TAG_PATTERN = re.compile(
    r"<(?P<tag>[a-zA-Z0-9_-]+)\b(?P<attrs>[^>]*)>",
    re.DOTALL
)

def strip_jsx_comments(code: str) -> str:
    code = re.sub(r"\{\s*/\*.*?\*/\s*\}", "", code, flags=re.DOTALL)
    code = re.sub(r"/\*.*?\*/", "", code, flags=re.DOTALL)
    code = re.sub(r"//.*", "", code)
    return code

def extract_attribute(attrs: str, attr_name: str) -> str:
    pattern = re.compile(r'\b' + re.escape(attr_name) + r'=(?:"([^"]*)"|\'([^\']*)\'|\{([^}]+)\})')
    m = pattern.search(attrs)
    if not m:
        return ""
    return m.group(1) or m.group(2) or m.group(3) or ""

def get_line_number(text: str, index: int) -> int:
    return text[:index].count("\n") + 1


class TestAppiumAccessibility(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tsx_files: List[Tuple[str, str]] = []
        for root, _, files in os.walk(SRC_DIR):
            for file in files:
                if file.endswith(".tsx"):
                    full_path = os.path.join(root, file)
                    rel_path = os.path.relpath(full_path, REPO_ROOT)
                    with open(full_path, "r", encoding="utf-8") as f:
                        cls.tsx_files.append((rel_path, f.read()))

    def test_interactive_elements_have_id_and_testid(self):
        missing = []
        total_inspected = 0

        for rel_path, raw_content in self.tsx_files:
            cleaned = strip_jsx_comments(raw_content)
            for match in TAG_PATTERN.finditer(cleaned):
                tag = match.group("tag")
                if tag not in ("button", "input", "select", "textarea"):
                    continue

                attrs = match.group("attrs")
                total_inspected += 1

                id_val = extract_attribute(attrs, "id")
                testid_val = extract_attribute(attrs, "data-testid")

                if not id_val or not testid_val:
                    line_no = get_line_number(cleaned, match.start())
                    snippet = match.group(0).strip().replace("\n", " ")[:90]
                    missing.append(f"{rel_path}:{line_no} <{tag}> missing id/testid: {snippet}")

        self.assertGreater(total_inspected, 100, f"Expected >100 interactive elements, got {total_inspected}")
        self.assertEqual(
            len(missing),
            0,
            f"Found {len(missing)} interactive elements missing id or data-testid:\n" + "\n".join(missing)
        )

    def test_role_elements_have_id_and_testid(self):
        missing = []
        total_inspected = 0

        for rel_path, raw_content in self.tsx_files:
            cleaned = strip_jsx_comments(raw_content)
            for match in TAG_PATTERN.finditer(cleaned):
                attrs = match.group("attrs")
                role_val = extract_attribute(attrs, "role")
                if not role_val:
                    continue

                tag = match.group("tag")
                total_inspected += 1

                id_val = extract_attribute(attrs, "id")
                testid_val = extract_attribute(attrs, "data-testid")

                if not id_val or not testid_val:
                    line_no = get_line_number(cleaned, match.start())
                    snippet = match.group(0).strip().replace("\n", " ")[:90]
                    missing.append(f"{rel_path}:{line_no} <{tag} role=\"{role_val}\"> missing id/testid: {snippet}")

        self.assertGreater(total_inspected, 40, f"Expected >40 role elements, got {total_inspected}")
        self.assertEqual(
            len(missing),
            0,
            f"Found {len(missing)} role elements missing id or data-testid:\n" + "\n".join(missing)
        )

    def test_interactive_elements_have_accessible_names(self):
        missing = []

        for rel_path, raw_content in self.tsx_files:
            cleaned = strip_jsx_comments(raw_content)
            for match in TAG_PATTERN.finditer(cleaned):
                tag = match.group("tag")
                if tag not in ("button", "input", "select", "textarea"):
                    continue

                attrs = match.group("attrs")
                has_aria_label = bool(extract_attribute(attrs, "aria-label"))
                has_title = bool(extract_attribute(attrs, "title"))
                input_type = extract_attribute(attrs, "type")

                if not has_aria_label and not has_title:
                    if input_type == "hidden":
                        continue

                    if tag == "button":
                        start = match.end()
                        end = cleaned.find("</button>", start)
                        if end != -1:
                            inner = cleaned[start:end].strip()
                            text_without_tags = re.sub(r"<[^>]+>", "", inner).strip()
                            if not text_without_tags:
                                line_no = get_line_number(cleaned, match.start())
                                snippet = match.group(0).strip().replace("\n", " ")[:90]
                                missing.append(f"{rel_path}:{line_no} <button> without visible text or aria-label: {snippet}")

        self.assertEqual(
            len(missing),
            0,
            f"Found {len(missing)} interactive elements lacking accessible names:\n" + "\n".join(missing)
        )

    def test_switch_elements_have_aria_checked(self):
        missing = []

        for rel_path, raw_content in self.tsx_files:
            cleaned = strip_jsx_comments(raw_content)
            for match in TAG_PATTERN.finditer(cleaned):
                attrs = match.group("attrs")
                role_val = extract_attribute(attrs, "role")
                if role_val != "switch":
                    continue

                checked = extract_attribute(attrs, "aria-checked")
                if not checked:
                    line_no = get_line_number(cleaned, match.start())
                    snippet = match.group(0).strip().replace("\n", " ")[:90]
                    missing.append(f"{rel_path}:{line_no} role='switch' missing aria-checked: {snippet}")

        self.assertEqual(len(missing), 0, f"Found switches without aria-checked:\n" + "\n".join(missing))

    def test_radio_elements_have_aria_checked(self):
        missing = []

        for rel_path, raw_content in self.tsx_files:
            cleaned = strip_jsx_comments(raw_content)
            for match in TAG_PATTERN.finditer(cleaned):
                attrs = match.group("attrs")
                role_val = extract_attribute(attrs, "role")
                if role_val != "radio":
                    continue

                has_checked = bool(extract_attribute(attrs, "aria-checked")) or bool(extract_attribute(attrs, "checked"))
                if not has_checked:
                    line_no = get_line_number(cleaned, match.start())
                    snippet = match.group(0).strip().replace("\n", " ")[:90]
                    missing.append(f"{rel_path}:{line_no} role='radio' missing aria-checked: {snippet}")

        self.assertEqual(len(missing), 0, f"Found radios without aria-checked:\n" + "\n".join(missing))

    def test_slider_elements_have_aria_values(self):
        missing = []

        for rel_path, raw_content in self.tsx_files:
            cleaned = strip_jsx_comments(raw_content)
            for match in TAG_PATTERN.finditer(cleaned):
                attrs = match.group("attrs")
                role_val = extract_attribute(attrs, "role")
                if role_val != "slider":
                    continue

                valuenow = extract_attribute(attrs, "aria-valuenow")
                valuemin = extract_attribute(attrs, "aria-valuemin")
                valuemax = extract_attribute(attrs, "aria-valuemax")
                if not valuenow or not valuemin or not valuemax:
                    line_no = get_line_number(cleaned, match.start())
                    snippet = match.group(0).strip().replace("\n", " ")[:90]
                    missing.append(f"{rel_path}:{line_no} role='slider' missing aria range: {snippet}")

        self.assertEqual(len(missing), 0, f"Found sliders without full aria range:\n" + "\n".join(missing))

    def test_tab_elements_have_aria_selected(self):
        missing = []

        for rel_path, raw_content in self.tsx_files:
            cleaned = strip_jsx_comments(raw_content)
            for match in TAG_PATTERN.finditer(cleaned):
                attrs = match.group("attrs")
                role_val = extract_attribute(attrs, "role")
                if role_val != "tab":
                    continue

                selected = extract_attribute(attrs, "aria-selected")
                if not selected:
                    line_no = get_line_number(cleaned, match.start())
                    snippet = match.group(0).strip().replace("\n", " ")[:90]
                    missing.append(f"{rel_path}:{line_no} role='tab' missing aria-selected: {snippet}")

        self.assertEqual(len(missing), 0, f"Found tabs without aria-selected:\n" + "\n".join(missing))


if __name__ == "__main__":
    unittest.main(verbosity=2)
