"""Exercise the shipped Unix launchers with disposable save-on-exit processes."""
import os
import json
import pathlib
import signal
import subprocess
import tempfile
import time
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
SERVER = r'''
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
static volatile sig_atomic_t stopping = 0;
static void shutdown_signal(int value) { (void)value; stopping = 1; }
int main(int argc, char **argv) {
    (void)argc; (void)argv;
    signal(SIGINT, shutdown_signal);
    signal(SIGTERM, SIG_IGN);
    FILE *ready = fopen("test-ready", "w"); fclose(ready);
    puts("Game server connected"); fflush(stdout);
#ifdef CONSOLE
    char command[128];
    while (fgets(command, sizeof(command), stdin)) {
        if (strcmp(command, "exit\n") == 0) { stopping = 1; break; }
    }
#else
    while (!stopping) pause();
#endif
    if (!stopping) return 2;
    FILE *saved = fopen("test-saved", "w"); fputs("saved", saved); fclose(saved);
    return 0;
}
'''

def wait_for(path, process, seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if path.exists():
            return
        if process.poll() is not None:
            raise AssertionError('Launcher exited before readiness')
        time.sleep(0.1)
    raise AssertionError('Readiness timed out')

class LauncherShutdown(unittest.TestCase):
    def exercise(self, game, wait_until_ready):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            data = root / 'data'
            server = data / 'server'
            server.mkdir(parents=True)
            steam = data / 'steamcmd'
            steam.mkdir()
            (steam / 'steamcmd.sh').write_text('#!/bin/sh\nexit 0\n')
            (steam / 'steamcmd.sh').chmod(0o755)
            source = root / 'server.c'
            source.write_text(SERVER)
            if game == 'valheim':
                binary = server / 'valheim_server.x86_64'
                flags = []
            else:
                binary = server / '1449' / 'Linux' / 'TerrariaServer.bin.x86_64'
                binary.parent.mkdir(parents=True)
                (server / 'terraria-server-1449.zip').touch()
                flags = ['-DCONSOLE']
            subprocess.run(['cc', '-Wall', '-Wextra', '-Werror', *flags, str(source), '-o', str(binary)], check=True)
            script = root / 'entrypoint.sh'
            script.write_text((ROOT / game / 'entrypoint.sh').read_text().replace('/data', str(data)))
            if game == 'valheim':
                manager = root / 'manage.py'
                manager.write_text((ROOT / game / 'manage.py').read_text().replace("ROOT = Path('/data')", f'ROOT = Path({str(data)!r})'))
                script.write_text(script.read_text().replace('/usr/local/lib/helix-valheim.py', str(manager)))
                (data / 'valheim.json').write_text(json.dumps(dict(world='Test', password='test-only', public=False, crossplay=False, save_interval=60, backups=4, backup_short=300, backup_long=600, preset='', modifiers={}, keys=[])))
            with (root / 'launcher.log').open('w') as log:
                process = subprocess.Popen(['/bin/sh', str(script)], stdout=log, stderr=log, start_new_session=True, env=dict(os.environ, HELIX_SERVER_NAME='Test', HELIX_GAME_PORT='2456'))
                try:
                    wait_for(binary.parent / 'test-ready', process, 10)
                    if wait_until_ready:
                        wait_for(data / '.helix-ready', process, 50)
                    process.send_signal(signal.SIGTERM)
                    self.assertEqual(process.wait(timeout=10), 0, (root / 'launcher.log').read_text())
                    self.assertEqual((binary.parent / 'test-saved').read_text(), 'saved')
                    self.assertFalse((data / '.helix-ready').exists())
                finally:
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait()

    def test_valheim_shutdown_during_startup(self): self.exercise('valheim', False)
    def test_terraria_shutdown_during_startup(self): self.exercise('terraria', False)
    def test_valheim_shutdown_after_ready(self): self.exercise('valheim', True)
    def test_terraria_shutdown_after_ready(self): self.exercise('terraria', True)

if __name__ == '__main__':
    unittest.main()
