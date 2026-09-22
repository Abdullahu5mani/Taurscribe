#!/usr/bin/env python3
"""Per-platform knowledge for the Decider meeting-detection E2E test.

Each adapter knows three things about one browser meeting platform:
  1. how the host creates a meeting and gets a guest join link (a Decider plan
     over the host's accessibility snapshot),
  2. how to tell the host is in the call / count participants,
  3. what Taurscribe should call it and which detector platform id it maps to.

The test itself (test_decider_meeting_detection.py) is platform-agnostic.
"""

from __future__ import annotations

import re
import time
from typing import TYPE_CHECKING, Dict, List, Optional

from decider import PlanStep

if TYPE_CHECKING:
    from test_decider_meeting_detection import ChromeHostAX


class NotSignedIn(Exception):
    """The host browser has no account for this platform; the test is skipped."""


class PlatformAdapter:
    key: str = ""               # detector platform id ("meet", "teams", ...)
    display_name: str = ""      # what the Taurscribe pill says
    host_window_hint: str = ""  # substring of the host window title while in use
    host_kind: str = "ax"       # "ax": user's Chrome via accessibility; "cdp": Mac harness Chrome; "vm": Linux VM
    local_role: str = "host"    # the Mac user's role: "host" (a remote guest joins) or "guest" (remote host)
    in_call = re.compile(r"^leave( call| meeting)?(\s*\(.*\))?$", re.I)
    # Popups that can appear over any host screen.
    common_interrupts: List[PlanStep] = [
        PlanStep("Deny Chrome permission prompt", r"^block$", when=r"wants to$", interrupt=True),
    ]

    def is_meeting_window(self, title: str) -> bool:
        raise NotImplementedError

    def start_meeting(self, host: "ChromeHostAX") -> str:
        """Creates a meeting with the host in it; returns the guest join URL."""
        raise NotImplementedError

    def participant_count(self, snapshot: Dict) -> Optional[int]:
        return None

    def prepare_admit(self, host: "ChromeHostAX", snapshot: Dict) -> bool:
        """Hook run each host tick while a guest joins; returns True if it clicked."""
        return False

    def host_sees_guest(self, snapshot: Dict, guest_name: str) -> Optional[bool]:
        """Host-side proof the guest is in. None means the platform shows no evidence."""
        count = self.participant_count(snapshot)
        return None if count is None else count >= 2

    def leave_host(self, host: "ChromeHostAX"):
        host.leave()

    def local_join(self, attendee: "ChromeHostAX", url: str, timeout: float) -> None:
        """For local_role == "guest": join `url` from the user's own Chrome."""
        raise NotImplementedError

    # Helpers shared by adapters ------------------------------------------------

    def run_plan(self, host: "ChromeHostAX", plan: List[PlanStep], done: re.Pattern,
                 timeout: float, stop_when=None) -> Dict:
        """Lets Decider walk `plan` on the host until `done` shows (or `stop_when(snap)`)."""
        deadline = time.time() + timeout
        snap: Dict = {}
        last_progress = time.time()
        while time.time() < deadline:
            if time.time() - last_progress > 20:
                host.report.screen(f"stuck_{self.key}_host_plan")
                last_progress = time.time()
            snap = host.snapshot()
            if stop_when and stop_when(snap):
                return snap
            action = host.decider.decide_plan(snap, self.common_interrupts + plan, done=None if stop_when else done)
            if action.operation == "DONE":
                return snap
            if action.operation == "CLICK":
                host.report.log("HOST", f"Decider → {action}")
                host.act(action)
                last_progress = time.time()
                time.sleep(2.0)
            else:
                time.sleep(1.0)
        raise RuntimeError(f"{self.display_name}: host plan timed out; last screen:\n{snap.get('table', '')[:1500]}")


