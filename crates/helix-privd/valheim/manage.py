#!/usr/bin/env python3
"""Valheim runtime and transactional Thunderstore installs, inside the game sandbox."""
import hashlib
import filecmp
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
import uuid
import zipfile

ROOT = Path('/data')
LOADER = 'denikson-BepInExPack_Valheim'
MIN_LOADER = (5, 4, 2350)
MAX_ARCHIVE = 256 * 1024 * 1024
MAX_EXPANDED = 1024 * 1024 * 1024
ALLOWED_HOSTS = {'thunderstore.io', 'gcdn.thunderstore.io', 'gcdn.thunderstore.org'}
API = 'https://thunderstore.io/api/experimental/package/'


def identity_environment(root):
    import pwd
    # PlayFab resolves the numeric runtime UID through NSS, even with HOME set.
    # Missing passwd entries otherwise crash its native logger. No host account is created.
    try:
        pwd.getpwuid(os.getuid())
        return {}
    except KeyError:
        directory = Path(tempfile.mkdtemp(prefix='helix-nss-'))
        passwd = directory / 'passwd'
        group = directory / 'group'
        passwd.write_text(f'helix:x:{os.getuid()}:{os.getgid()}:Valheim:{root}:/usr/sbin/nologin\n')
        group.write_text(f'helix:x:{os.getgid()}:\n')
        return {'NSS_WRAPPER_PASSWD': str(passwd), 'NSS_WRAPPER_GROUP': str(group),
                'LD_PRELOAD': '/usr/lib/x86_64-linux-gnu/libnss_wrapper.so'}


def safe_url(url):
    parsed = urllib.parse.urlsplit(url)
    if (parsed.scheme != 'https' or parsed.hostname not in ALLOWED_HOSTS
            or parsed.username or parsed.password or parsed.port not in (None, 443)):
        raise ValueError('Thunderstore returned an unsupported download host')
    return url


class Redirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return super().redirect_request(req, fp, code, msg, headers, safe_url(newurl))


def fetch(url, limit=4 * 1024 * 1024):
    request = urllib.request.Request(safe_url(url), headers={'User-Agent': 'Helix/1.0 (Valheim server manager)'})
    with urllib.request.build_opener(Redirects()).open(request, timeout=30) as response:
        result = bytearray()
        deadline = time.monotonic() + 180
        while True:
            block = response.read(min(1024 * 1024, limit + 1 - len(result)))
            result.extend(block)
            if len(result) > limit or time.monotonic() > deadline:
                raise ValueError('Thunderstore download exceeded its size or time limit')
            if not block:
                return bytes(result)


def reference(value):
    value = value.strip()
    if value.startswith('https://'):
        parsed = urllib.parse.urlsplit(value)
        if parsed.hostname not in {'thunderstore.io', 'valheim.thunderstore.io', 'old.thunderstore.io'}:
            raise ValueError('Paste a Valheim Thunderstore package link or Author-Package-Version')
        parts = parsed.path.strip('/').split('/')
        if parts[:3] == ['c', 'valheim', 'p']:
            parts = parts[3:]
        elif parts[:1] == ['package']:
            parts = parts[1:]
        else:
            raise ValueError('Use a package page, not a download link')
        if len(parts) == 4 and parts[2] == 'v':
            parts = [parts[0], parts[1], parts[3]]
        value = '-'.join(parts)
    if not re.fullmatch(r'[A-Za-z0-9_]{1,128}-[A-Za-z0-9_]{1,128}(?:-\d{1,8}\.\d{1,8}\.\d{1,8})?', value):
        raise ValueError('Use Author-Package or Author-Package-1.2.3, or paste its Thunderstore link')
    parts = value.split('-')
    return '-'.join(parts[:2]), parts[2] if len(parts) == 3 else None


def semver(value):
    if not re.fullmatch(r'\d{1,8}\.\d{1,8}\.\d{1,8}', value):
        raise ValueError('Invalid Thunderstore version')
    return tuple(int(part) for part in value.split('.'))


