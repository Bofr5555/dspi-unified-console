#!/usr/bin/env python3
"""WCAG 2 contrast of the Theme's text/background pairs and of textOnTint on ChannelPalette.
Usage: contrast.py qml/Theme/Theme.qml qml/Theme/ChannelPalette.qml [-v]   (ci/check.sh --step qt runs it)"""
import re, sys
src = open(sys.argv[1]).read()
tok = {}
for name, d, l in re.findall(r'property color (\w+): (?:toned\.\w+ \|\| \()?dark \? "(#\w+)" : "(#\w+)"', src): tok[name] = (d, l)
for name, v in re.findall(r'property color (\w+): "(#\w+)"', src): tok[name] = (v, v)
def lum(h):
    h = h.lstrip('#'); c = [int(h[i:i+2], 16) / 255 for i in (0, 2, 4)]
    c = [x / 12.92 if x <= 0.03928 else ((x + 0.055) / 1.055) ** 2.4 for x in c]
    return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
def ratio(a, b):
    la, lb = sorted((lum(a), lum(b)), reverse=True); return (la + 0.05) / (lb + 0.05)
surf = ('window', 'panel', 'sidebar', 'control', 'menu', 'selection')
pairs = [(t, b) for t in ('text', 'textMuted', 'textDisabled') for b in surf]
pairs += [(t, b) for t in ('accentText', 'dangerText', 'warning', 'success') for b in ('window', 'panel', 'sidebar', 'control')]
pairs += [(t, 'meterTrack') for t in ('meterLevel', 'meterLow', 'meterMid', 'meterHigh', 'meterHold')]  # -- WP QP18 -- graphics, 3:1
pairs += [(t, b) for t in ('alarmRaised', 'alarmAcknowledged') for b in ('window', 'panel', 'sidebar', 'control')]
pairs += [('textOnAccent', 'accent'), ('textOnAccent', 'danger'), ('tooltipText', 'tooltip')]
bad = 0
# -- WP LAY4 -- Modern's grey tones: each names the colours it changes ("grey dark": { window: "#202020", ... })
tones = {'': {}}
for tone, scheme, body in re.findall(r'"(\w+) (dark|light)": \{(.*?)\}', src, re.S):
    assert set(dict(re.findall(r'(\w+): "(#\w+)"', body))) <= set(tok), tone
    tones.setdefault(tone, {})[scheme] = dict(re.findall(r'(\w+): "(#\w+)"', body))
for tone, i, scheme in ((t, i, s) for t in tones for i, s in enumerate(('dark', 'light'))):
    at = {name: v[i] for name, v in tok.items()} | tones[tone].get(scheme, {})
    for t, b in pairs + [('text', 'textSelection')]:
        r = ratio(at[t], at[b]); ok = r >= (3.0 if t == 'textDisabled' or b == 'meterTrack' else 4.5)
        bad += not ok
        if not ok or '-v' in sys.argv: print(f'{tone:6} {scheme:5} {t:13} on {b:10} {r:5.2f} {"" if ok else "BELOW AA"}')
pal = re.findall(r'Qt\.rgba\(([\d.]+), ([\d.]+), ([\d.]+), 1\)', open(sys.argv[2]).read()) if len(sys.argv) > 2 and sys.argv[2] != '-v' else []
for r_, g, b in pal:
    h = '#%02x%02x%02x' % tuple(round(float(x) * 255) for x in (r_, g, b))
    k = ratio(tok['textOnTint'][0], h); bad += k < 4.5
    print(f'palette {h}: textOnTint {k:4.2f}{"" if k >= 4.5 else " BELOW AA"}')
print('pairs below AA:', bad); sys.exit(1 if bad else 0)
