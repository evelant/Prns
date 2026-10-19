"""No missing/capped Link trace can become a complete handshake explanation."""
import importlib.util
from pathlib import Path
import unittest
from tools.tests import test_group_trial_observer as base

path = Path(__file__).resolve().parents[1] / 'device/group-trial/link_timeline.py'
spec = importlib.util.spec_from_file_location('link_timeline', path)
TIMELINE = importlib.util.module_from_spec(spec)
spec.loader.exec_module(TIMELINE)


def event(second, fields):
    return {'schema': 1, 'capture_id': 'capture', 'kind': 'event', 'tag': 'prns.links', 'fields': fields,
            'host_observed_utc': base.stamp(second), 'host_monotonic_ns': int(second * 1e9)}


def core(sequence='1', source='1', phase='request_pending'):
    result = dict(stage='handshake', family='core', sequence=sequence, source_sequence=source, dropped_before='0',
                  link='1111111111111111', has_link='true', interface='2222222222222222', has_interface='true',
                  command='7', has_command='true', at_ms='0', has_at='false', phase=phase)
    if phase == 'request_pending':
        result.update(requested_at_ms='100', timeout_at_ms='24100', hops='3', bitrate='0', has_bitrate='false',
                      first_hop_floor_ms='0', has_floor='false', computed_timeout_ms='24000', route_generation='5')
    else:
        result.update(requested_at_ms='100', route_generation='5', detail='Removed')
    return result


def records():
    data = base.rows()
    for row in data:
        f = row.get('fields', {})
        if f.get('stage') == 'heartbeat':
            seconds = int(f['elapsed_ms']) / 1000
            f.update(links_ready='true', link_sequence=str(sum(seconds >= n for n in (6, 7, 14))))
    data.extend([event(0, {'stage': 'probe', 'category': 'installed'}), event(6, core()),
                 event(7, dict(stage='handshake', family='egress', sequence='2', source_sequence='1',
                    link='1111111111111111', has_link='true', interface='2222222222222222', has_interface='true',
                    packet='request', phase='Unavailable', queue_age_ms='0', has_age='false',
                    attempted='1', emitted='0', producer_dropped='0', sink_dropped='0')),
                 event(14, core('3', '2', 'initiator_timeout'))])
    data.sort(key=lambda r: r['host_monotonic_ns'])
    for index, row in enumerate(data, 1): row['sequence'] = index
    return data


def analyze(data):
    data.sort(key=lambda row: row['host_monotonic_ns'])
    for index, row in enumerate(data, 1): row['sequence'] = index
    return TIMELINE.analyze(data, base.OBSERVER.epoch(base.stamp(5)), base.OBSERVER.epoch(base.stamp(20)))