def package(value):
    name, version = reference(value)
    base = API + name.replace('-', '/') + '/'
    metadata = json.loads(fetch(base))
    if not any(item.get('community') == 'valheim' for item in metadata.get('community_listings', [])):
        raise ValueError('This package is not listed for Valheim')
    release = metadata['latest'] if version is None else json.loads(fetch(base + version + '/'))
    if release.get('is_active') is not True or release.get('full_name') != name + '-' + release.get('version_number', ''):
        raise ValueError('This Thunderstore release is unavailable or does not match the package')
    semver(release['version_number'])
    deps = release.get('dependencies', [])
    if not isinstance(deps, list) or len(deps) > 64:
        raise ValueError('Unsupported dependency list')
    for dependency in deps:
        if reference(dependency)[1] is None:
            raise ValueError('A dependency does not pin a version')
    return {
        'package': name, 'version': release['version_number'],
        'description': str(release.get('description', ''))[:2000],
        'dependencies': deps, 'download_url': safe_url(release['download_url']),
        'url': 'https://thunderstore.io/c/valheim/p/' + name.replace('-', '/') + '/',
        'deprecated': bool(metadata.get('is_deprecated')),
        'enabled': True,
    }


def regular_tree(path):
    """Never follow game-created links when copying configuration or package payloads."""
    if path.is_symlink():
        raise ValueError('A managed mod directory is a symbolic link; inspect it in Files')
    if not path.exists():
        return
    for base, dirs, files in os.walk(path, followlinks=False):
        for name in dirs + files:
            item = Path(base) / name
            mode = item.lstat().st_mode
            if not (stat.S_ISREG(mode) or stat.S_ISDIR(mode)):
                raise ValueError('Managed mod files must be regular files and directories')


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, delete=False, encoding='utf-8') as stream:
        temporary = Path(stream.name)
        try:
            json.dump(value, stream, indent=2)
            stream.flush()
            os.fsync(stream.fileno())
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    os.replace(temporary, path)


def load_state(root):
    path = root / '.helix-valheim' / 'mods.json'
    if not path.exists():
        return {'generation': None, 'packages': []}
    if path.is_symlink() or path.stat().st_size > 2 * 1024 * 1024:
        raise ValueError('Invalid mod inventory; restore a backup before changing mods')
    state = json.loads(path.read_text())
    if state.get('generation') is not None and not re.fullmatch('[a-f0-9]{32}', state['generation']):
        raise ValueError('Invalid mod generation')
    if len(state['packages']) > 128:
        raise ValueError('Too many managed mods')
    for item in state['packages']:
        reference(item['package'] + '-' + item['version'])
    return state


def extract_package(archive, target, expected):
    total = 0
    names = set()
    with zipfile.ZipFile(archive) as bundle:
        if len(bundle.infolist()) > 10000:
            raise ValueError('Package contains too many files')
        manifest_entry = bundle.getinfo('manifest.json')
        if manifest_entry.file_size > 64 * 1024:
            raise ValueError('Package manifest is too large')
        manifest = json.loads(bundle.read('manifest.json'))
        if (manifest.get('name') != expected['package'].split('-')[1]
                or manifest.get('version_number') != expected['version']
                or sorted(manifest.get('dependencies', [])) != sorted(expected['dependencies'])):
            raise ValueError('Package manifest does not match Thunderstore metadata')
        for entry in bundle.infolist():
            name = entry.orig_filename
            parts = PurePosixPath(name).parts
            mode = entry.external_attr >> 16
            if (not parts or '\\' in name or ':' in name or name.startswith('/')
                    or any(p in ('..', '.') for p in name.rstrip('/').split('/'))
                    or len(name) > 1024
                    or any(ord(c) < 32 for c in name)
                    or stat.S_ISLNK(mode) or (stat.S_IFMT(mode) not in (0, stat.S_IFREG, stat.S_IFDIR))):
                raise ValueError('Package contains an unsafe file path')
            folded = name.rstrip('/').casefold()
            if folded in names:
                raise ValueError('Package contains duplicate file paths')
            names.add(folded)
            total += entry.file_size
            if total > MAX_EXPANDED or entry.file_size > MAX_ARCHIVE:
                raise ValueError('Expanded package is too large')
            destination = target.joinpath(*parts)
            if entry.is_dir():
                destination.mkdir(parents=True, exist_ok=True)
                continue
            destination.parent.mkdir(parents=True, exist_ok=True)
            with bundle.open(entry) as source, destination.open('xb') as output:
                shutil.copyfileobj(source, output, 1024 * 1024)
    return total


