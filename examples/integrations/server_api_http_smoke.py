import base64
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import time
import uuid

if os.environ.get('HELIX_DISPOSABLE_API_TEST') != '1':
    raise SystemExit('Run only in the disposable linux-test Docker target, with HELIX_DISPOSABLE_API_TEST=1')
sys.path.insert(0, str(Path(__file__).resolve().parent))
from helix_client import HelixClient, HelixError

root = Path('/dev/shm/server-api-probe')
general_storage = Path('/probe-general-storage')
general_storage.mkdir(mode=0o700)
root.mkdir(mode=0o755)
for name in ('state', 'instances', 'backups', 'socket'):
    (root / name).mkdir(mode=0o755)
broker_config = Path('/probe-broker.json')
broker_config.write_text(json.dumps({'socket': str(root / 'socket/privd.sock'), 'managed_roots':[str(general_storage)],
    'host_control': {'docker_binary':'/bin/true','systemctl_binary':'/bin/true','systemd_run_binary':'/bin/true','systemd_analyze_binary':'/bin/true','broker_binary':'/tmp/helix-privd','broker_config_path':'/probe-broker.json'},
    'network': {'docker_binary':'/bin/true'},
    'native': {'state_root':str(root / 'state'), 'instance_root':str(root / 'instances'),
               'backup_root':str(root / 'backups'), 'docker_binary':'/bin/true'}}))
broker_config.chmod(0o600)
data = Path('/probe-data')
data.mkdir(mode=0o700)
os.chown(data,10001,10001)
web = Path('/probe-web')
web.mkdir(mode=0o755)
(web / 'index.html').write_text('<!doctype html><title>Helix API test</title>')
config = Path('/probe.toml')
config.write_text('[server]\nlisten = "127.0.0.1:8080"\n[paths]\ndata_dir = "/probe-data"\nweb_root = "/probe-web"\n')
runner = ['setpriv','--reuid=10001','--regid=10001','--clear-groups']
setup = subprocess.run(runner + ['/tmp/helixctl','--config',str(config),'setup-token'],check=True,capture_output=True,text=True)
tokens = re.findall(r'^[A-Za-z0-9_-]{43}$', setup.stdout, flags=re.MULTILINE)
assert len(tokens)==1
password = secrets.token_hex(32)
processes = []

def expect_failure(callback, status=400):
    try:
        callback()
        raise AssertionError('expected rejection')
    except HelixError as e:
        assert e.status == status, (e.status,e.code)

