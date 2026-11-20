#!/usr/bin/env python3
"""Synthetic acceptance: a real daemon sends after the enqueue client exits.

Usage: python3 closed_client.py --binary /path/to/solmu-scheduler
Only ephemeral loopback servers and fixture tokens are used, never accounts.
"""
import argparse
import http.server
import json
import os
import pathlib
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    args = parser.parse_args()
    sends = []

    class Beeper(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def answer(self, payload):
            body = json.dumps(payload).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.path == '/v1/chats/fixture-chat':
                self.answer({'id': 'fixture-chat', 'accountID': 'fixture-whatsapp'})
            else:
                self.send_error(404)

        def do_POST(self):
            if self.path != '/v1/chats/fixture-chat/messages':
                self.send_error(404)
                return
            sends.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
            self.answer({'pendingMessageID': 'fixture-sent'})

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Beeper)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    daemon = None
    try:
        with tempfile.TemporaryDirectory(prefix='solmu-closed-client-') as work:
            with socket.socket() as reservation:
                reservation.bind(('127.0.0.1', 0))
                port = reservation.getsockname()[1]
            base = 'http://127.0.0.1:%s' % port
            env = dict(os.environ, BEEPER_BASE_URL='http://127.0.0.1:%s' % server.server_port,
                       BEEPER_ACCESS_TOKEN='fixture-api', SOLMU_CONTROL_TOKEN='fixture-control',
                       SOLMU_QUEUE_PATH=str(pathlib.Path(work) / 'queue.json'),
                       SOLMU_LISTEN='127.0.0.1:%s' % port, SOLMU_TICK_SECS='1')
            daemon = subprocess.Popen([os.path.abspath(args.binary)], env=env,
                                      stdout=subprocess.PIPE, stderr=subprocess.STDOUT)

            def request(path, payload=None):
                data = None if payload is None else json.dumps(payload).encode()
                req = urllib.request.Request(base + path, data=data, headers={
                    'Authorization': 'Bearer fixture-control', 'Content-Type': 'application/json'})
                with urllib.request.urlopen(req, timeout=3) as response:
                    return json.load(response)

            deadline = time.monotonic() + 10
            while True:
                try:
                    request('/v1/health')
                    break
                except (OSError, urllib.error.URLError):
                    if time.monotonic() >= deadline or daemon.poll() is not None:
                        raise RuntimeError('fixture daemon did not become ready')
                    time.sleep(0.05)

            payload = {'account_id': '1', 'chat_id': 'fixture-chat',
                       'text': 'Synthetic closed-client acceptance', 'due_at': int(time.time()) + 5}
            try:
                request('/v1/scheduled', payload)
                raise AssertionError('local profile ID was accepted as a bridge identity')
            except urllib.error.HTTPError as error:
                assert error.code == 400
                error.close()
            assert request('/v1/scheduled') == []
            payload['account_id'] = 'fixture-whatsapp'
            # A separate client process enqueues and exits before the due time.
            client = subprocess.run([sys.executable, '-c',
                'import json,sys,urllib.request; '
                'r=urllib.request.Request(sys.argv[1],data=sys.stdin.buffer.read(),'
                'headers={"Authorization":"Bearer fixture-control","Content-Type":"application/json"}); '
                'print(json.load(urllib.request.urlopen(r,timeout=3))["id"])',
                base + '/v1/scheduled'], input=json.dumps(payload).encode(), capture_output=True, timeout=5)
            assert client.returncode == 0, 'synthetic enqueue client failed'
            item_id = client.stdout.decode().strip()
            assert item_id
            assert time.time() < payload['due_at'], 'enqueue client did not exit before due time'
            assert sends == [], 'message sent before due time'
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                items = request('/v1/scheduled')
                if items and items[0]['state'] == 'Sent':
                    break
                time.sleep(0.1)
            assert items[0]['id'] == item_id
            assert items[0]['state'] == 'Sent'
            assert items[0]['account_id'] == 'fixture-whatsapp'
            assert items[0]['sent_message_id'] == 'fixture-sent'
            assert [send['text'] for send in sends] == [payload['text']]
            time.sleep(2)
            assert len(sends) == 1, 'message was sent more than once'
            print('PASS: wrong account rejected; separate enqueue client exited before due time; daemon sent exact text once and persisted Sent.')
    finally:
        if daemon is not None:
            daemon.terminate()
            try:
                daemon.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                daemon.kill()
                daemon.communicate(timeout=5)
        server.shutdown()
        server.server_close()
        thread.join(timeout=3)


if __name__ == '__main__':
    main()