def resolve_packages(current, requested, lookup=package):
    selected = {item['package']: dict(item) for item in current}
    first = lookup(requested)
    selected[first['package']] = first
    pending = [first]
    # BepInEx is a runtime dependency, including for packages that forgot to declare it.
    loader = selected.get(LOADER)
    if loader is None or semver(loader['version']) < MIN_LOADER:
        loader = lookup(LOADER)
        if semver(loader['version']) < MIN_LOADER:
            raise ValueError('A supported Valheim BepInEx pack is not available')
        selected[LOADER] = loader
    loader['enabled'] = True
    pending.append(loader)
    visited = set()
    while pending:
        item = pending.pop()
        identity = (item['package'], item['version'])
        if identity in visited:
            continue
        visited.add(identity)
        if len(selected) > 128 or len(visited) > 256:
            raise ValueError('Dependency graph is too large')
        for dep in item['dependencies']:
            name, version = reference(dep)
            existing = selected.get(name)
            if existing is None or semver(existing['version']) < semver(version):
                if name == first['package']:
                    raise ValueError('Another enabled mod requires a newer version; update or remove that mod first')
                existing = lookup(dep)
                selected[name] = existing
            existing['enabled'] = True
            pending.append(existing)
    validate_dependencies(list(selected.values()))
    return sorted(selected.values(), key=lambda item: item['package'].lower())


def validate_dependencies(packages):
    enabled = {item['package']: item for item in packages if item['enabled']}
    if enabled and LOADER not in enabled:
        raise ValueError('Disable or remove the other mods before disabling BepInEx')
    for item in enabled.values():
        for dep in item['dependencies']:
            name, version = reference(dep)
            if name not in enabled or semver(enabled[name]['version']) < semver(version):
                raise ValueError(f"{item['package']} needs {dep}; change the dependent mod first")
    visiting, complete = set(), set()

    def visit(name):
        if name in visiting:
            raise ValueError('Package dependencies contain a cycle')
        if name in complete:
            return
        visiting.add(name)
        for dep in enabled[name]['dependencies']:
            visit(reference(dep)[0])
        visiting.remove(name)
        complete.add(name)

    for name in enabled:
        visit(name)


def copy_payload(source, destination):
    regular_tree(source)
    destination.parent.mkdir(parents=True, exist_ok=True)
    if source.is_dir():
        destination.mkdir(parents=True, exist_ok=True)
        for child in source.iterdir():
            copy_payload(child, destination / child.name)
    else:
        if destination.exists():
            if not destination.is_file() or not filecmp.cmp(source, destination, shallow=False):
                raise ValueError('Packages contain conflicting files; install them separately to review their defaults')
            return
        shutil.copy2(source, destination)


def assemble_package(source, generation, item):
    name = item['package']
    if name == LOADER:
        base = source / 'BepInExPack_Valheim'
        for relative in ['BepInEx/core', 'BepInEx/config', 'doorstop_libs']:
            if not (base / relative).is_dir():
                raise ValueError('Unsupported BepInEx pack layout')
            copy_payload(base / relative, generation / relative)
        if not (generation / 'doorstop_libs/libdoorstop_x64.so').is_file():
            raise ValueError('BepInEx pack is missing the Linux loader')
        return
    base = source / 'BepInEx' if (source / 'BepInEx').is_dir() else source
    structured = any((base / folder).is_dir() for folder in ['plugins', 'patchers', 'config'])
    if structured:
        for folder in ['plugins', 'patchers', 'config']:
            path = base / folder
            if path.is_dir():
                destination = generation / 'BepInEx' / folder
                if folder != 'config':
                    destination /= name
                copy_payload(path, destination)
    else:
        for path in source.iterdir():
            if path.name.lower() in {'manifest.json', 'icon.png', 'readme.md', 'changelog.md'}:
                continue
            copy_payload(path, generation / 'BepInEx/plugins' / name / path.name)


