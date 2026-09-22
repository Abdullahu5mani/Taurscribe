/**
 * Atomic DOM snapshotter inspired by Jev Ultrafast (TypeSafe / Browser-Use).
 * Evaluates in ~10-15ms inside any Chromium / WebKit page over CDP.
 * Traverses visible interactive elements and produces a clean, indexed action table.
 */
(() => {
  const isVisible = (el) => {
    if (!el) return false;
    const style = window.getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') {
      return false;
    }
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0 && rect.bottom >= 0 && rect.top <= window.innerHeight;
  };

  const getCleanText = (el) => {
    // Check aria-label, placeholder, title, or innerText
    const label = el.getAttribute('aria-label') || el.getAttribute('placeholder') || el.getAttribute('title');
    if (label && label.trim()) return label.trim();
    const text = el.innerText || el.textContent || '';
    return text.replace(/\s+/g, ' ').trim().slice(0, 100);
  };

  const interactiveSelectors = [
    'button',
    'input',
    'select',
    'textarea',
    '[role="button"]',
    '[role="checkbox"]',
    '[role="combobox"]',
    '[role="switch"]',
    '[role="menuitem"]',
    '[role="tab"]',
    'a[href]'
  ];

  const elements = [];
  const nodes = document.querySelectorAll(interactiveSelectors.join(','));
  let index = 1;

  nodes.forEach((node) => {
    if (!isVisible(node)) return;

    const tag = node.tagName.toLowerCase();
    const role = node.getAttribute('role') || tag;
    const text = getCleanText(node);
    const value = node.value !== undefined ? String(node.value) : '';
    const disabled = Boolean(node.disabled || node.getAttribute('aria-disabled') === 'true');
    const checked = Boolean(node.checked || node.getAttribute('aria-checked') === 'true' || node.getAttribute('aria-pressed') === 'true');
    const inputType = node.getAttribute('type') || (tag === 'input' ? 'text' : '');

    // Skip blank or useless controls
    if (!text && !value && tag !== 'input') return;

    // Assign DOM node marker for direct reference
    node.setAttribute('data-jev-index', String(index));

    elements.push({
      index,
      tag,
      role,
      type: inputType,
      text,
      value,
      disabled,
      checked,
    });
    index++;
  });

  // Build indexed element table string
  const tableLines = elements.map((e) => {
    let desc = `[${e.index}] ${e.role} '${e.text}'`;
    if (e.value) desc += ` · value: '${e.value}'`;
    if (e.disabled) desc += ` · disabled`;
    if (e.checked) desc += ` · checked/active`;
    return desc;
  });

  // Check in-call status for Google Meet & Teams
  const inCallSelectors = [
    'button[aria-label*="Leave call" i]',
    'button[data-call-action="hangup"]',
    'button[data-tid="hangup-button"]',
    'button[aria-label*="Hang up" i]',
    'button[aria-label*="Leave" i]',
    '#leave-btn',
    '#join-btn'
  ];
  let isInCall = false;
  for (const s of inCallSelectors) {
    const el = document.querySelector(s);
    if (el && isVisible(el)) {
      if (s === '#join-btn') {
        isInCall = el.innerText.includes('Call in progress');
      } else {
        isInCall = true;
      }
      if (isInCall) break;
    }
  }

  // Knock/waiting screens show a "Leave call" button before the host admits us.
  const pageText = document.body ? document.body.innerText : '';
  if (/asking to be let in|brings you into the call|should let you in soon|let you in shortly|let people know you're waiting|waiting for someone to let you in|when someone lets you in|someone in the call needs to let you in/i.test(pageText)) {
    isInCall = false;
  } else if (!isInCall) {
    const leaveMatch = elements.find(e => /^(leave|hang up|end call)\b/i.test(e.text));
    if (leaveMatch) isInCall = true;
  }

  // Check for pending participant admission dialog on Host view
  let admitRequest = null;
  const admitBtn = document.querySelector('button[aria-label*="Admit" i], button:not([disabled])');
  const bodyText = document.body ? document.body.innerText : '';
  if (bodyText.includes('wants to join') || bodyText.includes('waiting in the lobby')) {
    const admitMatch = elements.find(e => /admit/i.test(e.text));
    if (admitMatch) {
      admitRequest = { text: 'Participant waiting to join', admit_target: admitMatch.index };
    }
  }

  return {
    title: document.title || '',
    url: window.location.href || '',
    is_in_call: isInCall,
    admit_request: admitRequest,
    elements: elements,
    table: tableLines.join('\n')
  };
})();
