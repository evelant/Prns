"""CRC framing and complete-prefix board Link coverage must never invent absence."""
import copy
import importlib.util
from pathlib import Path
import unittest
import zlib
import tempfile
import json
from unittest.mock import patch
from tools.tests import test_group_trial_board_observer as capture
from tools.tests import test_group_trial_link_timeline as native
from tools.tests import test_group_trial_observer as base

path = Path(__file__).resolve().parents[1] / 'device/group-trial/board_link_timeline.py'
spec = importlib.util.spec_from_file_location('board_link_timeline', path)
BOARD = importlib.util.module_from_spec(spec); spec.loader.exec_module(BOARD)
SCHEMA = BOARD.SCHEMA
BOOT = '123456789abcdef0'


def probe():
    return dict(stage='probe',category='installed',core_ms='1000',radio_before_ms='2000',radio_after_ms='2002')


def health(core=0, lora=0, **changes):
    value = dict(stage='health', core_seen=str(core), lora_seen=str(lora), core_admitted=str(core), lora_admitted=str(lora),
                 core_published=str(core), lora_published=str(lora), sink_dropped='0', core_capped='false', lora_capped='false', healthy='true')
    value.update(changes); return value


def lora(source='1', phase='TxComplete'):
    value = dict(stage='handshake', family='lora', source_sequence=source, sequence='1', link='1111111111111111', has_link='true',
                 packet='Request', phase=phase, at_ms='6500', has_at='true', age_ms='500', has_age='true', queue_bytes='0', active='true', frame_index='0', frame_count='1')
    value.update({k:'0' for k in SCHEMA.LORA_COUNTERS}); value.update(attempted=source,emitted=source,tx_started='1',tx_complete='1')
    return value


def frame(fields, sequence=1, observed=0, boot=BOOT):
    fields = {k:v for k,v in fields.items() if k != 'sequence'}
    body = f'v=1 boot={boot} seq={sequence} observed_ms={observed} ' + ' '.join(f'{k}={v}' for k,v in fields.items())
    return SCHEMA.PREFIX + body + f' checksum={zlib.crc32(body.encode()):08x}'


def resgin(body):
    """Deliberately compute a valid checksum for a structurally invalid payload."""
    return SCHEMA.PREFIX + body + f' checksum={zlib.crc32(body.encode()):08x}'


def row(second, kind, **fields):
    return dict(schema=1,capture_id='capture',host_observed_utc=base.stamp(second),host_monotonic_ns=int(second*1e9),kind=kind,**fields)