class GoogleMeet(PlatformAdapter):
    key = "meet"
    display_name = "Google Meet"
    room_re = re.compile(r"meet\.google\.com/([a-z]{3}-[a-z]{4}-[a-z]{3})")

    def is_meeting_window(self, title: str) -> bool:
        return bool(re.search(r"^Meet [–-] [a-z]{3}-[a-z]{4}-[a-z]{3}|^Google Meet\b", title))

    def start_meeting(self, host):
        host.open_window("https://meet.google.com/new")
        url = host.wait_front_url(self.room_re, timeout=30,
                                  hint="Is Chrome signed in to a Google account?")
        room = self.room_re.search(url).group(1)
        self.host_window_hint = room
        host.window = room
        host.wait_window()
        # /new drops the host straight into the call; a lobby "Join now" is handled too.
        self.run_plan(host, [PlanStep("Host joins", r"^(join now|start meeting)$")],
                      done=self.in_call, timeout=30)
        return f"https://meet.google.com/{room}"

    def participant_count(self, snapshot):
        els = snapshot.get("elements", [])
        for i, el in enumerate(els):
            if el["text"] == "People" and i + 1 < len(els) and els[i + 1]["text"].isdigit():
                return int(els[i + 1]["text"])
        return None


class MicrosoftTeams(PlatformAdapter):
    """Personal Teams (teams.live.com) 'Meet now'. Work accounts use the same UI."""

    key = "teams"
    display_name = "Microsoft Teams"
    host_window_hint = "Microsoft Teams"
    link_re = re.compile(r"https://teams\.(live|microsoft)\.com/(meet|l/meetup-join)/\S+")

    def is_meeting_window(self, title: str) -> bool:
        # A Teams chat window is fine; one that is capturing audio is a live call.
        return "Microsoft Teams" in title and re.search(r"(microphone|camera) recording", title, re.I) is not None

    def start_meeting(self, host):
        host.window = self.host_window_hint
        host.open_window("https://teams.live.com/v2/")
        try:
            host.wait_window(timeout=40)
        except Exception:
            raise NotSignedIn("Teams did not load a signed-in workspace in Chrome")
        # Wait for the app shell, then check we are not on a sign-in page.
        deadline = time.time() + 40
        while time.time() < deadline:
            snap = host.snapshot()
            texts = [e["text"] for e in snap["elements"]]
            if any(re.search(r"^meet now$", t, re.I) for t in texts):
                break
            if any(re.search(r"^(sign in|enter your email)", t, re.I) for t in texts):
                raise NotSignedIn("Chrome is not signed in to Microsoft Teams (teams.live.com)")
            time.sleep(1.5)

        # Meet now → Get a link to share → (read link) → Start meeting.
        link_holder: Dict[str, str] = {}

        def have_link(snap):
            for el in snap["elements"]:
                m = self.link_re.search(el.get("value", "") or "")
                if m:
                    link_holder["url"] = m.group(0)
                    return True
            return False

        self.run_plan(host, [
            PlanStep("Open instant meeting", r"^meet now$"),
            PlanStep("Ask for a shareable link", r"^get a link to share$"),
        ], done=None, timeout=40, stop_when=have_link)
        host.report.log("HOST", f"Teams join link: {link_holder['url']}")

        self.run_plan(host, [
            PlanStep("Start the meeting", r"^start meeting$"),
            PlanStep("Close invite dialog", r"^close$", when=r"^invite people to join you$", interrupt=True),
            PlanStep("Join from pre-join screen", r"^join now$"),
        ], done=self.in_call, timeout=60)
        return link_holder["url"]

    ROSTER_OPEN = re.compile(r"^(participants|in this meeting|in the meeting|waiting in (the )?lobby|lobby)\b", re.I)

    def prepare_admit(self, host, snapshot):
        """Teams only surfaces Admit inside the People pane (its lobby toast is a
        dismiss target), so keep the pane open while the guest joins."""
        if any(self.ROSTER_OPEN.search(e["text"]) and e["role"] != "AXButton" for e in snapshot["elements"]):
            return False
        people = next((e for e in snapshot["elements"]
                       if e["role"] == "AXButton" and e["text"] == "People" and not e["disabled"]), None)
        if people:
            from decider import DeciderAction
            action = DeciderAction("CLICK", target=people["index"], reason="Open the People pane to see the lobby")
            host.report.log("HOST", f"Decider → {action}")
            host.act(action)
            return True
        return False

    def participant_count(self, snapshot):
        # The People button reads "People" and, once others join, a separate count
        # node follows; the roster label also appears as "People (2)" in some builds.
        els = snapshot.get("elements", [])
        for el in els:
            # Participants pane header, e.g. "In this meeting (2)".
            m = re.match(r"^in this meeting \((\d+)\)$", el["text"], re.I)
            if m:
                return int(m.group(1))
        for i, el in enumerate(els):
            m = re.match(r"^people\s*\(?(\d+)\)?$", el["text"], re.I)
            if m:
                return int(m.group(1))
            if el["text"] == "People" and el["role"] == "AXButton" and i + 1 < len(els) and els[i + 1]["text"].isdigit():
                return int(els[i + 1]["text"])
        return None