def publish_mods(root, packages, download=fetch):
    validate_dependencies(packages)
    previous_generation = load_state(root)['generation']
    managed = root / '.helix-valheim'
    for folder in [managed, managed / 'packages', managed / 'generations']:
        if folder.is_symlink():
            raise ValueError('Managed mod folders cannot be symbolic links')
    generation_id = uuid.uuid4().hex
    generation = managed / 'generations' / generation_id
    generation.mkdir(parents=True)
    total = 0
    try:
        # Loader first; dependencies remain separate to avoid overwriting another mod's files.
        for item in sorted(packages, key=lambda item: item['package'] != LOADER):
            cache = managed / 'packages' / (item['package'] + '-' + item['version'])
            if not cache.is_dir():
                with tempfile.TemporaryDirectory(dir=managed) as temporary:
                    stage = Path(temporary)
                    archive = stage / 'package.zip'
                    payload = download(item['download_url'], MAX_ARCHIVE)
                    total += len(payload)
                    if total > MAX_EXPANDED:
                        raise ValueError('This install exceeds the 1 GiB download budget')
                    archive.write_bytes(payload)
                    item['sha256'] = hashlib.sha256(payload).hexdigest()
                    unpacked = stage / 'unpacked'
                    unpacked.mkdir()
                    extract_package(archive, unpacked, item)
                    cache.parent.mkdir(parents=True, exist_ok=True)
                    os.replace(unpacked, cache)
            if item['enabled']:
                assemble_package(cache, generation, item)
        # Config lives outside immutable mod generations and is never overwritten on update.
        config = root / 'BepInEx/config'
        regular_tree(config)
        config.mkdir(parents=True, exist_ok=True)
        defaults = generation / 'BepInEx/config'
        if defaults.exists():
            for path in defaults.rglob('*'):
                if path.is_file():
                    destination = config / path.relative_to(defaults)
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    if not destination.exists():
                        shutil.copy2(path, destination)
            shutil.rmtree(defaults)
        state = {'generation': generation_id, 'packages': packages}
        atomic_json(managed / 'mods.json', state)
        # Full server backups hold rollback copies. Keep only one extra prepared generation.
        try:
            for old in (managed / 'generations').iterdir():
                if (re.fullmatch('[a-f0-9]{32}', old.name) and not old.is_symlink()
                        and old.is_dir() and old.name not in {generation_id, previous_generation}):
                    shutil.rmtree(old)
            keep = {item['package'] + '-' + item['version'] for item in packages}
            for old in (managed / 'packages').iterdir():
                if not old.is_symlink() and old.is_dir() and old.name not in keep:
                    reference(old.name)
                    shutil.rmtree(old)
        except (OSError, ValueError):
            print('Helix: unused mod cache could not be cleaned; the new generation is active', flush=True)
        return state
    except BaseException:
        shutil.rmtree(generation)
        raise


def steam_update(root, repair=False):
    steam = root / 'steamcmd'
    if not (steam / 'steamcmd.sh').is_file():
        shutil.copytree('/opt/steamcmd', steam, dirs_exist_ok=True)
    command = [str(steam / 'steamcmd.sh'), '+@sSteamCmdForcePlatformType', 'linux',
               '+force_install_dir', str(root / 'server'), '+login', 'anonymous', '+app_update', '896660']
    if repair:
        command.append('validate')
    deadline = time.monotonic() + 1200
    for attempt in range(3):
        result = subprocess.run(command + ['+quit'], timeout=max(1, deadline - time.monotonic()))
        if result.returncode == 0:
            break
        if result.returncode != 8 or attempt == 2:
            raise ValueError(f'SteamCMD could not install Valheim (exit {result.returncode}). Existing worlds and settings were kept; retry once Steam is reachable.')
        # SteamCMD can race its first app-info fetch and report "Missing configuration".
        print('Helix: Steam metadata was not ready; retrying the install', flush=True)
        time.sleep(2 * (attempt + 1))
    if not (root / 'server/valheim_server.x86_64').is_file():
        raise ValueError('SteamCMD did not install the Valheim executable')


def launch_arguments(settings, name, port):
    args = ['-nographics', '-batchmode', '-name', name, '-port', str(port),
            '-world', settings['world'], '-password', settings['password'],
            '-public', '1' if settings['public'] else '0', '-savedir', str(ROOT),
            '-saveinterval', str(settings['save_interval']), '-backups', str(settings['backups']),
            '-backupshort', str(settings['backup_short']), '-backuplong', str(settings['backup_long'])]
    if settings['crossplay']:
        args += ['-crossplay', '-instanceid', os.environ.get('HELIX_INSTANCE_ID', str(port))]
    if settings['preset']:
        args += ['-preset', settings['preset']]
    for key, value in settings['modifiers'].items():
        args += ['-modifier', key, value]
    for key in settings['keys']:
        args += ['-setkey', key]
    return args


