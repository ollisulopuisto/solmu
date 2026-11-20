import contextlib
import importlib.util
import io
import json
import os
import pathlib
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('inventory', pathlib.Path(__file__).with_name('check_endpoint.py'))
inventory = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inventory)

class InventoryTests(unittest.TestCase):
    def test_malformed_pages_never_pass_as_an_empty_healthy_inventory(self):
        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *args): self.close()
        cases = [
            ([{}], None),
            ([{'accountID': 'fixture'}], {}),
            ([], {'error': 'private server failure'}),
            ([], {'items': [None]}),
            ([], {'items': [{'accountID': 'fixture'}]}),
            ([], {'items': [], 'hasMore': 'false'}),
            ([], {'items': [], 'hasMore': True, 'oldestCursor': ['bad']}),
        ]
        for accounts, page in cases:
            with self.subTest(accounts=accounts, page=page):
                opener = mock.Mock()
                payloads = [accounts] if page is None else [accounts, page]
                opener.open.side_effect = [Response(json.dumps(p).encode()) for p in payloads]
                with mock.patch.object(inventory.urllib.request, 'build_opener', return_value=opener):
                    with self.assertRaises(ValueError) as failure:
                        inventory.check('http://127.0.0.1:3000', 'fixture')
                self.assertNotIn('private server failure', str(failure.exception))

    def test_missing_history_items_never_report_zero_messages(self):
        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *args): self.close()
        opener = mock.Mock()
        payloads = [[], {'items': [{'id': 'fixture-chat', 'accountID': 'fixture'}]}, {}]
        opener.open.side_effect = [Response(json.dumps(p).encode()) for p in payloads]
        with mock.patch.object(inventory.urllib.request, 'build_opener', return_value=opener):
            with self.assertRaisesRegex(ValueError, 'history'):
                inventory.check('http://127.0.0.1:3000', 'fixture', True)

    def test_invalid_inventory_exits_without_a_success_summary_or_private_payload(self):
        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *args): self.close()
        opener = mock.Mock()
        opener.open.side_effect = [Response(b'[]'), Response(b'{"error":"private payload"}')]
        output, errors = io.StringIO(), io.StringIO()
        with mock.patch('sys.argv', ['check_endpoint.py', '--base-url', 'http://127.0.0.1:3000']), \
             mock.patch.dict(os.environ, {'BEEPER_ACCESS_TOKEN': 'fixture-secret'}), \
             mock.patch.object(inventory.urllib.request, 'build_opener', return_value=opener), \
             contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
            self.assertEqual(inventory.main(), 1)
        self.assertEqual(output.getvalue(), '')
        self.assertNotIn('private payload', errors.getvalue())
        self.assertNotIn('fixture-secret', errors.getvalue())

    def test_empty_valid_inventory_is_not_confused_with_a_malformed_page(self):
        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *args): self.close()
        opener = mock.Mock()
        opener.open.side_effect = [Response(b'[]'), Response(b'{"items":[],"hasMore":false}')]
        with mock.patch.object(inventory.urllib.request, 'build_opener', return_value=opener):
            result = inventory.check('http://127.0.0.1:3000', 'fixture')
        self.assertEqual(result['chat_count'], 0)
        self.assertEqual(result['accounts'], [])

    def test_endpoint_validation_happens_before_prompt(self):
        with mock.patch('sys.argv', ['check_endpoint.py', '--base-url', 'http://127.0.0.1?token=secret']), mock.patch.dict(os.environ, {}, clear=True), mock.patch.object(inventory.getpass, 'getpass') as prompt, contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(inventory.main(), 1)
            prompt.assert_not_called()

    def test_named_target_never_inherits_desktop_token(self):
        with mock.patch('sys.argv', ['check_endpoint.py', '--beeper-target', 'trial', '--sample-history']), mock.patch.dict(os.environ, {'BEEPER_ACCESS_TOKEN': 'wrong-desktop'}), mock.patch.object(inventory, 'resolve_beeper_target', return_value=('http://127.0.0.1:3000', 'target-only')), mock.patch.object(inventory, 'check', return_value={'read_only': True}) as check, contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(inventory.main(), 0)
            check.assert_called_once_with('http://127.0.0.1:3000', 'target-only', True)

    def test_oversized_response_is_refused_without_loading_unbounded_body(self):
        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *args): self.close()
        opener = mock.Mock()
        opener.open.return_value = Response(b' ' * (16 * 1024 * 1024 + 2))
        with mock.patch.object(inventory.urllib.request, 'build_opener', return_value=opener), self.assertRaisesRegex(ValueError, 'size limit'):
            inventory.check('http://127.0.0.1:3000', 'fixture')

    def test_expected_counts_fail_without_claiming_full_network_health(self):
        output = io.StringIO()
        with mock.patch('sys.argv', ['check_endpoint.py', '--base-url', 'http://127.0.0.1:3000', '--expect-accounts', '2', '--expect-chats', '5']), mock.patch.dict(os.environ, {'BEEPER_ACCESS_TOKEN': 'fixture'}), mock.patch.object(inventory, 'check', return_value={'read_only': True, 'accounts': [{}], 'chat_count': 0}), contextlib.redirect_stdout(output):
            self.assertEqual(inventory.main(), 2)
        result = json.loads(output.getvalue())
        self.assertFalse(result['counts_match'])
        self.assertFalse(result['full_network_health_verified'])

    def test_empty_advancing_pages_are_bounded(self):
        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *args): self.close()
        opener = mock.Mock()
        count = 0
        def response(*args, **kwargs):
            nonlocal count
            count += 1
            if count > 1002:
                raise RuntimeError('unbounded empty pagination')
            payload = [] if count == 1 else {'items': [], 'hasMore': True, 'oldestCursor': str(count)}
            return Response(json.dumps(payload).encode())
        opener.open.side_effect = response
        with mock.patch.object(inventory.urllib.request, 'build_opener', return_value=opener), self.assertRaisesRegex(ValueError, 'page limit'):
            inventory.check('http://127.0.0.1:3000', 'fixture')

    def test_final_page_cannot_exceed_inventory_bound(self):
        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *args): self.close()
        opener = mock.Mock()
        opener.open.side_effect = [Response(b'[]'), Response(json.dumps({'items':[{'id':'x'}] * 100001, 'hasMore':False}).encode())]
        with mock.patch.object(inventory.urllib.request, 'build_opener', return_value=opener), self.assertRaisesRegex(ValueError, 'size limit'):
            inventory.check('http://127.0.0.1:3000', 'fixture')

    def test_credential_bearing_errors_are_not_printed(self):
        stderr = io.StringIO()
        with mock.patch('sys.argv', ['check_endpoint.py', '--base-url', 'https://endpoint.example']), mock.patch.dict(os.environ, {'BEEPER_ACCESS_TOKEN': 'secret-token'}), mock.patch.object(inventory, 'check', side_effect=ValueError('secret-token private account secret-cookie')), contextlib.redirect_stderr(stderr):
            self.assertEqual(inventory.main(), 1)
        for text in ('secret-token','private account','secret-cookie','endpoint.example'):
            self.assertNotIn(text,stderr.getvalue())

    def test_redirect_rejected_and_loopback_ignores_proxy(self):
        from test_run_client import Handler
        import http.server
        import threading
        Handler.status = 302
        Handler.requests = []
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever)
        worker.start()
        try:
            with mock.patch.dict(os.environ, {'http_proxy':'http://127.0.0.1:1','HTTP_PROXY':'http://127.0.0.1:1','NO_PROXY':'','no_proxy':''}), self.assertRaisesRegex(ValueError,'redirected'):
                inventory.check('http://127.0.0.1:%s' % server.server_port, 'local-only')
            self.assertEqual(len(Handler.requests),1)
            self.assertEqual(Handler.requests[0],('/v1/accounts','Bearer local-only'))
        finally:
            server.shutdown();server.server_close();worker.join()

    def test_current_statuses_and_providers_survive_redaction(self):
        statuses = ['backfilling', 'connection_required', 'reconnect_required', 'attention_required', 'disabled']
        providers = ['self-hosted', 'platform-sdk']
        payloads = [[{'accountID': 'private-' + str(i), 'status': status, 'bridge': {'type': 'signal', 'provider': providers[i % 2]}} for i, status in enumerate(statuses)], {'items': []}]
        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *args): self.close()
        opener = mock.Mock()
        opener.open.side_effect = [Response(json.dumps(p).encode()) for p in payloads]
        with mock.patch.object(inventory.urllib.request, 'build_opener', return_value=opener):
            result = inventory.check('http://127.0.0.1:3000', 'fixture')
        self.assertEqual([a['status'] for a in result['accounts']], statuses)
        self.assertEqual([a['provider'] for a in result['accounts']], [providers[i % 2] for i in range(len(statuses))])
        self.assertNotIn('private-', json.dumps(result))

    def test_summary_never_reveals_inventory_identifiers(self):
        payloads = [
            [{'accountID': 'private-account', 'user': {'fullName': 'Private Name'}, 'bridge': {'type': 'signal', 'provider': 'local'}, 'status': 'connected'}],
            {'items': [{'id': 'private-chat', 'accountID': 'private-account', 'title': 'Private title'}]},
            {'items': [{'text': 'Private message'}], 'hasMore': True, 'oldestCursor': 'private-cursor'},
        ]
        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *args): self.close()
        opener = mock.Mock()
        opener.open.side_effect = [Response(json.dumps(p).encode()) for p in payloads]
        with mock.patch.object(inventory.urllib.request, 'build_opener', return_value=opener):
            result = inventory.check('http://127.0.0.1:3000', 'secret-token', True)
        output = json.dumps(result)
        for private in ['private-account', 'private-chat', 'Private Name', 'Private title', 'Private message', 'secret-token']:
            self.assertNotIn(private, output)
        self.assertEqual(result['chat_count'], 1)
        self.assertEqual(result['sample_history']['messages'], 1)
        self.assertEqual([c.args[0].get_method() for c in opener.open.call_args_list], ['GET'] * 3)

if __name__ == '__main__': unittest.main()