class Zoom(PlatformAdapter):
    """Zoom web client (app.zoom.us/wc).

    Hosted from the harness Chrome profile over CDP: Zoom keeps its toolbar
    (Leave / End) out of the DOM until the pointer hovers the video, which only
    CDP can do without touching the user's real mouse.
    """

    key = "zoom"
    display_name = "Zoom"
    # The signed-in host runs in the Linux VM (full DevTools control, so it can
    # end the meeting for all); the Mac user joins from their own Chrome.
    host_kind = "vm"
    local_role = "guest"
    id_re = re.compile(r"/wc/(\d{9,11})/")
    invite_re = re.compile(r"https://[\w.]*zoom\.us/j/(\d{9,11})\?pwd=([\w.\-]+)")

    def is_meeting_window(self, title: str) -> bool:
        return "Zoom Meeting" in title and re.search(r"(microphone|camera) recording", title, re.I) is not None

    def start_meeting(self, host):
        # Home and the meeting both live inside iframes of the PWA shell; target the
        # meeting frame once it exists, the shell before that.
        host.frame_hint = re.compile(r"/wc/\d{9,11}/(start|join)")
        host.open_window("https://app.zoom.us/wc/home")
        host.wait_window(timeout=30)
        deadline = time.time() + 40
        while time.time() < deadline:
            texts = [e["text"] for e in host.snapshot()["elements"]]
            if any(re.search(r"^new meeting$", t, re.I) for t in texts):
                break
            if any(re.search(r"^(sign in|enter email|sign up)", t, re.I) for t in texts) or "signin" in host.url():
                if getattr(host, "remote_vm", None) and not getattr(self, "_synced_login", False):
                    # The VM has no screen to sign in on: bring the Mac test profile's login over.
                    self._synced_login = True
                    if host.import_login_from_mac("zoom.us"):
                        host.open_window("https://app.zoom.us/wc/home")
                        time.sleep(4)
                        continue
                raise NotSignedIn("Zoom is not signed in for the test host. Sign in once in the Mac test Chrome "
                                  "(profile ~/.taurscribe-harness/host-chrome); the VM copies that login")
            time.sleep(1.5)
        else:
            raise NotSignedIn("Zoom web client never showed 'New meeting'")

        def blocked(snap):
            if any(e["text"] == "End other meeting" or "meeting that is currently in-progress" in e["text"]
                   for e in snap["elements"]):
                raise RuntimeError("Zoom says another meeting on this account is still in progress. "
                                   "End it yourself (the test will not end meetings it did not start).")
            return False

        def in_call(snap):
            blocked(snap)
            return host.decider.has_leave_control(snap)

        self.run_plan(host, [
            PlanStep("Start an instant meeting", r"^new meeting$"),
            PlanStep("Join with audio", r"^use microphone and camera$"),
            PlanStep("Dismiss tip", r"^got it$", interrupt=True),
        ], done=None, timeout=60, stop_when=in_call)

        meeting_id = self.id_re.search(host.url()).group(1)
        invite = self.find_invite(host)
        if not invite:
            raise RuntimeError("Could not find the Zoom invite link (passcode) on the host page")
        host.report.log("HOST", f"Zoom meeting {meeting_id} live")
        mid, pwd = invite
        return f"https://app.zoom.us/wc/join/{mid}?pwd={pwd}"

    def find_invite(self, host):
        m = re.search(r"pwd=([\w.\-]+)", host.url())
        mid = self.id_re.search(host.url()).group(1)
        if m:
            return mid, m.group(1)
        scan = "document.documentElement.innerHTML + ' ' + [...document.querySelectorAll('input,textarea')].map(e => e.value).join(' ')"
        for _ in range(3):
            html = str(host.page.evaluate(scan))
            m = self.invite_re.search(html)
            if m:
                return m.group(1), m.group(2)
            # The meeting-information popover holds the invite link.
            snap = host.snapshot()
            info = next((e for e in snap["elements"] if re.search(r"meeting info", e["text"], re.I)), None)
            if info:
                from decider import DeciderAction
                host.act(DeciderAction("CLICK", target=info["index"], reason="Open meeting information"))
                time.sleep(1.5)
        return None

    def local_join(self, attendee, url, timeout):
        attendee.window = "Zoom"
        attendee.open_window(url)
        attendee.wait_window(timeout=30)

        def in_call(snap):
            # The preview screen already records the mic, so also require that the
            # final Join button is gone.
            mic_live = re.search(r"microphone recording", snap.get("title", ""), re.I) is not None
            join_visible = any(re.search(r"^join$", e["text"], re.I) and not e.get("disabled")
                               for e in snap["elements"])
            return mic_live and not join_visible

        def fill_name_or_stop(snap):
            texts = [e["text"] for e in snap["elements"]]
            if any(re.search(r"i'?m not a robot|verify you are human|select all images", t, re.I) for t in texts):
                raise RuntimeError("Zoom is showing a CAPTCHA challenge; the test does not solve CAPTCHAs")
            # Zoom pre-fills the name for a signed-in browser, but not always.
            field = next((e for e in snap["elements"]
                          if e["role"] == "AXTextField" and not e.get("value")
                          and re.search(r"your name|name", e["text"], re.I)), None)
            if field:
                import ax_driver
                attendee.raise_window()
                ax_driver.set_value(attendee.pid, field["index"], "Taurscribe Test (You)",
                                    window=attendee.window, expect=field["text"])
                attendee.report.log("YOU", f"typed a display name into '{field['text']}'")
            return in_call(snap)

        self.run_plan(attendee, [
            PlanStep("Join with audio", r"^(use microphone and camera|join audio by computer|join audio)$"),
            PlanStep("Join", r"^join$"),
            PlanStep("Allow mic for this visit", r"^allow this time$", when=r"^use your microphones?$", interrupt=True),
            PlanStep("Dismiss macOS camera notice", r"^close$", when=r"needs access to your device's camera", interrupt=True),
            PlanStep("Dismiss tip", r"^got it$", interrupt=True),
        ], done=None, timeout=timeout, stop_when=fill_name_or_stop)

    def participant_count(self, snapshot):
        for e in snapshot["elements"]:
            # Zoom's label: "open the manage participants list pane,2 particpants" (sic).
            m = re.search(r"(\d+)\s*partic\w*", e["text"], re.I)
            if m and "button" in e["role"].lower():
                return int(m.group(1))
        return None

    def host_sees_guest(self, snapshot, guest_name):
        if any(e["text"].lower() == f"{guest_name} has joined the meeting".lower() for e in snapshot["elements"]):
            return True
        count = self.participant_count(snapshot)
        return None if count is None else count >= 2

    def leave_host(self, host):
        def gone(snap):
            return not host.decider.has_leave_control(snap)
        self.run_plan(host, [
            PlanStep("End (host)", r"^(end|leave)$"),
            PlanStep("End for everyone", r"^end meeting for all$", interrupt=True),
        ], done=None, timeout=30, stop_when=gone)
        # The toolbar disappearing is not proof: wait for Zoom to confirm the end
        # (or leave the meeting URL) before navigating away, or the meeting can
        # stay "in progress" server-side and block the next run.
        deadline = time.time() + 20
        ended = False
        while time.time() < deadline:
            text = str(host._eval("document.body ? document.body.innerText : ''") or "")
            if re.search(r"meeting (has been )?ended|ended by host|thank you for attending|you left the meeting", text, re.I) \
                    or not self.id_re.search(host.url()):
                ended = True
                break
            time.sleep(1)
        host.report.log("HOST", "Decider ended the Zoom meeting for all" + ("" if ended else " (no end confirmation seen)"))
        time.sleep(2)
        host.close()


PLATFORMS: Dict[str, PlatformAdapter] = {
    "meet": GoogleMeet(),
    "teams": MicrosoftTeams(),
    "zoom": Zoom(),
}