def launch(root):
    ready = root / '.helix-ready'
    ready.unlink(missing_ok=True)
    if not (root / 'server/valheim_server.x86_64').is_file():
        print('Helix: installing Valheim through SteamCMD', flush=True)
        steam_update(root)
    settings = json.loads((root / 'valheim.json').read_text())
    state = load_state(root)
    environment = dict(os.environ, SteamAppId='892970', LD_LIBRARY_PATH=str(root / 'server/linux64'))
    environment.update(identity_environment(root))
    if any(item['enabled'] for item in state['packages']):
        generation = root / '.helix-valheim/generations' / state['generation']
        for folder, target in [('config', root / 'BepInEx/config'), ('plugins/manual', root / 'plugins')]:
            target.mkdir(parents=True, exist_ok=True)
            link = generation / 'BepInEx' / folder
            link.parent.mkdir(parents=True, exist_ok=True)
            if not link.exists() and not link.is_symlink():
                link.symlink_to(target, target_is_directory=True)
        environment.update(DOORSTOP_ENABLED='1', DOORSTOP_TARGET_ASSEMBLY=str(generation / 'BepInEx/core/BepInEx.Preloader.dll'),
                           LD_PRELOAD=':'.join(filter(None, [environment.get('LD_PRELOAD'), str(generation / 'doorstop_libs/libdoorstop_x64.so')])))
    log_root = root / 'logs'
    log_root.mkdir(exist_ok=True)
    log_path = log_root / 'valheim.log'
    if log_path.exists():
        os.replace(log_path, log_root / 'valheim.previous.log')
    command = [str(root / 'server/valheim_server.x86_64')] + launch_arguments(settings, os.environ['HELIX_SERVER_NAME'], os.environ['HELIX_GAME_PORT'])
    process = subprocess.Popen(command, cwd=root / 'server', env=environment, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, start_new_session=True)
    stopping = False

    def stop(_signal, _frame):
        nonlocal stopping
        if not stopping:
            stopping = True
            ready.unlink(missing_ok=True)
            try:
                os.killpg(process.pid, signal.SIGINT)
            except ProcessLookupError:
                pass

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    with log_path.open('wb') as log:
        for raw in iter(process.stdout.readline, b''):
            line = raw.decode('utf-8', errors='replace')
            # Docker's bounded logs feed Helix's persistent console archive.
            print(line.rstrip(), flush=True)
            log.write(raw)
            log.flush()
            if log.tell() > 20 * 1024 * 1024:
                log.seek(0)
                log.truncate()
            ready_message = 'registered with join code' if settings['crossplay'] else 'game server connected'
            if not stopping and ready_message in line.lower():
                ready.touch()
    ready.unlink(missing_ok=True)
    return process.wait()


def manage(root, request):
    action = request['action']
    state = load_state(root)
    if action == 'package':
        return package(request['reference'])
    if action == 'check_updates':
        updates = []
        for item in state['packages']:
            latest = package(item['package'])
            if semver(latest['version']) > semver(item['version']):
                updates.append(latest)
        return {'updates': updates}
    if action == 'update_game':
        steam_update(root, request['repair'])
        return {'updated': True}
    if action == 'install':
        packages = resolve_packages(state['packages'], request['reference'])
    else:
        name, _ = reference(request['package'])
        if not any(item['package'] == name for item in state['packages']):
            raise ValueError('This mod is not installed')
        packages = [dict(item) for item in state['packages']]
        if action == 'remove_mod':
            packages = [item for item in packages if item['package'] != name]
        elif action == 'set_mod_enabled':
            for item in packages:
                if item['package'] == name:
                    item['enabled'] = request['enabled']
        else:
            raise ValueError('Unsupported Valheim action')
    return publish_mods(root, packages)


if __name__ == '__main__':
    try:
        if len(sys.argv) == 1:
            sys.exit(launch(ROOT))
        result = manage(ROOT, json.load(sys.stdin) if sys.argv[1] == '-' else json.loads(sys.argv[1]))
        print('HELIX_RESULT=' + json.dumps(result), flush=True)
    except Exception as error:
        print('HELIX_ERROR=' + str(error), file=sys.stderr, flush=True)
        sys.exit(1)
