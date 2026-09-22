#!/usr/bin/env python3
"""Decider Agent for Ultrafast Meeting Automation.

Inspired by Jev Ultrafast and Mapika/decider-2b:
Consumes an indexed DOM observation table and outputs one of:
  - CLICK <target_index>
  - TYPE_TEXT <target_index> <text>
  - WAIT
  - DONE

Supports:
1. Native Mapika/decider-2b via `decider` or `transformers` if installed.
2. Local OpenAI-compatible server (Ollama, LM Studio, llama-server).
3. Zero-dependency heuristic System-1 classifier (default fallback when no runner is active).
"""

from __future__ import annotations
import json
import os
import re
import urllib.request
import urllib.error
from typing import Any, Dict, List, Optional


class DeciderAction:
    def __init__(self, operation: str, target: Optional[int] = None, text: Optional[str] = None, reason: str = ""):
        self.operation = operation.upper()  # CLICK, TYPE_TEXT, WAIT, DONE
        self.target = target
        self.text = text
        self.reason = reason

    def to_dict(self) -> Dict[str, Any]:
        return {
            "operation": self.operation,
            "target": self.target,
            "text": self.text,
            "reason": self.reason,
        }

    def __repr__(self) -> str:
        tgt = f" [{self.target}]" if self.target is not None else ""
        txt = f" text='{self.text}'" if self.text is not None else ""
        return f"<DeciderAction {self.operation}{tgt}{txt} ({self.reason})>"


class PlanStep:
    """One click in a platform flow: click the control whose label matches `pattern`,
    optionally only while some text matching `when` is on screen."""

    def __init__(self, name: str, pattern: str, when: Optional[str] = None,
                 interrupt: bool = False, plain_only: bool = True):
        self.name = name
        self.pattern = re.compile(pattern, re.I)
        self.when = re.compile(when, re.I) if when else None
        self.interrupt = interrupt
        self.plain_only = plain_only


