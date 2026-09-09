"""Run the actual Linux launcher with a disposable save-on-SIGINT process."""
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

SERVER = r'''
#include <signal.h>
#include <stdio.h>
#include <unistd.h>
static volatile sig_atomic_t stopping = 0;
static void stop(int value) { (void)value; stopping = 1; }
int main(void) {
    signal(SIGINT, stop);
    signal(SIGTERM, SIG_IGN);
    puts("Game server connected"); fflush(stdout);
    while (!stopping) pause();
    FILE *saved = fopen("world-saved", "w");
    if (!saved) return 2;
    fputs("complete save", saved); fclose(saved);
    return 0;
}
'''


@unittest.skipUnless(sys.platform == 'linux' and shutil.which('cc'), 'Linux and a C compiler required')
class RuntimeTests(unittest.TestCase):
    def test_stop_saves_and_clears_readiness(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            server = root / 'server'
            server.mkdir()
            source = root / 'fixture.c'
            source.write_text(SERVER)
            subprocess.run(['cc', '-o', str(server / 'valheim_server.x86_64'), str(source)], check=True)
            (root / 'valheim.json').write_text(json.dumps(dict(world='Fixture', password='test-only',
                public=False, crossplay=False, save_interval=900, backups=4, backup_short=7200,
                backup_long=43200, preset='', modifiers={}, keys=[])))
            manager = str(Path(__file__).with_name('manage.py'))
            program = ('import importlib.util, pathlib, sys; '
                       's=importlib.util.spec_from_file_location("manager", sys.argv[1]); '
                       'm=importlib.util.module_from_spec(s); s.loader.exec_module(m); '
                       'm.ROOT=pathlib.Path(sys.argv[2]); sys.exit(m.launch(m.ROOT))')
            environment = dict(os.environ, HELIX_SERVER_NAME='Fixture', HELIX_GAME_PORT='2456')
            process = subprocess.Popen([sys.executable, '-c', program, manager, str(root)],
                                       env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 10
                while not (root / '.helix-ready').exists():
                    if process.poll() is not None or time.monotonic() > deadline:
                        self.fail('Launcher did not report real readiness')
                    time.sleep(0.05)
                process.send_signal(signal.SIGTERM)
                output = process.communicate(timeout=10)[0]
                self.assertEqual(process.returncode, 0, output.decode())
                self.assertEqual((server / 'world-saved').read_text(), 'complete save')
                self.assertFalse((root / '.helix-ready').exists())
                self.assertIn(b'Game server connected', (root / 'logs/valheim.log').read_bytes())
            finally:
                if process.poll() is None:
                    process.kill()
                process.communicate(timeout=5)


if __name__ == '__main__':
    unittest.main()
