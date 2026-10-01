"""Trusted task constraints for optional model navigation, independent of scoring."""
import re


NAVIGATION_LABELS = {
    'open_settings': {'Settings', 'Open settings', 'Open Settings'},
    'close_settings': {'Close settings'},
    'files': {'File transcription mode'},
    'meetings': {'Meeting detection and dual-channel recording mode'},
    'microphone': {'Microphone dictation mode'},
}


def ui_labels(goal):
    """Unknown goals do not grant permission to record, delete or change values."""
    goal = goal.strip()
    tab = re.fullmatch(r'view (?:the )?(Meetings|Recording|Models|About|Grammar|Text|App|General)(?: settings(?: tab)?)?', goal, re.I)
    if tab:
        if tab.group(1).lower() == 'app':
            return {'General', 'App'}
        return {tab.group(1).title()}
    if re.search(r'\bclose settings\b', goal, re.I):
        return NAVIGATION_LABELS['close_settings']
    if re.search(r'\bopen (?:Taurscribe )?settings\b', goal, re.I):
        return NAVIGATION_LABELS['open_settings']
    for pattern, key in ((r'\bview file transcription mode\b', 'files'),
                         (r'\bview meeting detection and dual-channel recording mode\b', 'meetings'),
                         (r'\bview microphone dictation mode\b', 'microphone')):
        if re.search(pattern, goal, re.I):
            return NAVIGATION_LABELS[key]
    transcript = re.fullmatch(r'expand the transcript of (.+)', goal, re.I)
    if transcript:
        return {'Show transcript for ' + transcript.group(1)}
    if re.fullmatch(r'retry saving the completed transcript to history', goal, re.I):
        return {'Retry saving to history'}
    if re.fullmatch(r'review the saved meeting', goal, re.I):
        return {'Review meeting'}
    if re.fullmatch(r'turn the microphone on before joining; it is currently muted', goal, re.I):
        return {'Unmute microphone', 'Unmute mic', 'Turn on microphone'}
    return set()


def constrain(snapshot, *, target=None, labels=None):
    """Keep full observation state, restrict actionable indices unambiguously."""
    allowed = set()
    for item in snapshot.get('elements', []):
        index = item.get('index')
        if type(index) is not int or index < 0 or item.get('disabled'):
            continue
        if (target is not None and index == target) or (labels is not None and item.get('text') in labels):
            if sum(e.get('index') == index for e in snapshot.get('elements', [])) == 1:
                allowed.add(index)
    return dict(snapshot, _allowed_action_targets=allowed)
