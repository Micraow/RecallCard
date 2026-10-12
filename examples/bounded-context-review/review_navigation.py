"""Finite, monotonic traversal of already-returned candidate-review graph edges.

No timestamps or array ordering establish graph edges. A navigation probe may
cost a second, explicitly reserved call to follow one unique returned neighbor.
"""
from __future__ import annotations
import copy
from judgment_contract import _require


def gap(ranges, begin, end):
    position = begin
    for lo, hi in sorted(ranges):
        if lo > position:
            break
        position = max(position, hi)
    return position if position < end else None


class Navigation:
    def __init__(self, wire):
        self.anchor = wire['anchor_ref']
        self.begin = wire['anchor_start_byte']
        self.sources = {}
        self.attempted = set()
        self.closed = set()
        self.initial_edges = {d: [s['reference'] for s in wire['evidence'] if s['relation'] == d]
                              for d in ('previous', 'next')}
        for targets in self.initial_edges.values():
            _require(len(set(targets)) <= 1, 'ambiguous initial neighbor')
        for span in wire['evidence']:
            self.add(span, 0 if span['reference'] == self.anchor else 1)

    def add(self, span, distance):
        ref = span['reference']
        meta = {k: span[k] for k in ('snapshot', 'relation', 'role', 'occurred_at', 'total_bytes')}
        source = self.sources.setdefault(ref, dict(meta, distance=distance, ranges=[], navigation=None, window=None))
        _require(all(source[k] == v for k, v in meta.items()) and source['distance'] == distance,
                 'navigation source identity changed')
        source['ranges'].append([span['start_byte'], span['end_byte']])

    def offset(self, ref, probe=False):
        if ref in self.closed:
            return None
        source = self.sources.get(ref)
        begin = self.begin if ref == self.anchor else 0
        end = min(begin+2048, source['total_bytes']) if source else begin+2048
        value = gap(source['ranges'] if source else [], begin, end)
        if value is None and probe and source and source['navigation'] is None:
            value = begin
        return value if value is not None and (ref, value) not in self.attempted else None

    def spec(self, ref, action, remaining):
        if remaining <= 0 or ref not in self.sources or ref in self.closed:
            return None
        source = self.sources[ref]
        if action == 'continue':
            offset = self.offset(ref)
            return None if offset is None else dict(target=ref, offset=offset, max_calls=1,
                probe=False, direction=source['relation'], distance=source['distance'])
        _require(action in ('check_previous', 'check_next'), 'unknown navigation action')
        direction = action.removeprefix('check_')
        if source['relation'] not in ('anchor', direction):
            return None
        nav = source['navigation']
        if nav is None and ref == self.anchor and self.initial_edges[direction]:
            target = self.initial_edges[direction][0]
            offset = self.offset(target, True)
            return None if offset is None else dict(target=target, offset=offset, max_calls=1,
                probe=False, direction=direction, distance=1)
        if nav is None:
            offset = self.offset(ref, True)
            return None if offset is None else dict(target=ref, offset=offset,
                max_calls=min(remaining, 2), probe=True, direction=direction, distance=source['distance'])
        if source['distance'] >= 2:
            return None
        targets = [nav['previous_ref']] if direction == 'previous' and nav['previous_ref'] else (nav['next_refs'] if direction == 'next' else [])
        if len(targets) != 1:
            return None
        target = targets[0]
        if target in (self.anchor, ref):
            return None
        if target in self.sources and self.sources[target]['relation'] != direction:
            return None
        offset = self.offset(target, True)
        return None if offset is None else dict(target=target, offset=offset, max_calls=1,
            probe=False, direction=direction, distance=source['distance']+1)

    def accept(self, spec, page):
        """Validate a copy first; caller commits it only with the bounded adapter."""
        trial = copy.deepcopy(self)
        ref = spec['target']
        trial.attempted.add((ref, spec['offset']))
        if page.get('status') not in ('partial', 'complete'):
            trial.closed.add(ref)
            return trial
        updated = page.get('updated_candidate')
        if updated is None:
            _require(ref in trial.sources, 'unknown neighbor without bounded evidence')
            trial.closed.add(ref)
            return trial
        span = updated['evidence'][0]
        relation = self.sources[ref]['relation'] if spec['probe'] else spec['direction']
        _require(span['reference'] == ref and span['start_byte'] == spec['offset']
                 and span['relation'] == relation, 'inspection offset or direction changed')
        nav = page['navigation']
        distance = nav['distance_from_anchor']
        _require(type(distance) is int and distance == spec['distance'] and 0 <= distance <= 2
                 and (distance == 0) == (ref == self.anchor), 'navigation distance changed')
        previous, following = nav.get('previous_ref'), nav.get('next_refs')
        _require(previous is None or type(previous) is str and previous.startswith('event:'), 'invalid previous edge')
        _require(type(following) is list and len(following) <= 1
                 and all(type(x) is str and x.startswith('event:') for x in following), 'ambiguous next edge')
        _require(ref != previous and ref not in following and (previous is None or previous not in following), 'cyclic graph edge')
        trial.add(span, distance)
        source = trial.sources[ref]
        window = [page['window']['start_byte'], page['window']['end_byte']]
        _require(source['window'] is None or source['window'] == window, 'window rebased')
        _require(source['navigation'] is None or source['navigation'] == nav, 'navigation changed within revision')
        source['window'], source['navigation'] = window, copy.deepcopy(nav)
        return trial
