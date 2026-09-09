import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location('valheim_manage', Path(__file__).with_name('manage.py'))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


def mod(name='Author_Mod-Test', version='1.0.0', dependencies=None):
    return dict(package=name, version=version, dependencies=dependencies or [], enabled=True,
                description='', deprecated=False, download_url='https://thunderstore.io/package/download/A/B/1.0.0/')


def archive(item, files):
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, 'w') as bundle:
        bundle.writestr('manifest.json', json.dumps(dict(name=item['package'].split('-')[1], version_number=item['version'], dependencies=item['dependencies'])))
        for name, content in files.items():
            bundle.writestr(name, content)
    stream.seek(0)
    return stream


class ValheimTests(unittest.TestCase):
    def test_references(self):
        self.assertEqual(m.reference('https://thunderstore.io/c/valheim/p/A/B/v/1.2.3/'), ('A-B', '1.2.3'))
        self.assertEqual(m.reference('A-B'), ('A-B', None))
        for bad in ['../../x', 'A-B-1.2', 'https://evil.test/package/A/B/', 'A/B', 'A-B;id']:
            with self.assertRaises(ValueError): m.reference(bad)

    def test_download_hosts(self):
        self.assertEqual(m.safe_url('https://gcdn.thunderstore.io/file.zip'), 'https://gcdn.thunderstore.io/file.zip')
        for bad in ['http://thunderstore.io/a', 'https://127.0.0.1/a', 'https://thunderstore.io.evil.test/a', 'https://user:pass@thunderstore.io/a', 'https://thunderstore.io:444/a']:
            with self.assertRaises(ValueError): m.safe_url(bad)

    def test_archive_validation(self):
        item = mod()
        for name in ['../outside.dll', '/absolute.dll', 'C:/bad.dll', 'BepInEx/../../bad.dll']:
            with tempfile.TemporaryDirectory() as tmp, self.assertRaises(ValueError):
                m.extract_package(archive(item, {name: b'bad'}), Path(tmp), item)
        malformed = archive(item, {'a/b.dll': b'bad'}).getvalue().replace(b'a/b.dll', b'a\\b.dll')
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(ValueError):
            m.extract_package(io.BytesIO(malformed), Path(tmp), item)
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp)
            m.extract_package(archive(item, {'plugins/hello.dll': b'test'}), target, item)
            self.assertEqual((target / 'plugins/hello.dll').read_bytes(), b'test')

    def test_manifest_mismatch(self):
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(ValueError):
            m.extract_package(archive(mod(version='2.0.0'), {}), Path(tmp), mod())

    def test_dependency_resolution_and_downgrade(self):
        loader = mod(m.LOADER, '5.4.2350')
        dependency = mod('Author-Dependency')
        first = mod('Author-First', dependencies=['Author-Dependency-1.0.0'])
        lookup = lambda ref: {m.LOADER: loader, 'Author-First': first, 'Author-Dependency-1.0.0': dependency}[ref]
        result = m.resolve_packages([], 'Author-First', lookup)
        self.assertEqual(len(result), 3)
        dependency['enabled'] = False
        with self.assertRaises(ValueError): m.validate_dependencies([first, dependency, loader])
        dependency['enabled'] = True
        dependent = mod('Other-Dependent', dependencies=['Author-First-2.0.0'])
        with self.assertRaises(ValueError): m.resolve_packages([dependent, loader], 'Author-First', lookup)

    def test_transaction_preserves_configuration_and_failure_preserves_inventory(self):
        loader = mod(m.LOADER, '5.4.2350')
        loader_zip = archive(loader, {
            'BepInExPack_Valheim/BepInEx/core/BepInEx.Preloader.dll': b'loader',
            'BepInExPack_Valheim/doorstop_libs/libdoorstop_x64.so': b'linux',
            'BepInExPack_Valheim/BepInEx/config/BepInEx.cfg': b'default',
        }).getvalue()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            config = root / 'BepInEx/config'
            config.mkdir(parents=True)
            (config / 'BepInEx.cfg').write_text('valuable settings')
            (root / 'world.db').write_text('valuable world')
            first = m.publish_mods(root, [loader], lambda url, limit: loader_zip)
            self.assertEqual((config / 'BepInEx.cfg').read_text(), 'valuable settings')
            new = mod('Author-Broken')
            with self.assertRaises(zipfile.BadZipFile):
                m.publish_mods(root, [loader, new], lambda url, limit: b'not a zip')
            self.assertEqual(m.load_state(root), first)
            self.assertEqual((root / 'world.db').read_text(), 'valuable world')
            second = m.publish_mods(root, [loader], lambda url, limit: self.fail('Cache was not reused'))
            self.assertNotEqual(first['generation'], second['generation'])
            third = m.publish_mods(root, [loader], lambda url, limit: self.fail('Cache was not reused'))
            self.assertEqual({p.name for p in (root / '.helix-valheim/generations').iterdir()},
                             {second['generation'], third['generation']})

    def test_dependency_cycles_are_rejected(self):
        with self.assertRaisesRegex(ValueError, 'cycle'):
            m.validate_dependencies([mod(m.LOADER, '5.4.2350'),
                mod('Author-A', dependencies=['Author-B-1.0.0']),
                mod('Author-B', dependencies=['Author-A-1.0.0'])])

    def test_conflicting_payloads_cannot_overwrite_defaults(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source, destination = root / 'source', root / 'destination'
            source.write_text('new default')
            destination.write_text('other mod default')
            with self.assertRaisesRegex(ValueError, 'conflicting'):
                m.copy_payload(source, destination)
            self.assertEqual(destination.read_text(), 'other mod default')

    def test_launch_arguments_are_not_a_shell(self):
        settings = dict(world='World with spaces', password='literal;$(secret)', public=False, crossplay=True,
                        save_interval=900, backups=8, backup_short=3600, backup_long=7200,
                        preset='hard', modifiers={'resources': 'more'}, keys=['nomap'])
        args = m.launch_arguments(settings, 'Our server', 2456)
        self.assertIn('literal;$(secret)', args)
        self.assertIn('-crossplay', args)
        self.assertLess(args.index('-preset'), args.index('-modifier'))
        self.assertEqual(args[args.index('-world') + 1], 'World with spaces')


if __name__ == '__main__':
    unittest.main()
