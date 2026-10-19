#!/usr/bin/env python3
"""Join positive phone/board Link observations without inferring RF or delivery.

Each input keeps its own complete-prefix coverage result. Public ephemeral Link
hashes correlate only that handshake; no group payload or message ID is inferred.
"""
from __future__ import annotations
import argparse
from collections import defaultdict
import importlib.util
import json
from pathlib import Path
import re


def sibling(name):
    spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name(name+'.py'))
    assert spec and spec.loader
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module


BOARD=sibling('board_link_timeline');NATIVE=sibling('link_timeline');ROUTES=BOARD.ROUTES


def analyze(inputs,start,end,*,same_host=False):
    sources,events={},[]
    for name,(kind,path) in inputs.items():
        rows,info=ROUTES.snapshot(path)
        coverage=(BOARD if kind=='board' else NATIVE).analyze(rows,start,end)
        if info['errors']:coverage['qualified_coverage']=False
        sources[name]={'type':kind,'input':info,**{k:v for k,v in coverage.items() if k!='events'}}
        # Select positive validated observations even if a missing final heartbeat
        # makes coverage unqualified. Never repair or synthesize missing records.
        clock_anchors=BOARD.clock_anchors(rows) if kind=='board' else {}
        for row in rows:
            at=ROUTES.PHONE.epoch(row['host_observed_utc'])
            if not start<=at<=end:continue
            if kind=='board':
                valid=row.get('kind')=='link_event' and BOARD.valid_record(row)
            else:
                valid=row.get('tag')=='prns.links' and NATIVE.valid(row.get('fields',{}))
            if not valid:continue
            event={'source':name,'capture_sequence':row['sequence'],
                   'host_observed_utc':row['host_observed_utc'],'host_monotonic_ns':row['host_monotonic_ns'],**row['fields']}
            if kind=='board':event.update(boot_token=row['boot_token'],board_observed_ms=row['board_observed_ms'],core_in_radio_clock_ms=BOARD.core_clock_intervals(row,clock_anchors))
            events.append(event)
    events.sort(key=lambda e:(e['host_monotonic_ns'] if same_host else ROUTES.PHONE.epoch(e['host_observed_utc']),e['source'],e['capture_sequence']))
    links=defaultdict(list);unattributed=[]
    for event in events:
        token=event.get('link')
        if event.get('has_link')=='true' and token!='0'*16:links[token].append(event)
        else:unattributed.append(event)
    return {'sources':sources,'all_sources_qualified':bool(sources) and all(s['qualified_coverage'] for s in sources.values()),
            'ordering':'same_host_monotonic' if same_host else 'wall_clock_context_only',
            'links':[{'link_token':token,'sources_observed':sorted({e['source'] for e in value}),'events':value} for token,value in links.items()],
            'unattributed_events':unattributed,
            'limits':'Positive software observations only. Shared Link hashes do not bind a group message. Cross-stream receipt timestamps include reader buffering and scheduling; they are not RF latency. A complete software prefix never proves RF absence or durable delivery.'}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--phone',action='append',default=[],metavar='NAME=CAPTURE')
    parser.add_argument('--board',action='append',default=[],metavar='NAME=CAPTURE')
    parser.add_argument('--start',required=True);parser.add_argument('--end',required=True)
    parser.add_argument('--same-host',action='store_true',help='All capture journals use this one host monotonic clock')
    args=parser.parse_args();inputs={}
    for kind,values in (('phone',args.phone),('board',args.board)):
        for value in values:
            name,separator,path=value.partition('=')
            if not separator or not re.fullmatch(r'[A-Za-z0-9_-]{1,48}',name) or not path or name in inputs:parser.error('use unique NAME=CAPTURE inputs')
            inputs[name]=(kind,Path(path))
    if not 1<=len(inputs)<=8:parser.error('one to eight inputs required')
    start,end=ROUTES.PHONE.epoch(args.start),ROUTES.PHONE.epoch(args.end)
    if end<=start:parser.error('end must be after start')
    print(json.dumps(analyze(inputs,start,end,same_host=args.same_host),indent=2))


if __name__=='__main__':main()
