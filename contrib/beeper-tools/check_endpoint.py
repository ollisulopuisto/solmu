#!/usr/bin/env python3
"""Read-only local Beeper inventory check. Never prints tokens, IDs or chat text."""
import argparse
import getpass
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from collections import Counter
from run_client import validate_endpoint, resolve_beeper_target, LOOPBACK


def page_items(page, kind):
    # Error-shaped or partial JSON is not evidence of an empty inventory.
    # Keep diagnostics static: response text may contain private chat data.
    if not isinstance(page, dict) or not isinstance(page.get('items'), list):
        raise ValueError('%s response was not an items page' % kind)
    if any(not isinstance(item, dict) for item in page['items']):
        raise ValueError('%s items were not objects' % kind)
    if 'hasMore' in page and not isinstance(page['hasMore'], bool):
        raise ValueError('%s pagination flag was not a boolean' % kind)
    if page.get('hasMore') and (
            not isinstance(page.get('oldestCursor'), str) or not page['oldestCursor']):
        raise ValueError('%s pagination cursor was missing or invalid' % kind)
    return page['items']


def check(base, token, sample_history=False):
    parsed = validate_endpoint(base)
    handlers = [NoRedirect()]
    if parsed.hostname in LOOPBACK:
        handlers.append(urllib.request.ProxyHandler({}))
    opener = urllib.request.build_opener(*handlers)
    def get(path, query=None):
        url = base.rstrip('/') + path
        if query:
            url += '?' + urllib.parse.urlencode(query)
        request = urllib.request.Request(url, headers={'Authorization': 'Bearer ' + token})
        with opener.open(request, timeout=20) as response:
            body = response.read(16 * 1024 * 1024 + 1)
            if len(body) > 16 * 1024 * 1024:
                raise ValueError('response exceeded size limit')
            return json.loads(body)
    accounts = get('/v1/accounts')
    if not isinstance(accounts, list) or any(
            not isinstance(account, dict)
            or not isinstance(account.get('accountID'), str)
            or not account['accountID'] for account in accounts):
        raise ValueError('accounts response was not a valid account array')
    rows, seen, cursor = [], set(), None
    pages = 0
    while True:
        pages += 1
        if pages > 1000:
            raise ValueError('inventory exceeded page limit')
        query = {'limit': 100}
        if cursor:
            query.update(cursor=cursor, direction='before')
        page = get('/v1/chats', query)
        items = page_items(page, 'chat inventory')
        if len(rows) + len(items) > 100000:
            raise ValueError('inventory exceeded size limit')
        if any(not isinstance(row.get('id'), str) or not row['id']
               or not isinstance(row.get('accountID'), str) or not row['accountID']
               for row in items):
            raise ValueError('chat inventory contained missing or invalid identities')
        rows.extend(items)
        if not page.get('hasMore'):
            break
        cursor = page.get('oldestCursor')
        if not cursor or cursor in seen:
            raise ValueError('chat pagination did not advance')
        seen.add(cursor)
    by_account = Counter(row.get('accountID', '') for row in rows)
    account_ids = {a.get('accountID', '') for a in accounts}
    summary = []
    for a in accounts:
        bridge = a.get('bridge') or {}
        network = bridge.get('type') or bridge.get('id') or 'unknown'
        allowed_networks = {'whatsapp', 'signal', 'telegram', 'instagram', 'facebook', 'messenger', 'linkedin', 'twitter', 'x', 'matrix', 'beeper', 'slack', 'slackgo', 'discord', 'discordgo', 'imessage', 'googlemessages', 'googlechat', 'googlevoice'}
        provider = bridge.get('provider', 'unknown')
        status = a.get('status', 'unknown')
        summary.append({'network': network if network in allowed_networks else 'other',
                        'provider': provider if provider in {'cloud', 'local', 'on-device', 'self-hosted', 'platform-sdk'} else 'unknown',
                        'status': status if status in {'connected', 'disconnected', 'connecting', 'error', 'backfilling', 'connection_required', 'reconnect_required', 'attention_required', 'disabled'} else 'unknown',
                        'chats': by_account[a.get('accountID', '')]})
    result = {'read_only': True, 'accounts': summary,
              'chat_count': len({row.get('id') for row in rows}),
              'unmatched_chat_account_count': len(set(by_account) - account_ids)}
    if sample_history and rows:
        page = get('/v1/chats/' + urllib.parse.quote(rows[0]['id'], safe='') + '/messages')
        items = page_items(page, 'sample history')
        result['sample_history'] = {'messages': len(items),
                                    'has_more': bool(page.get('hasMore'))}
    return result


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError('Endpoint redirected. Check its base URL; no token was forwarded.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    route = parser.add_mutually_exclusive_group()
    route.add_argument('--base-url')
    route.add_argument('--beeper-target', help='Read exactly one existing named target, never install/start/reconnect')
    parser.add_argument('--label', choices=('desktop', 'headless', 'other'), default='desktop', help='Label the endpoint being checked, not a migration')
    parser.add_argument('--sample-history', action='store_true', help='GET one history page; never mark read')
    parser.add_argument('--expect-accounts', type=int, help='Expected account count from a previous local check; mismatch exits2')
    parser.add_argument('--expect-chats', type=int, help='Expected total chat count; mismatch exits2')
    args = parser.parse_args()
    if any(value is not None and value < 0 for value in (args.expect_accounts, args.expect_chats)):
        parser.error('Expected counts must be non-negative')
    try:
        target_token = None
        if args.beeper_target:
            args.base_url, target_token = resolve_beeper_target(args.beeper_target)
        args.base_url = args.base_url or 'http://127.0.0.1:23373'
        validate_endpoint(args.base_url)
        token = (target_token if args.beeper_target else os.environ.get('BEEPER_ACCESS_TOKEN')) or getpass.getpass('Beeper API token (local terminal only): ')
        result = check(args.base_url, token, args.sample_history)
        result['endpoint_kind'] = args.label
        result['full_network_health_verified'] = False
        result['counts_match'] = (
            (args.expect_accounts is None or len(result.get('accounts', [])) == args.expect_accounts)
            and (args.expect_chats is None or result.get('chat_count') == args.expect_chats)
        )
        print(json.dumps(result, indent=2))
        if not result['counts_match']:
            return 2
    except urllib.error.HTTPError as exc:
        print('Endpoint returned HTTP %s. Check URL and token locally.' % exc.code, file=sys.stderr)
        return 1
    except Exception as exc:
        # Avoid URLs, server payloads or credential-bearing exception messages.
        print('Endpoint check failed (%s). Check local connectivity and API configuration.' % type(exc).__name__, file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