class LinkTimelineTests(unittest.TestCase):
    def shifted_host_records(self):
        data=records()
        for row in data:
            if row.get('kind')=='observer_heartbeat':
                second=row['host_monotonic_ns']/1e9+2
                row.update(host_monotonic_ns=int(second*1e9),host_observed_utc=base.stamp(second))
        return data

    def test_independently_phased_closing_host_heartbeat_qualifies_same_window(self):
        result=analyze(self.shifted_host_records())
        self.assertTrue(result['qualified_coverage'],result)
        self.assertGreater(result['through_capture_sequence'],result['through_native_capture_sequence'])
        self.assertEqual(result['sink_watermark'],3)
        self.assertEqual(len(result['events']),3)

    def test_missing_or_gapped_host_stream_still_fails(self):
        for remove in (lambda t:t>=20,lambda t:5<t<20):
            data=[r for r in self.shifted_host_records() if not(r.get('kind')=='observer_heartbeat' and remove(r['host_monotonic_ns']/1e9))]
            result=analyze(data)
            self.assertFalse(result['qualified_coverage'])
            self.assertIn('host_coverage_gap',result['problems'])

    def test_interruption_before_independent_host_close_fails_but_later_does_not(self):
        for second,expected in ((21,False),(23,True)):
            data=self.shifted_host_records()
            data.append(dict(schema=1,capture_id='capture',kind='capture_ended',host_observed_utc=base.stamp(second),host_monotonic_ns=int(second*1e9)))
            self.assertEqual(analyze(data)['qualified_coverage'],expected)

    def test_positive_egress_refusal_is_distinct_from_timeout(self):
        result = analyze(records())
        self.assertTrue(result['qualified_coverage'], result)
        self.assertEqual([r['phase'] for r in result['events']], ['request_pending', 'Unavailable', 'initiator_timeout'])
        self.assertIn('durable delivery are not established', result['limits'])

    def test_every_missing_event_is_unqualified_including_final_tail(self):
        for seq in ('1', '2', '3'):
            data = records()
            data = [r for r in data if not (r.get('tag') == 'prns.links' and r.get('fields', {}).get('sequence') == seq)]
            self.assertFalse(analyze(data)['qualified_coverage'])

    def test_caps_sink_loss_and_reset_fail_closed(self):
        for change in ({'dropped_before': '1'}, {'source_sequence': '1'}, {'phase': 'future_phase'}):
            data = records()
            target = next(r['fields'] for r in data if r.get('fields', {}).get('phase') == 'initiator_timeout')
            target.update(change)
            self.assertFalse(analyze(data)['qualified_coverage'])
        data = records()
        data.insert(-1, event(12, {'stage': 'diagnostics', 'category': 'sink_limit', 'limit': '8192'}))
        self.assertFalse(analyze(data)['qualified_coverage'])

    def test_lost_limit_notice_still_fails_from_health(self):
        data = records()
        for row in data:
            if row.get('fields', {}).get('elapsed_ms') == '20000': row['fields']['links_ready'] = 'false'
        self.assertFalse(analyze(data)['qualified_coverage'])

    def test_unrelated_delivery_cap_does_not_invalidate_link_prefix(self):
        data = records()
        target = next(r for r in data if r.get('fields', {}).get('stage') == 'local_send_result')
        target['fields'] = {'stage': 'diagnostics', 'category': 'delivery_line_limit'}
        self.assertTrue(analyze(data)['qualified_coverage'])

    def test_metadata_parser_and_closed_schema(self):
        fields = core()
        line = '[prns.links] ' + ' '.join(f'{k}={v}' for k,v in fields.items())
        parsed = base.OBSERVER.parse_safe_line(line)
        self.assertTrue(TIMELINE.valid(parsed['fields']))
        self.assertIsNone(base.OBSERVER.parse_safe_line(line, expected_pid=23))
        fields.update(phase='proof_completion', detail='WriteRejected(NotPending)')
        for key in TIMELINE.CORE_PHASES['request_pending']: fields.pop(key)
        self.assertTrue(TIMELINE.valid(fields))
        fields['detail'] = 'PrivatePayload'
        self.assertFalse(TIMELINE.valid(fields))
        fields['secret'] = 'data'
        self.assertFalse(TIMELINE.valid(fields))

    def test_session_gap_and_heartbeat_absence_are_unqualified(self):
        for kind in ('session', 'sequence', 'missing'):
            data = records()
            target = next(r for r in data if r.get('fields', {}).get('elapsed_ms') == '20000')
            if kind == 'missing':
                data = [r for r in data if r['host_monotonic_ns'] < 20e9]
            else: target['fields'][kind] = 'other' if kind == 'session' else '99'
            self.assertFalse(analyze(data)['qualified_coverage'])

    def test_old_probe_cannot_qualify_new_session_outside_window(self):
        data = [r for r in records() if r.get('fields', {}).get('stage') != 'handshake']
        for row in data:
            f = row.get('fields', {})
            if f.get('stage') == 'heartbeat':
                seconds = int(f['elapsed_ms']) / 1000
                f['link_sequence'] = '0'
                if seconds >= 10:
                    f['session'] = 'restarted'
                    f['sequence'] = str(int(seconds / 5) - 1)
        result = TIMELINE.analyze(data, base.OBSERVER.epoch(base.stamp(25)), base.OBSERVER.epoch(base.stamp(30)))
        self.assertFalse(result['qualified_coverage'])
        self.assertIn('native_session_changed', result['problems'])

    def test_old_capture_has_no_link_coverage(self):
        self.assertFalse(analyze(base.rows())['qualified_coverage'])


if __name__ == '__main__': unittest.main()