def records():
    data = [row(0,'capture_started',link_heartbeat_expected=True), row(.1,'link_probe',fields=probe()),
            row(6.5,'link_event',fields=native.core()), row(7.5,'link_event',fields=lora())]
    for second in range(2, 27, 2): data.append(row(second,'link_health',fields=health(int(second>=6.5),int(second>=7.5))))
    for second in range(0, 31, 5): data.append(row(second+.2,'capture_health',read_loop_alive=True,port_open=True,rejected_link_lines=0,undecodable_lines=0,oversized_lines=0))
    data.sort(key=lambda r:r['host_monotonic_ns']); seq=0
    for index, r in enumerate(data,1):
        r['sequence']=index
        if r['kind'] in BOARD.KINDS:
            seq+=1; r.update(trace_version=1,boot_token=BOOT,board_sequence=seq,board_observed_ms=r['host_monotonic_ns']//1_000_000)
            if r['kind']=='link_event':r['fields']['sequence']=str(seq)
    return data


def analyze(data,start=5,end=20):
    data.sort(key=lambda r:r['host_monotonic_ns'])
    for index,r in enumerate(data,1):r['sequence']=index
    return BOARD.analyze(data,base.OBSERVER.epoch(base.stamp(start)),base.OBSERVER.epoch(base.stamp(end)))


class BoardLinkSchemaTests(unittest.TestCase):
    def test_probe_core_lora_health_and_terminal_shapes(self):
        for fields,kind in ((probe(),'link_probe'),(native.core(),'link_event'),(lora(),'link_event'),(health(),'link_health'),
                            (dict(stage='limit',category='health',limit='1800'),'link_limit')):
            parsed=capture.BOARD.parse_safe_line(frame(fields))
            self.assertEqual(parsed['kind'],kind); self.assertNotIn('raw',parsed)
        for phase in SCHEMA.LORA_PHASES:
            self.assertIsNotNone(SCHEMA.parse(frame(lora(phase=phase))))

    def test_changed_bytes_interleave_truncation_concat_and_duplicate_rejected(self):
        line=frame(lora()); body=line[len(SCHEMA.PREFIX):].rsplit(' checksum=',1)[0]
        for value in (line.replace('queue_bytes=0','queue_bytes=1'),line[:100]+'other log PRIVATE'+line[100:],line[:100],line[100:],line+line,
                      resgin(body+' secret=PRIVATE'),resgin(body+' sequence=7'),resgin(body+' seq=7'),resgin(body.replace('TxComplete','UnknownPhase')),
                      resgin(body.replace('frame_count=1','frame_count=256')),resgin(body.replace('queue_bytes=0','queue_bytes=65536')),
                      resgin(body.replace('has_link=true','has_link=maybe')),resgin(body.replace('source_sequence=1','source_sequence=18446744073709551616'))):
            self.assertIsNone(capture.BOARD.parse_safe_line(value),value)
        self.assertIsNone(SCHEMA.parse('x'*769))

    def test_capture_metadata_retains_gaps_and_drops_private_lines(self):
        helper=capture.BoardObserverTests()
        bad=frame(lora()).replace('tx_started=1','tx_started=2')
        _,rows,text,_=helper.run_capture(seconds=4,chunks=[(frame(probe())+'\n'+frame(health(),sequence=3)+'\n'+bad+'\nPRIVATE\n').encode()])
        self.assertEqual(rows[-1]['rejected_link_lines'],1);self.assertNotIn('PRIVATE',text)
        self.assertTrue(rows[-1]['board_heartbeat_available']);self.assertIn('link_event_gap',rows[-1]['problems'])

    def test_boot_change_and_terminal_are_explicit(self):
        recorder=capture.Recorder(); observer=capture.BOARD.Observation(recorder)
        observer.accept(SCHEMA.parse(frame(probe())))
        observer.accept(SCHEMA.parse(frame(health(),sequence=2,boot='0'*16)))
        observer.accept(SCHEMA.parse(frame(dict(stage='limit',category='health',limit='1800'),sequence=3,boot='0'*16)))
        self.assertIn('link_boot_changed',observer.problems);self.assertIn('link_health_budget_exhausted',observer.problems)

    def test_setup_reset_is_opt_in_ordered_and_journaled(self):
        helper=capture.BoardObserverTests()
        with patch.object(capture.BOARD.time,'sleep') as sleep:
            _,rows,_,port=helper.run_capture(setup_reset=True)
        self.assertEqual(port.calls[:10], [('dtr',False),('rts',False),('port','/dev/test'),('open',False,False),
                         ('dtr',False),('rts',True),('dtr',False),('rts',True),('rts',False),('read',4096)])
        self.assertEqual([call.args for call in sleep.call_args_list],[(.1,),(.1,),(.1,)])
        self.assertEqual([r['kind'] for r in rows[:4]],['capture_started','serial_opened','setup_reset_started','setup_reset_completed'])
        self.assertTrue(rows[0]['setup_reset_requested'])
        _,rows,_,port=helper.run_capture()
        self.assertFalse(rows[0]['setup_reset_requested']);self.assertNotIn(('rts',True),port.calls)
        _,rows,_,port=helper.run_capture(setup_reset=True,matching=False)
        self.assertFalse(port.calls)
        with self.assertRaises(ValueError):capture.BOARD.setup_reset(capture.FakePort(),vid=0x303A,pid=2)

    def test_board_envelope_sequence_can_exceed_phone_budget(self):
        for f in (native.core(),lora()):
            self.assertIsNotNone(SCHEMA.parse(frame(f,sequence=14090)))

    def test_fault_prefixes_keep_only_closed_categories(self):
        examples={
            '====================== PANIC ======================':'panic_banner',
            'Backtrace:':'backtrace_header', 'panicked at PRIVATE/path.rs:7:9:':'panic_location',
            'Stack overflow detected in 0xPRIVATE':'stack_overflow',
            'assertion `left == right` failed: Stack overflow detected in 0xPRIVATE':'stack_overflow',
            'Detected a write to the stack guard value on Cpu0 PRIVATE':'stack_guard',
            'Exception occurred on Cpu0 PRIVATE':'exception', 'Exception: PRIVATE':'exception',
            'Double Exception: PRIVATE':'double_exception', 'Breakpoint on PRIVATE':'breakpoint'}
        for text,category in examples.items():
            self.assertEqual(capture.BOARD.parse_safe_line(text),dict(kind='firmware_fault',category=category))
        for text in ('ordinary PRIVATE panic', 'Backtrace: PRIVATE', 'stack guard PRIVATE', '0xPRIVATE', 'panic! PRIVATE'):
            self.assertIsNone(capture.BOARD.parse_safe_line(text))
        _,rows,text,_=capture.BoardObserverTests().run_capture(chunks=[('panicked at PRIVATE/path.rs:1:1:\nStack overflow detected in 0xPRIVATE\n').encode()])
        self.assertNotIn('PRIVATE',text);self.assertEqual(rows[-1]['firmware_faults_observed'],2)

    def test_fault_capture_is_bounded_and_does_not_advance_link_sequence(self):
        recorder=capture.Recorder();observer=capture.BOARD.Observation(recorder)
        for _ in range(40):observer.accept(dict(kind='firmware_fault',category='panic_banner'))
        self.assertEqual(len(recorder.rows),33);self.assertEqual(recorder.rows[-1]['kind'],'firmware_fault_limit')
        observer.accept(SCHEMA.parse(frame(probe())))
        self.assertNotIn('link_event_gap',observer.problems)


class BoardLinkCoverageTests(unittest.TestCase):
    def test_qualified_prefix_keeps_positive_software_boundaries(self):
        result=analyze(records());self.assertTrue(result['qualified_coverage'],result)
        self.assertEqual([e['phase'] for e in result['events']],['request_pending','TxComplete'])
        self.assertEqual(result['observed_source_sequences'],dict(core=1,lora=1));self.assertIn('Neither establishes RF absence',result['limits'])

    def test_core_deadline_uses_bracketed_offset_not_publication_timestamp(self):
        result=analyze(records());anchor=result['clock_anchors'][BOOT]
        self.assertTrue(result['clock_conversion_qualified'])
        self.assertEqual(anchor['radio_minus_core_ms'],[999,1003])
        self.assertEqual(result['events'][0]['core_in_radio_clock_ms']['timeout_at_ms'],[25099,25103])
        self.assertNotIn('at_ms',result['events'][0]['core_in_radio_clock_ms'])
        bad=probe();bad['radio_after_ms']='1999'
        self.assertIsNone(SCHEMA.parse(frame(bad)))
        self.assertIsNone(SCHEMA.parse(frame(dict(stage='probe',category='installed'))))

    def test_stale_pre_restore_clock_suppresses_conversion_not_positive_evidence(self):
        data=records();event=next(r for r in data if r.get('fields',{}).get('family')=='core')
        # Reproduce the copied boot clock versus resumed persistent core clock.
        event['fields'].update(has_at='true',at_ms='1701002142')
        result=analyze(data)
        self.assertTrue(result['qualified_coverage'])
        self.assertFalse(result['clock_conversion_qualified'])
        self.assertEqual(result['clock_anchors'],{})
        self.assertEqual(result['events'][0]['core_in_radio_clock_ms'],{})
        self.assertEqual(result['events'][0]['at_ms'],'1701002142')
        self.assertEqual(len(result['events']),2)
        self.assertEqual(result['clock_problems'][0]['category'],'clock_anchor_inconsistent')
        self.assertEqual(result['clock_problems'][0]['board_publication_ms'],6500)

    def test_clock_guard_respects_rounding_and_uses_event_not_deadline(self):
        for at,qualified in (('2500',True),('5501',True),('5502',False)):
            data=records();event=next(r for r in data if r.get('fields',{}).get('family')=='core')
            event['fields'].update(has_at='true',at_ms=at)
            result=analyze(data)
            self.assertEqual(result['clock_conversion_qualified'],qualified,at)
        # A planned deadline in the future is normal, and unknown at_ms is not
        # an observation of a future source event.
        data=records();event=next(r for r in data if r.get('fields',{}).get('family')=='core')
        event['fields'].update(has_at='false',at_ms='1701002142')
        self.assertTrue(analyze(data)['clock_conversion_qualified'])

    def test_known_request_time_catches_stale_clock_without_optional_at(self):
        data=records();event=next(r for r in data if r.get('fields',{}).get('family')=='core')
        event['fields'].update(requested_at_ms='1701002142')
        self.assertEqual(event['fields']['has_at'],'false')
        result=analyze(data)
        self.assertTrue(result['qualified_coverage'])
        self.assertFalse(result['clock_conversion_qualified'])
        self.assertEqual(result['clock_problems'][0]['core_field'],'requested_at_ms')
        self.assertEqual(result['events'][0]['core_in_radio_clock_ms'],{})

    def test_every_lost_frame_including_last_event_and_probe_fails(self):
        original=records()
        for index,r in enumerate(original):
            if r['kind'] in BOARD.KINDS and r['host_monotonic_ns']<=20e9:
                data=copy.deepcopy(original);data.pop(index)
                self.assertFalse(analyze(data)['qualified_coverage'],r)

    def test_tail_watermark_source_and_publication_must_reconcile(self):
        for change in (dict(core_seen='2'),dict(core_admitted='2'),dict(core_published='2'),dict(healthy='false'),dict(core_capped='true'),dict(sink_dropped='1')):
            data=records();target=next(r for r in data if r['kind']=='link_health' and r['host_monotonic_ns']==20e9);target['fields'].update(change)
            self.assertFalse(analyze(data)['qualified_coverage'],change)

    def test_earlier_pending_writer_can_reconcile(self):
        data=records();target=next(r for r in data if r['kind']=='link_health' and r['host_monotonic_ns']==6e9)
        target['fields'].update(core_seen='1',core_admitted='1',healthy='false')
        self.assertTrue(analyze(data)['qualified_coverage'])

    def test_unhealthy_drained_prefix_cannot_be_healed_by_a_later_true(self):
        data=records();next(r for r in data if r['kind']=='link_health')['fields']['healthy']='false'
        self.assertFalse(analyze(data)['qualified_coverage'])

    def test_source_loss_cap_unknown_and_impossible_counters_fail(self):
        for change in (dict(source_sequence='2'),dict(phase='LimitReached'),dict(phase='UnknownPhase'),dict(producer_dropped='1'),dict(sink_dropped='1')):
            data=records();next(r for r in data if r.get('fields',{}).get('family')=='lora')['fields'].update(change)
            self.assertFalse(analyze(data)['qualified_coverage'],change)
        data=records();next(r for r in data if r['kind']=='link_health')['fields']['core_seen']='1'
        self.assertFalse(analyze(data)['qualified_coverage'])

    def test_capture_loss_host_gap_board_gap_and_missing_closing_anchors_fail(self):
        for name in ('rejected_link_lines','undecodable_lines','oversized_lines'):
            data=records();next(r for r in data if r['kind']=='capture_health')[name]=1
            self.assertFalse(analyze(data)['qualified_coverage'])
        for kind in ('capture_health','link_health'):
            data=[r for r in records() if not(r['kind']==kind and r['host_monotonic_ns']>10e9)]
            result=analyze(data);self.assertFalse(result['qualified_coverage']);self.assertEqual(len(result['events']),2)

    def test_old_probe_does_not_qualify_new_boot_outside_window(self):
        data=records()
        for r in data:
            if r['kind'] in BOARD.KINDS and r['host_monotonic_ns']>=10e9:r['boot_token']='0'*16
        self.assertFalse(analyze(data,start=15,end=20)['qualified_coverage'])

    def test_prior_failure_persists_but_later_failure_does_not_rewrite_window(self):
        data=records();bad=row(1,'observation_gap',reason='link_event_gap');data.append(bad)
        self.assertFalse(analyze(data)['qualified_coverage'])
        data=records();data.append(row(25,'capture_ended',reason='duration_complete'))
        self.assertTrue(analyze(data)['qualified_coverage'])

    def test_fault_after_probe_fails_but_preprobe_or_later_fault_is_context(self):
        for second,expected in ((.05,True),(1,False),(25,True)):
            data=records();data.append(row(second,'firmware_fault',category='stack_guard'))
            self.assertEqual(analyze(data)['qualified_coverage'],expected)
        data=records();data.append(row(.05,'firmware_fault_limit',limit=32))
        self.assertFalse(analyze(data)['qualified_coverage'])
        data=[r for r in records() if r['host_monotonic_ns']<15e9];data.append(row(14,'firmware_fault',category='stack_guard'))
        result=analyze(data)
        self.assertIn('firmware_fault_observed',result['problems']);self.assertIn('closing_board_heartbeat_missing',result['problems'])
        self.assertEqual(result['firmware_faults'][0]['category'],'stack_guard')

    def test_legacy_caps_do_not_consume_link_budget(self):
        data=records();data.append(row(3,'board_diagnostics_limit',limit=2048));data.append(row(4,'queue_diagnostics_limit',limit=512))
        self.assertTrue(analyze(data)['qualified_coverage'])

    def test_missing_capture_contract_unknown_fields_and_regressing_clocks_fail(self):
        data=records();data[0]['link_heartbeat_expected']=False;self.assertFalse(analyze(data)['qualified_coverage'])
        data=records();next(r for r in data if r['kind']=='link_event')['fields']['secret']='PRIVATE';self.assertFalse(analyze(data)['qualified_coverage'])
        data=records();next(r for r in data if r['kind']=='link_event')['board_observed_ms']=0;self.assertFalse(analyze(data)['qualified_coverage'])


class CombinedTimelineTests(unittest.TestCase):
    def test_combined_timeline_suppresses_stale_clock_conversion(self):
        join=BOARD.sibling('handshake_timeline')
        data=records();next(r for r in data if r.get('fields',{}).get('family')=='core')['fields'].update(has_at='true',at_ms='1701002142')
        with tempfile.TemporaryDirectory() as directory:
            capture_path=Path(directory)/'board.jsonl'
            capture_path.write_text(''.join(json.dumps(r)+'\n' for r in data))
            result=join.analyze({'board':('board',capture_path)},base.OBSERVER.epoch(base.stamp(5)),base.OBSERVER.epoch(base.stamp(20)),same_host=True)
        self.assertTrue(result['sources']['board']['qualified_coverage'])
        self.assertFalse(result['sources']['board']['clock_conversion_qualified'])
        core=next(e for link in result['links'] for e in link['events'] if e['family']=='core')
        self.assertEqual(core['core_in_radio_clock_ms'],{})
        self.assertEqual(core['at_ms'],'1701002142')

    def test_shared_link_token_joins_sources_with_separate_coverage(self):
        join=BOARD.sibling('handshake_timeline')
        with tempfile.TemporaryDirectory() as directory:
            a=Path(directory)/'board.jsonl';b=Path(directory)/'phone.jsonl'
            a.write_text(''.join(json.dumps(r)+'\n' for r in records()))
            # Native fixture lacks its closing heartbeat: retain positive phases
            # while its coverage remains unqualified.
            b.write_text(''.join(json.dumps(r)+'\n' for r in native.records() if r['host_monotonic_ns']<20e9))
            result=join.analyze({'board':('board',a),'phone':('phone',b)},base.OBSERVER.epoch(base.stamp(5)),base.OBSERVER.epoch(base.stamp(20)),same_host=True)
        self.assertEqual(len(result['links']),1)
        self.assertEqual(result['links'][0]['sources_observed'],['board','phone'])
        self.assertTrue(result['sources']['board']['qualified_coverage'])
        self.assertFalse(result['all_sources_qualified'])
        self.assertIn('not RF latency',result['limits'])


if __name__=='__main__':unittest.main()
