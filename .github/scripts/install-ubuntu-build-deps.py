#!/usr/bin/env python3
"""Bounded apt install with an official Ubuntu mirror fallback for CI."""
import contextlib
import ipaddress
import pathlib
import tempfile
import subprocess
import sys


def use_official_mirrors(root):
    apt = root / 'etc/apt'
    paths = [apt / 'sources.list', apt / 'apt-mirrors.txt', apt / 'apt-mirrors-security.txt']
    paths += sorted((apt / 'sources.list.d').glob('*.list'))
    paths += sorted((apt / 'sources.list.d').glob('*.sources'))
    replacements = {
        'mirror+file:/etc/apt/apt-mirrors-security.txt': 'https://security.ubuntu.com/ubuntu',
        'mirror+file:/etc/apt/apt-mirrors.txt': 'https://archive.ubuntu.com/ubuntu',
        'http://azure.archive.ubuntu.com/ubuntu': 'https://archive.ubuntu.com/ubuntu',
        'https://azure.archive.ubuntu.com/ubuntu': 'https://archive.ubuntu.com/ubuntu',
        'http://archive.ubuntu.com/ubuntu': 'https://archive.ubuntu.com/ubuntu',
        'http://security.ubuntu.com/ubuntu': 'https://security.ubuntu.com/ubuntu',
    }
    changed = 0
    for path in paths:
        if not path.is_file():
            continue
        old = path.read_text()
        new = old
        for before, after in replacements.items():
            new = new.replace(before, after)
        if new != old:
            path.write_text(new)
            changed += 1
    return changed


@contextlib.contextmanager
def ubuntu_hosts(root, query=subprocess.check_output):
    """Fresh fixed-host DNS answers, visible only in each apt child's mount NS."""
    lines = []
    for host in ['archive.ubuntu.com', 'security.ubuntu.com', 'azure.archive.ubuntu.com']:
        answer = query(['dig', '+time=2', '+tries=1', '+short', 'A', host, '@1.1.1.1'],
                       text=True, timeout=5)
        addresses = []
        for value in answer.splitlines():
            try:
                ip = ipaddress.IPv4Address(value.strip())
            except ipaddress.AddressValueError:
                continue  # dig may include CNAME records
            if not ip.is_global:
                raise ValueError('Non-public Ubuntu DNS answer')
            if str(ip) not in addresses:
                addresses.append(str(ip))
        if not addresses:
            raise ValueError('No public IPv4 answer for ' + host)
        lines.extend(address + ' ' + host for address in addresses[:4])
    # No global resolver or hosts write, no persistent address pinning.
    with tempfile.TemporaryDirectory(prefix='ubuntu-apt-hosts-') as directory:
        path = pathlib.Path(directory) / 'hosts'
        path.write_text((root / 'etc/hosts').read_text() + '\n' + '\n'.join(lines) + '\n')
        path.chmod(0o644)
        yield path


def in_hosts_namespace(hosts, command):
    # Arguments go through argv. The shell program is fixed and the exec keeps
    # apt's exit status. Mount is private and vanishes when the child exits.
    return ['unshare', '--mount', '--propagation', 'private', '--', 'bash', '-euc',
            'mount --bind "$1" /etc/hosts; shift; exec "$@"', 'apt-hosts', str(hosts), *command]


def install(packages, root=pathlib.Path('/'), run=subprocess.run, hosts_factory=ubuntu_hosts):
    options = ['-o', 'Acquire::Retries=2', '-o', 'Acquire::http::Timeout=15',
               '-o', 'Acquire::https::Timeout=15']
    def attempt(hosts=None):
        commands = [
            (['apt-get', *options, '-o', 'APT::Update::Error-Mode=any', 'update'], 90),
            (['apt-get', *options, 'install', '-y', '--no-install-recommends', *packages], 180),
        ]
        for command, timeout in commands:
            run(in_hosts_namespace(hosts, command) if hosts else command,
                check=True, timeout=timeout)
    try:
        attempt()
        return
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
        print('apt failed; retrying official HTTPS mirrors with apt-only DNS answers', flush=True)
    use_official_mirrors(root)
    with hosts_factory(root) as hosts:
        attempt(hosts)


if __name__ == '__main__':
    if len(sys.argv) < 2:
        raise SystemExit('Pass the build dependency package names')
    install(sys.argv[1:])
