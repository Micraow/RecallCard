"""不启动程序或网络：运行包精确来源、可复现与恶意归档负向合同。"""
import hashlib
from copy import deepcopy
import importlib.util
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

ROOT = Path(__file__).resolve().parents[2]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


package = load('package_desktop')
verify = load('verify_runtime_package')


class RuntimePackageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for name in ['docs', 'fixtures', 'extension', 'desktop/src-tauri', 'desktop/third-party-licenses']:
            (self.root / name).mkdir(parents=True)
        (self.root/'docs/desktop-quickstart-v0.5.md').write_text('[说明](detail.md) [官方](https://example.org/doc) [本节](#标题)')
        (self.root/'docs/detail.md').write_text('合成说明')
        (self.root/'desktop/src-tauri/tauri.conf.json').write_text('{"version":"0.5.0"}')
        (self.root/'desktop/third-party-licenses/README.md').write_text('合成来源')
        for binary in ['recallcard-desktop', 'recallcard']:
            (self.root/binary).write_bytes(binary.encode())
        self.source = {'application_commit':'a'*40, 'profile':'dev',
                       'binaries':{name:verify.sha((self.root/name).read_bytes()) for name in ['recallcard-desktop','recallcard']},
                       'acceptance':{'commit':'b'*40,'run_id':42}}
        self.provenance=self.root/'source.json'; self.provenance.write_text(json.dumps(self.source))
        self.metadata=[]
        for index, version in enumerate(['1.0.0','2.0.0']):
            directory=self.root/('dependency'+str(index));directory.mkdir()
            (directory/'LICENSE-MIT').write_text('合成MIT许可'+version)
            pkg={'id':'same-name@'+version,'name':'same-name','version':version,'source':'registry+synthetic',
                 'manifest_path':str(directory/'Cargo.toml'),'license':'MIT'}
            meta=self.root/('metadata'+str(index)+'.json')
            meta.write_text(json.dumps({'resolve':{'nodes':[{'id':pkg['id']}]},'packages':[pkg]}))
            self.metadata.append(meta)

    def build(self, out='first', extra=()):
        arguments=['package_desktop.py','--binary',str(self.root/'recallcard-desktop'),'--cli',str(self.root/'recallcard'),
                   '--commit','a'*40,'--packaging-commit','c'*40,'--acceptance-commit','b'*40,'--acceptance-run-id','42',
                   '--gui-sha256',self.source['binaries']['recallcard-desktop'],'--cli-sha256',self.source['binaries']['recallcard'],
                   '--provenance',str(self.provenance),'--out',str(self.root/out)]
        for metadata in self.metadata:arguments+=['--metadata',str(metadata)]
        with patch.object(sys,'argv',arguments+list(extra)),patch.object(package,'ROOT',self.root),patch.object(package.subprocess,'check_output',return_value='GLIBC_2.35 GLIBC_2.17'):
            package.main()
        return next((self.root/out).glob('*.zip'))

    def test_exact_union_keeps_both_versions_and_is_reproducible(self):
        first=self.build();second=self.build('second')
        self.assertEqual(first.read_bytes(),second.read_bytes())
        files=verify.archive_files(first);info=verify.check_files(files,self.source)
        notices=json.loads(files['third-party-licenses/index.json'][0])
        self.assertEqual([entry['version'] for entry in notices],['1.0.0','2.0.0'])
        self.assertEqual(info['dependency_metadata']['graphs'],2)
        self.assertEqual(info['packaging_commit'],'c'*40)
        self.assertEqual(info['requires'][1],'GLIBC >= 2.35')
        self.assertIn(b'docs/detail.md',files['开始使用-中文.md'][0])
        self.assertIn(b'https://example.org/doc',files['开始使用-中文.md'][0])
        self.assertIn(b'](#',files['开始使用-中文.md'][0])
        with zipfile.ZipFile(first) as archive:
            self.assertTrue(all(entry.date_time==(2026,1,1,0,0,0) for entry in archive.infolist()))

    def test_reboot_includes_current_guide_public_worker_and_frontend_licenses(self):
        (self.root/'desktop/src-tauri/tauri.conf.json').write_text(json.dumps({'version':'0.6.0-dev'}))
        self.source['application_version']='0.6.0-dev'
        self.source['acceptance']['suite']='v0.6-first-use-native'
        self.provenance.write_text(json.dumps(self.source))
        (self.root/'docs/desktop-quickstart-v0.6.md').write_text('合成新版导入说明')
        (self.root/'desktop/package.json').write_text(json.dumps({'version':'0.6.0-dev'}))
        for name in ['react','react-dom','scheduler']:
            dependency=self.root/'desktop/node_modules'/name
            dependency.mkdir(parents=True)
            (dependency/'package.json').write_text(json.dumps({'version':'1.0.0'}))
            (dependency/'LICENSE').write_text('合成 MIT 许可证')
        worker=self.root/'python/recallcard_dream'
        worker.mkdir(parents=True)
        (worker/'runtime.py').write_text('# 合成公开代码')
        (worker/'.env').write_text('SYNTHETIC_DO_NOT_PACKAGE=true')
        archive=self.build()
        files=verify.archive_files(archive)
        self.assertEqual(files['开始使用-中文.md'][0].decode(),'合成新版导入说明')
        self.assertIn('python/recallcard_dream/runtime.py',files)
        self.assertNotIn('python/recallcard_dream/.env',files)
        for name in ['react','react-dom','scheduler']:
            self.assertIn(f'third-party-licenses/npm-{name}-1.0.0/LICENSE',files)

    def test_safe_extract_and_post_runtime_verification(self):
        archive=self.build();destination=self.root/'extracted'
        self.assertTrue(verify.unpack(archive,destination,self.source)['verified'])
        self.assertTrue(verify.unpack(archive,destination,self.source,True)['verified'])
        with self.assertRaisesRegex(ValueError,'全新目录'):verify.unpack(archive,destination,self.source)
        (destination/'recallcard').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError,'内容或权限'):verify.unpack(archive,destination,self.source,True)

    def test_extra_files_and_symlinks_rejected_after_runtime(self):
        archive=self.build();destination=self.root/'extracted';verify.unpack(archive,destination,self.source)
        extra=destination/'extra';extra.write_text('changed')
        with self.assertRaises(ValueError):verify.unpack(archive,destination,self.source,True)
        extra.unlink();extra.symlink_to(destination/'recallcard')
        with self.assertRaisesRegex(ValueError,'符号链接'):verify.unpack(archive,destination,self.source,True)

    def test_archive_path_traversal_and_aliases_rejected(self):
        for name in ['../outside','/absolute','a/../b','a//b','a/./b','a\\b','C:bad','']:
            with self.subTest(name=name),self.assertRaises(ValueError):verify.safe_name(name)

    def bad_archive(self, entries):
        archive=self.root/'bad.zip'
        with zipfile.ZipFile(archive,'w') as opened:
            for name,mode in entries:
                entry=zipfile.ZipInfo(name);entry.create_system=3;entry.external_attr=mode<<16
                opened.writestr(entry,b'synthetic')
        return archive

    def test_symlink_directory_special_and_privileged_modes_rejected(self):
        for mode in [stat.S_IFLNK|0o777,stat.S_IFDIR|0o755,stat.S_IFIFO|0o644,stat.S_IFREG|0o4755,stat.S_IFREG|0o666]:
            with self.subTest(mode=mode),self.assertRaises(ValueError):
                verify.archive_files(self.bad_archive([('entry',mode)]))

    def test_duplicate_zip_member_rejected(self):
        with self.assertRaises(ValueError):verify.archive_files(self.bad_archive([('entry',0o100644),('entry',0o100644)]))

    def test_manifest_rejects_missing_extra_duplicate_and_corrupted_files(self):
        archive=self.build();original=verify.archive_files(archive)
        cases=[]
        files=original.copy();del files['docs/detail.md'];cases.append(files)
        files=original.copy();files['extra']=(b'',0o644);cases.append(files)
        files=original.copy();files['docs/detail.md']=(b'corrupted',0o644);cases.append(files)
        files=original.copy();files['SHA256SUMS']=(files['SHA256SUMS'][0]*2,0o644);cases.append(files)
        for files in cases:
            with self.subTest(keys=len(files)),self.assertRaises(ValueError):verify.check_files(files,self.source)

    def test_binary_external_identity_cannot_be_replaced_with_self_consistent_manifest(self):
        archive=self.build();files=verify.archive_files(archive)
        old=verify.sha(files['recallcard'][0]);files['recallcard']=(b'changed',0o755)
        files['SHA256SUMS']=(files['SHA256SUMS'][0].replace(old.encode(),verify.sha(b'changed').encode()),0o644)
        with self.assertRaisesRegex(ValueError,'已验收'):verify.check_files(files,self.source)

    def test_package_rejects_wrong_binary_and_wrong_provenance(self):
        with self.assertRaises(SystemExit):self.build(extra=['--gui-sha256','0'*64])
        self.source['application_commit']='d'*40;self.provenance.write_text(json.dumps(self.source))
        with self.assertRaises(SystemExit):self.build()

    def test_metadata_missing_resolved_package_and_conflict_rejected(self):
        metadata=json.loads(self.metadata[0].read_text());metadata['packages']=[]
        self.metadata[0].write_text(json.dumps(metadata))
        with self.assertRaisesRegex(ValueError,'缺少'):package.resolved_packages(self.metadata)

    def test_missing_license_body_rejected(self):
        (self.root/'dependency0/LICENSE-MIT').unlink()
        with self.assertRaisesRegex(ValueError,'许可证'):self.build()

    def test_nested_license_bodies_and_duplicate_basenames_keep_relative_paths(self):
        base=self.root/'dependency0'
        for path,body in [('src/unicode/LICENSE-UNICODE','合成Unicode许可'),('protocols/COPYING','合成协议许可'),('nested/LICENSE-MIT','另一版权所有人')]:
            target=base/path;target.parent.mkdir(parents=True,exist_ok=True);target.write_text(body)
        files=verify.archive_files(self.build())
        notices=json.loads(files['third-party-licenses/index.json'][0])
        self.assertEqual(notices[0]['included_files'],['LICENSE-MIT','nested/LICENSE-MIT','protocols/COPYING','src/unicode/LICENSE-UNICODE'])
        self.assertEqual(files['third-party-licenses/same-name-1.0.0/nested/LICENSE-MIT'][0].decode(),'另一版权所有人')
        self.assertNotEqual(files['third-party-licenses/same-name-1.0.0/LICENSE-MIT'][0],files['third-party-licenses/same-name-1.0.0/nested/LICENSE-MIT'][0])

    @unittest.skipUnless(hasattr(os,'mkfifo'),'需要Unix FIFO')
    def test_post_runtime_rejects_nonregular_extra_nodes(self):
        archive=self.build();destination=self.root/'extracted';verify.unpack(archive,destination,self.source)
        os.mkfifo(destination/'unexpected-fifo')
        with self.assertRaisesRegex(ValueError,'特殊节点'):verify.unpack(archive,destination,self.source,True)

    def test_delivery_workflow_is_no_build_and_requires_success_before_upload(self):
        text=(ROOT/'.github/workflows/deliver-validated-runtime.yml').read_text()
        self.assertNotIn('cargo build',text.replace('Run cargo build --locked --manifest-path desktop/src-tauri/Cargo.toml','BUILD_STEP_NAME'))
        self.assertEqual(text.count('--filter-platform x86_64-unknown-linux-gnu'),2)
        self.assertIn('test ! -e "$repository/desktop/ui"',text)
        self.assertIn('--application "$PWD/运行桌面版.sh"',text)
        self.assertIn('--native-summary',text)
        upload=text.split('name: 仅成功后保存版本化运行包与校验清单',1)[1].split('name: 保存隔离验收',1)[0]
        self.assertNotIn('always()',upload)
        self.assertNotIn('validate/desktop-',text)

    def test_main_guide_local_links_resolve_and_backup_keeps_independent_state(self):
        guide=(ROOT/'docs/desktop-quickstart-v0.5.md').read_text()
        import re
        for target in re.findall(r'\]\(([^)]+)\)',guide):
            if not re.match(r'^[a-z]+:|^#|^/',target):self.assertTrue((ROOT/'docs'/target).is_file(),target)
        for token in ['RECALLCARD_STATE_DIR','XDG_STATE_HOME','recent-workspace.json','隐藏的 `.git`','不能只换回旧程序','不会自动更新已连接客户端']:
            self.assertIn(token,guide)

    def test_v06_native_gate_requires_complete_real_journey_and_exact_package_identity(self):
        destination=self.root/'isolated'
        result={'application':str(destination/'运行桌面版.sh'),'destination':str(destination),
                'application_version':'0.6.0-dev'}
        source={**self.source,'application_version':'0.6.0-dev',
                'acceptance':{**self.source['acceptance'],'suite':'v0.6-first-use-native'}}
        identity={'version':'0.6.0-dev','commit':'a'*40,'dirty':False}
        summary={'success':True,'suite':'v0.6-first-use-native',
                 'passed_steps':verify.FIRST_USE_STEPS.copy(),'application':result['application'],
                 'cli':str(destination/'recallcard'),'build':{'gui':identity.copy(),'cli':identity.copy()},
                 'synthetic_data_only':True,'invoke_mocked':False,'external_chatgpt_verified':False}
        checked=verify.check_native_summary(summary,result,source)
        self.assertTrue(checked['post_native_verified'])
        self.assertEqual(checked['native_suite'],'v0.6-first-use-native')
        cases=[]
        for key,value in [('success',False),('suite','legacy-native-37'),('application','/tmp/wrong-app'),
                          ('cli','/tmp/wrong-cli'),('invoke_mocked',True),('synthetic_data_only',False),
                          ('passed_steps',verify.FIRST_USE_STEPS[:-1]),
                          ('passed_steps',list(reversed(verify.FIRST_USE_STEPS)))]:
            invalid=deepcopy(summary);invalid[key]=value;cases.append(invalid)
        for component,key,value in [('gui','commit','c'*40),('cli','commit','c'*40),
                                    ('gui','dirty',True),('cli','version','0.5.0')]:
            invalid=deepcopy(summary);invalid['build'][component][key]=value;cases.append(invalid)
        for invalid in cases:
            with self.subTest(invalid=invalid),self.assertRaises(ValueError):
                verify.check_native_summary(invalid,result,source)

    def test_legacy_37_steps_cannot_substitute_for_reboot(self):
        result={'application':'/tmp/isolated/运行桌面版.sh','destination':'/tmp/isolated',
                'application_version':'0.5.0'}
        summary={'success':True,'synthetic_data_only':True,'application':result['application'],
                 'passed_steps':[f'合成历史步骤{i}' for i in range(37)]}
        self.assertEqual(verify.check_native_summary(summary,result,self.source)['native_steps'],37)
        with self.assertRaises(ValueError):
            verify.check_native_summary(summary,result,{**self.source,'application_version':'0.6.0-dev'})

    def test_actual_v06_package_never_falls_back_to_legacy_summary_or_provenance(self):
        (self.root/'desktop/src-tauri/tauri.conf.json').write_text(json.dumps({'version':'0.6.0-dev'}))
        (self.root/'docs/desktop-quickstart-v0.6.md').write_text('合成新版说明')
        with self.assertRaises(SystemExit):self.build()
        self.source['application_version']='0.6.0-dev'
        self.source['acceptance']['suite']='v0.6-first-use-native'
        self.provenance.write_text(json.dumps(self.source))
        archive=self.build();destination=self.root/'extracted-v06'
        result=verify.unpack(archive,destination,self.source)
        self.assertEqual(result['application_version'],'0.6.0-dev')
        legacy={'success':True,'synthetic_data_only':True,'application':result['application'],
                'passed_steps':[f'合成历史步骤{i}' for i in range(37)]}
        with self.assertRaises(ValueError):verify.check_native_summary(legacy,result,self.source)

    def test_self_consistent_v06_archive_with_old_provenance_is_rejected_on_unpack(self):
        files=verify.archive_files(self.build())
        info=json.loads(files['build-info.json'][0])
        info.update(application='RecallCard Desktop 0.6.0-dev',application_version='0.6.0-dev')
        files['build-info.json']=(json.dumps(info).encode(),0o644)
        files['SHA256SUMS']=(''.join(f'{verify.sha(data)}  {name}\n'
                                    for name,(data,_) in sorted(files.items())
                                    if name!='SHA256SUMS').encode(),0o644)
        archive=self.root/'v06-with-old-source.zip'
        with zipfile.ZipFile(archive,'w') as z:
            for name,(data,mode) in files.items():
                entry=zipfile.ZipInfo(name);entry.create_system=3;entry.external_attr=(stat.S_IFREG|mode)<<16
                z.writestr(entry,data)
        with self.assertRaisesRegex(ValueError,'v0.6包'):
            verify.unpack(archive,self.root/'rejected',self.source)
        self.assertFalse((self.root/'rejected').exists())

    def test_v06_package_contract_tracks_all_actual_driver_checkpoints(self):
        import ast
        tree=ast.parse((ROOT/'desktop/tests/first_use_native.py').read_text())
        exercise=next(node for node in ast.walk(tree) if isinstance(node,ast.FunctionDef) and node.name=='exercise')
        calls=sorted((node for node in ast.walk(exercise) if isinstance(node,ast.Call)
                      and isinstance(node.func,ast.Attribute) and node.func.attr=='checkpoint'),
                     key=lambda node:node.lineno)
        self.assertEqual([ast.literal_eval(call.args[0]) for call in calls],verify.FIRST_USE_STEPS)


if __name__=='__main__':unittest.main()