class MeetingDecider:
    """Decider engine for Google Meet, Teams, and WebRTC meeting lobbies."""

    def __init__(
        self,
        model_name: str = "Mapika/decider-2b",
        api_base: Optional[str] = None,
        api_key: Optional[str] = None,
        prefer_local_runner: bool = False,
    ):
        self.model_name = model_name
        # DECIDER_API_BASE (any OpenAI-compatible server) switches Decider from the
        # System-1 rules to a real model without code changes.
        self.api_base = api_base or os.environ.get("DECIDER_API_BASE") or "http://127.0.0.1:11434/v1"
        self.api_key = api_key or os.environ.get("DECIDER_API_KEY") or "local"
        self.model_name = os.environ.get("DECIDER_MODEL", model_name)
        self.prefer_local_runner = prefer_local_runner or bool(os.environ.get("DECIDER_API_BASE"))
        self._decider_lib = None
        self._check_available_runners()

    def _check_available_runners(self):
        # Check if python decider library is installed
        try:
            from decider.infer import Decider as NativeDecider
            self._decider_lib = NativeDecider(self.model_name)
            print(f"[DECIDER] Native Mapika/decider library loaded: {self.model_name}")
        except Exception:
            self._decider_lib = None

    def decide(self, snapshot: Dict[str, Any], goal: str = "Join meeting as guest", guest_name: str = "Dr. Sarah Chen") -> DeciderAction:
        """Determines the next optimal action given a DOM snapshot."""
        if snapshot.get("is_in_call"):
            return DeciderAction("DONE", reason="Call is already in progress")

        # If native Mapika decider or local LLM server is reachable and requested, use it
        if self._decider_lib:
            try:
                return self._decide_native(snapshot, goal, guest_name)
            except Exception as e:
                print(f"[DECIDER] Native decider failed ({e}), falling back to System-1 classifier")

        if self.prefer_local_runner and self._is_local_api_reachable():
            try:
                return self._decide_http(snapshot, goal, guest_name)
            except Exception as e:
                print(f"[DECIDER] Local API failed ({e}), falling back to System-1 classifier")

        # High-speed System-1 heuristic decider (0ms latency, zero dependencies).
        # A control that has been clicked twice without the screen moving on (a
        # toggle whose label never changes) is dropped so the flow can progress.
        attempts = self.__dict__.setdefault("_attempts", {})
        banned = {k for k, n in attempts.items() if n >= 2}
        pruned = dict(snapshot)
        pruned["elements"] = [e for e in snapshot.get("elements", []) if (e.get("role"), e.get("text")) not in banned]
        action = self._decide_system1(pruned, goal, guest_name)
        if action.operation == "CLICK":
            el = next((e for e in pruned["elements"] if e["index"] == action.target), None)
            if el:
                key = (el.get("role"), el.get("text"))
                attempts[key] = attempts.get(key, 0) + 1
        return action

    def _is_local_api_reachable(self) -> bool:
        try:
            req = urllib.request.Request(f"{self.api_base}/models", headers={"Authorization": f"Bearer {self.api_key}"})
            with urllib.request.urlopen(req, timeout=1.0) as resp:
                return resp.status == 200
        except Exception:
            return False

    def _decide_system1(self, snapshot: Dict[str, Any], goal: str, guest_name: str) -> DeciderAction:
        """Non-autoregressive deterministic System-1 classifier over the indexed element table."""
        elements: List[Dict[str, Any]] = snapshot.get("elements", [])
        if not elements:
            return DeciderAction("WAIT", reason="No interactive elements found on page")

        # 1. Check for pending host admission requests
        admit_req = snapshot.get("admit_request")
        if admit_req and admit_req.get("admit_target"):
            return DeciderAction("CLICK", target=admit_req["admit_target"], reason="Admit waiting participant")

        for el in elements:
            if re.search(r"\b(admit all|admit)\b", el["text"], re.I) and not el["disabled"]:
                return DeciderAction("CLICK", target=el["index"], reason="Admit participant into call")

        # 2. Check for popups/dialogs to dismiss
        dismiss_patterns = [
            r"continue without (microphone|audio|camera)",
            r"join on the web instead",
            r"continue on this browser",
            r"\b(got it|dismiss|close|not now|allow once)\b"
        ]
        for pat in dismiss_patterns:
            for el in elements:
                if re.search(pat, el["text"], re.I) and not el["disabled"]:
                    return DeciderAction("CLICK", target=el["index"], reason=f"Dismiss lobby dialog: '{el['text']}'")

        # 3. Name / display name first: platforms (Teams) disable Join until it is set
        name_input_patterns = [
            r"your name",
            r"display name",
            r"enter name",
            r"name",
            r"who is joining"
        ]
        for el in elements:
            if el["tag"] in ("input", "textarea") and el["type"] in ("text", ""):
                # If text matches name input and value does not match guest name
                if any(re.search(p, el["text"], re.I) for p in name_input_patterns) or el.get("placeholder"):
                    if not el.get("value") or el["value"] != guest_name:
                        return DeciderAction("TYPE_TEXT", target=el["index"], text=guest_name, reason="Enter guest display name")

        # If any single text input exists and is empty
        text_inputs = [e for e in elements if e["tag"] == "input" and e["type"] in ("text", "")]
        if len(text_inputs) == 1 and not text_inputs[0].get("value"):
            return DeciderAction("TYPE_TEXT", target=text_inputs[0]["index"], text=guest_name, reason="Fill name into only available text input")

        # 4. Mute microphone and camera if buttons are active/on
        # Anchored: "Unmute mic" must never match, or toggles flip forever.
        mute_mic_patterns = [
            r"^turn off microphone",
            r"^mute microphone",
            r"^mute mic\b",
            r"^turn off mic\b"
        ]
        # Only when the goal asks for it: a guest that must speak joins unmuted.
        wants_mic_muted = re.search(r"mute microphone", goal, re.I) is not None
        for el in elements if wants_mic_muted else []:
            if any(re.search(p, el["text"], re.I) for p in mute_mic_patterns):
                if not el["checked"] and not el["disabled"]:
                    return DeciderAction("CLICK", target=el["index"], reason="Mute microphone before joining")

        mute_cam_patterns = [
            r"^turn off camera",
            r"^turn off video",
            r"^disable camera",
            r"^stop video\b"
        ]
        for el in elements:
            if any(re.search(p, el["text"], re.I) for p in mute_cam_patterns):
                if not el["checked"] and not el["disabled"]:
                    return DeciderAction("CLICK", target=el["index"], reason="Turn off camera before joining")

        # 5. Click "Ask to join" / "Join now" / "Join meeting"
        join_patterns = [
            r"ask to join",
            r"join now",
            r"join meeting",
            r"join call",
            r"\bjoin\b"
        ]
        for pat in join_patterns:
            for el in elements:
                if re.search(pat, el["text"], re.I) and not el["disabled"]:
                    return DeciderAction("CLICK", target=el["index"], reason=f"Submit join request: '{el['text']}'")

        # 6. Fallback local room join
        local_join = next((e for e in elements if "join" in e["text"].lower() and not e["disabled"]), None)
        if local_join:
            return DeciderAction("CLICK", target=local_join["index"], reason="Click join control")

        return DeciderAction("WAIT", reason="Waiting for lobby to settle or admission approval")

    def _decide_http(self, snapshot: Dict[str, Any], goal: str, guest_name: str) -> DeciderAction:
        """Queries local OpenAI-compatible endpoint with indexed table."""
        prompt = (
            f"Goal: {goal}. Guest Name: '{guest_name}'.\n"
            f"Current observation table:\n{snapshot.get('table', '')}\n\n"
            "Choose next action. Respond ONLY with JSON: "
            '{"operation": "CLICK"|"TYPE_TEXT"|"WAIT"|"DONE", "target": <int or null>, "text": "<string or null>", "reason": "<string>"}'
        )
        payload = {
            "model": self.model_name,
            "messages": [
                {"role": "system", "content": "You are Jev Ultrafast decider. Pick element index and operation."},
                {"role": "user", "content": prompt}
            ],
            "temperature": 0.0,
            "max_tokens": 100,
        }
        req = urllib.request.Request(
            f"{self.api_base}/chat/completions",
            data=json.dumps(payload).encode("utf-8"),
            headers={"Content-Type": "application/json", "Authorization": f"Bearer {self.api_key}"}
        )
        with urllib.request.urlopen(req, timeout=3.0) as resp:
            data = json.loads(resp.read().decode("utf-8"))
            content = data["choices"][0]["message"]["content"]
            # Extract JSON block
            match = re.search(r"\{.*?\}", content, re.DOTALL)
            if match:
                res = json.loads(match.group(0))
                return DeciderAction(
                    operation=res.get("operation", "WAIT"),
                    target=res.get("target"),
                    text=res.get("text"),
                    reason=res.get("reason", "Local LLM decision")
                )
        return self._decide_system1(snapshot, goal, guest_name)

    # ── Host-side and verification skills ────────────────────────────────────
    #
    # These work on any indexed snapshot: snapshot.js over CDP, or ax_snapshot
    # over the macOS accessibility tree (desktop apps, the user's real Chrome).

    # Teams/Zoom append shortcuts to labels ("Leave (⌘+Shift+H)"), so allow a tail.
    # Zoom's host toolbar says just "End".
    LEAVE_PATTERN = re.compile(r"^(leave call|leave meeting|leave|hang up|end call|end)(\s*\(.*\))?$", re.I)

    def _enabled(self, snapshot: Dict[str, Any]) -> List[Dict[str, Any]]:
        return [e for e in snapshot.get("elements", []) if not e.get("disabled")]

    def _find_button(self, snapshot: Dict[str, Any], pattern: re.Pattern) -> Optional[Dict[str, Any]]:
        for el in self._enabled(snapshot):
            if "button" in el.get("role", "").lower() and pattern.search(el.get("text", "")):
                return el
        return None

    @staticmethod
    def _is_plain_button(el: Dict[str, Any]) -> bool:
        """A button that performs an action, as opposed to one that opens a menu/popup."""
        role = el.get("role", "").lower()
        return role in ("axbutton", "button", "menuitem", "axmenuitem")

    def has_leave_control(self, snapshot: Dict[str, Any]) -> bool:
        return self._find_button(snapshot, self.LEAVE_PATTERN) is not None

    def decide_host(self, snapshot: Dict[str, Any]) -> DeciderAction:
        """Host goal: be in the call and let waiting guests in."""
        if self.prefer_local_runner and self._is_local_api_reachable():
            try:
                return self._ask_http_action(
                    snapshot,
                    "You are the meeting host. Admit anyone waiting to join. If not yet in the "
                    "call, join it. If in the call with nobody waiting, answer DONE.",
                )
            except Exception as e:
                print(f"[DECIDER] Local API failed ({e}), falling back to System-1 classifier")

        # A real admit control is a plain button ("Admit <name>", "Admit all").
        # Popup buttons like Meet's "Admit one guest" only open the People panel;
        # clicking one again toggles the panel shut, so never treat it as the admit.
        admit_re = re.compile(r"^admit\b", re.I)
        for el in self._enabled(snapshot):
            if self._is_plain_button(el) and admit_re.search(el.get("text", "")):
                return DeciderAction("CLICK", target=el["index"], reason=f"Admit waiting guest: '{el['text']}'")

        texts = [e.get("text", "") for e in snapshot.get("elements", [])]
        panel_open = any(re.search(r"^(waiting to join|people panel is open|participants)$", t, re.I) for t in texts)
        knocking = any(re.search(r"waiting in (the )?lobby|wants to join|asking to join|waiting to be admitted|^admit \w+ guests?$", t, re.I)
                       for t in texts)
        if knocking and not panel_open:
            # Openers that lead to an Admit button, most specific first:
            #   Meet  — popup "Admit one guest"
            #   any   — the People/participants button
            # Never "View": in Teams that is the call layout menu.
            # (A Teams lobby toast is NOT an opener: pressing it just dismisses it.)
            for pattern, plain in ((r"^admit\b", False), (r"^people$", True)):
                for el in self._enabled(snapshot):
                    role = el.get("role", "").lower()
                    if "button" not in role:
                        continue
                    if plain is True and not self._is_plain_button(el):
                        continue
                    if plain is False and self._is_plain_button(el):
                        continue
                    if re.search(pattern, el.get("text", ""), re.I):
                        return DeciderAction("CLICK", target=el["index"], reason=f"Open pending join requests: '{el['text'][:60]}'")
        elif knocking and panel_open:
            return DeciderAction("WAIT", reason="Join request panel open, admit button not rendered yet")
        if self.has_leave_control(snapshot):
            return DeciderAction("DONE", reason="Host is in the call and nobody is waiting")
        # Never a bare "Join": Teams lists joinable chat meetings with that label.
        join = self._find_button(snapshot, re.compile(r"^(join now|start meeting)$", re.I))
        if join:
            return DeciderAction("CLICK", target=join["index"], reason=f"Host joins the call: '{join['text']}'")
        return DeciderAction("WAIT", reason="Host view still loading")

    def decide_plan(self, snapshot: Dict[str, Any], plan: List["PlanStep"],
                    done: Optional[re.Pattern] = None) -> DeciderAction:
        """Follows an ordered click plan, always acting on the furthest step visible.

        Picking the furthest step (not the first) means a re-rendered earlier
        control never drags the flow backwards. Steps flagged `interrupt` (popups,
        permission bubbles) win over everything whenever they appear.
        """
        if done and self._find_button(snapshot, done):
            return DeciderAction("DONE", reason=f"Goal reached ('{done.pattern}' visible)")
        texts = [e.get("text", "") for e in snapshot.get("elements", [])]
        ordered = [s for s in plan if s.interrupt] + list(reversed([s for s in plan if not s.interrupt]))
        for step in ordered:
            if step.when and not any(step.when.search(t) for t in texts):
                continue
            for el in self._enabled(snapshot):
                role = el.get("role", "").lower()
                if step.plain_only and not self._is_plain_button(el):
                    continue
                if any(k in role for k in ("button", "link", "checkbox", "menuitem", "switch")):
                    if step.pattern.search(el.get("text", "")):
                        return DeciderAction("CLICK", target=el["index"], reason=f"{step.name}: '{el['text']}'")
        return DeciderAction("WAIT", reason="No plan step visible yet")

    def decide_leave(self, snapshot: Dict[str, Any]) -> DeciderAction:
        """Goal: get out of the call."""
        leave = self._find_button(snapshot, self.LEAVE_PATTERN)
        if leave:
            return DeciderAction("CLICK", target=leave["index"], reason=f"Leave the call: '{leave['text']}'")
        return DeciderAction("DONE", reason="No leave control visible, call already left")

    def assess_meeting_detection(self, snapshot: Dict[str, Any], expected_platform: str) -> Dict[str, Any]:
        """Reads the Taurscribe window and reports whether it shows a detected meeting.

        Returns {detected, platform, evidence, reason, mode}.
        """
        if self.prefer_local_runner and self._is_local_api_reachable():
            try:
                verdict = self._ask_http_json(
                    snapshot,
                    "This is the Taurscribe desktop app. Does its UI currently show that a "
                    "meeting/call has been detected? Respond ONLY with JSON: "
                    '{"detected": true|false, "platform": "<name or null>", "evidence": <element index or null>, "reason": "<string>"}',
                )
                verdict["mode"] = "llm"
                return verdict
            except Exception as e:
                print(f"[DECIDER] Local API failed ({e}), falling back to System-1 classifier")

        # System-1: the MeetingHeaderPill button always carries an aria-label of
        # "Active call detected: <Platform>..." or "Recording <Platform> call..."
        pill = re.compile(r"^(active call detected: (?P<a>[^.]+)|recording (?P<b>.+?) call\b)", re.I)
        banner = re.compile(r"active call detected", re.I)
        for el in snapshot.get("elements", []):
            m = pill.search(el.get("text", ""))
            if m and "button" in el.get("role", "").lower():
                platform = (m.group("a") or m.group("b") or "").strip()
                return {
                    "detected": True,
                    "platform": platform,
                    "evidence": f"[{el['index']}] {el['role']} '{el['text']}'",
                    "reason": "Meeting header pill is visible",
                    "mode": "system1",
                }
        for el in snapshot.get("elements", []):
            if banner.search(el.get("text", "")):
                return {
                    "detected": True,
                    "platform": None,
                    "evidence": f"[{el['index']}] {el['role']} '{el['text']}'",
                    "reason": "Meeting banner visible without header pill",
                    "mode": "system1",
                }
        return {
            "detected": False,
            "platform": None,
            "evidence": None,
            "reason": f"No meeting pill or banner among {len(snapshot.get('elements', []))} elements",
            "mode": "system1",
        }

    # ── Taurscribe recording skills (read the app window) ────────────────────

    def find_record_call_control(self, snapshot: Dict[str, Any]) -> Optional[Dict[str, Any]]:
        """The meeting pill doubles as the 'start dual-channel recording' control."""
        for el in self._enabled(snapshot):
            if "button" in el.get("role", "").lower() and re.search(
                    r"click to start dual-channel recording|^record call$", el.get("text", ""), re.I):
                return el
        return None

    def assess_recording(self, snapshot: Dict[str, Any]) -> Dict[str, Any]:
        """Is Taurscribe recording right now, and of which call?"""
        for el in snapshot.get("elements", []):
            m = re.search(r"^recording (.+?) call\b|^rec: (.+)$", el.get("text", ""), re.I)
            if m:
                return {"recording": True, "platform": (m.group(1) or m.group(2)).strip(),
                        "evidence": f"[{el['index']}] {el['role']} '{el['text'][:80]}'"}
        for el in snapshot.get("elements", []):
            if re.search(r"(start|stop) recording", el.get("text", ""), re.I) and el.get("value") == "1":
                return {"recording": True, "platform": None, "evidence": f"[{el['index']}] '{el['text']}' is on"}
        return {"recording": False, "platform": None, "evidence": None}

    def find_stop_recording_control(self, snapshot: Dict[str, Any]) -> Optional[Dict[str, Any]]:
        for el in self._enabled(snapshot):
            text = el.get("text", "")
            if re.search(r"^stop recording|^stop (&|and) transcribe", text, re.I):
                return el
            if re.search(r"recording \(rec\)|^(start|stop) recording", text, re.I) and el.get("value") == "1":
                return el
        return None

    def find_load_model_control(self, snapshot: Dict[str, Any]) -> Optional[Dict[str, Any]]:
        texts = " ".join(e.get("text", "") for e in snapshot.get("elements", []))
        if not re.search(r"status load required", texts, re.I):
            return None
        return self._find_button(snapshot, re.compile(r"^load model$", re.I))

    def _chat(self, prompt: str) -> str:
        payload = {
            "model": self.model_name,
            "messages": [
                {"role": "system", "content": "You are Decider. You read indexed UI element tables and answer tersely."},
                {"role": "user", "content": prompt},
            ],
            "temperature": 0.0,
            "max_tokens": 150,
        }
        req = urllib.request.Request(
            f"{self.api_base}/chat/completions",
            data=json.dumps(payload).encode("utf-8"),
            headers={"Content-Type": "application/json", "Authorization": f"Bearer {self.api_key}"},
        )
        with urllib.request.urlopen(req, timeout=10.0) as resp:
            return json.loads(resp.read().decode("utf-8"))["choices"][0]["message"]["content"]

    def _ask_http_json(self, snapshot: Dict[str, Any], instruction: str) -> Dict[str, Any]:
        content = self._chat(f"{instruction}\n\nUI elements:\n{snapshot.get('table', '')}")
        match = re.search(r"\{.*\}", content, re.DOTALL)
        if not match:
            raise ValueError(f"No JSON in model reply: {content[:200]}")
        return json.loads(match.group(0))

    def _ask_http_action(self, snapshot: Dict[str, Any], instruction: str) -> DeciderAction:
        res = self._ask_http_json(
            snapshot,
            instruction + ' Respond ONLY with JSON: {"operation": "CLICK"|"WAIT"|"DONE", "target": <int or null>, "reason": "<string>"}',
        )
        return DeciderAction(res.get("operation", "WAIT"), target=res.get("target"), reason=res.get("reason", "LLM decision"))

    def _decide_native(self, snapshot: Dict[str, Any], goal: str, guest_name: str) -> DeciderAction:
        """Queries native Mapika/decider library."""
        elements = snapshot.get("elements", [])
        options = [f"[{e['index']}] {e['text']}" for e in elements]
        if not options:
            return DeciderAction("WAIT", reason="No elements")

        context = f"Goal: {goal}. Observation:\n{snapshot.get('table', '')}"
        questions = [{"question": "What is the next action?", "options": options}]
        results = self._decider_lib.decide(context, questions)
        # Select best option
        best_option = results[0]["prediction"]
        m = re.match(r"\[(\d+)\]", best_option)
        if m:
            target_idx = int(m.group(1))
            matched_el = next((e for e in elements if e["index"] == target_idx), None)
            if matched_el and matched_el["tag"] == "input":
                return DeciderAction("TYPE_TEXT", target=target_idx, text=guest_name, reason="Decider chose input")
            return DeciderAction("CLICK", target=target_idx, reason="Decider chose click")
        return self._decide_system1(snapshot, goal, guest_name)