with open('/probe-api.log','w') as log:
    try:
        broker = subprocess.Popen(['/tmp/helix-privd','--config',str(broker_config)],stdout=log,stderr=log)
        processes.append(broker)
        deadline = time.monotonic()+15
        while not (root / 'socket/privd.sock').exists():
            assert broker.poll() is None and time.monotonic()<deadline, 'broker failed to start'
            time.sleep(0.1)
        os.chown(root / 'socket/privd.sock',0,10001)
        ids = []
        # Real native registry and file broker; no game process or Docker socket in this sandbox.
        for game, software in [('minecraft','paper'),('minecraft','pumpkin'),('minecraft','custom'),
                               ('vrising','vanilla'),('valheim','vanilla'),('terraria','vanilla')]:
            identity = str(uuid.uuid4())
            ids.append('helix:'+identity)
            instance = root / 'instances' / identity
            instance.mkdir(mode=0o750)
            (instance / 'config.txt').write_text('old')
            manifest = {'schema_version':1,'kind':game,'id':identity,'name':'Fixture '+game,'instance_name':'fixture-'+identity,
                'container_name':'helix-game-'+identity,'software':software,'minecraft_version':'1.21.1','build':'fixture',
                'java_version':21,'runtime_image':'fixture','artifact_url':'https://example.invalid/server.jar',
                'artifact_sha256':'a'*64,'memory_mb':1024,'max_players':2,'game_port':25565,'rcon_port':30000,
                'rcon_password':secrets.token_hex(16),'start_on_boot':False,'run_uid':10001,'created_at_unix_ms':1}
            (root / 'state' / (identity+'.json')).write_text(json.dumps(manifest))
            backup_dir = root / 'backups' / identity
            backup_dir.mkdir()
            (backup_dir / '1787799939239.tar.gz').write_bytes(b'fixture archive')
        environment = {**os.environ,'HELIX_PRIVD_SOCKET':str(root / 'socket/privd.sock')}
        daemon = subprocess.Popen(runner + ['/tmp/helixd','--config',str(config)],env=environment,stdout=log,stderr=log)
        processes.append(daemon)
        client = HelixClient('http://127.0.0.1:8080',timeout=5)
        deadline = time.monotonic()+15
        while True:
            try:
                client.request('GET','/api/v1/setup/status')
                break
            except HelixError:
                assert daemon.poll() is None and time.monotonic()<deadline
                time.sleep(0.1)
        expect_failure(lambda:client.server_capabilities(ids[0]),401)
        client.request('POST','/api/v1/setup/owner',{'bootstrapToken':tokens[0],'loginName':'integration.test','displayName':'Integration Test','password':password})
        client.login('integration.test',password)
        assert client.request('GET','/api/v1/openapi.json') == json.loads(Path('/build/docs/openapi.json').read_text())
        local = Path('/probe-source.bin')
        local.write_bytes(bytes(range(256))*5000)  # Multi-chunk transfer, not a tiny single-write shortcut.
        for index, server_id in enumerate(ids):
            cap = client.server_capabilities(server_id)
            assert cap['files'] is True and cap['console_commands'] == (index < 3)
            entries = list(client.iter_server_directory(server_id))
            assert len(entries)==1
            old = client.server_files(server_id,'read',path='config.txt')
            new = client.server_files(server_id,'write',path='config.txt',content='new',expected_revision=old['revision'])
            expect_failure(lambda:client.server_files(server_id,'write',path='config.txt',content='wrong',expected_revision=old['revision']))
            assert client.server_files(server_id,'read',path=new['recovery_path'])['content']=='old'
            client.server_files(server_id,'mkdir',path='assets')
            client.upload_file(server_id,local,'assets/plugin.bin')
            output = Path('/probe-download-'+str(index))
            client.download_file(server_id,'assets/plugin.bin',output)
            assert output.read_bytes()==local.read_bytes()
            expect_failure(lambda:client.server_files(server_id,'read',path='../../etc/passwd'))
            backup_path = '/api/v1/servers/'+server_id+'/backups/1787799939239/download'
            stat = client.request('POST',backup_path,{'action':'stat'})
            chunk = client.request('POST',backup_path,{'action':'download','offset':0,'length':1024,'expected_revision':stat['revision']})
            assert base64.b64decode(chunk['data_base64'])==b'fixture archive'
        client.transfer_file(ids[0],'assets/plugin.bin',ids[1],'assets/copy.bin')
        assert client.server_files(ids[1],'stat',path='assets/copy.bin')['size']==local.stat().st_size
        started = client.server_files(ids[0],'upload_begin',path='unfinished',size=1,sha256=hashlib.sha256(b'x').hexdigest())
        expect_failure(lambda:client.server_files(ids[1],'upload_status',upload_id=started['upload_id']))
        client.server_files(ids[0],'upload_abort',upload_id=started['upload_id'])
        expect_failure(lambda:client.server_files('amp:unrelated','read',path='config.txt'))
        client.logout()
        expect_failure(lambda:client.server_capabilities(ids[0]),401)
        print('PASS: actual HTTP, sessions, broker, all six native game/software cases, multi-chunk uploads/downloads, cross-server transfer, revisions, recovery, backup export, path and upload-scope rejection.')
    except Exception:
        log.flush()
        print(Path('/probe-api.log').read_text()[-5000:], file=sys.stderr)
        raise
    finally:
        for process in reversed(processes):
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
