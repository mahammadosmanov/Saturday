import hashlib,io,os,struct,tarfile,tempfile,unittest
from pathlib import Path
from unittest import mock
from scripts import prebuilt_runtime_stage1 as stage1
from scripts.prebuilt_runtime_stage1 import PrivateWorkspace,extract_tar,extract_tar_layers,inspect_runtime,validate_tar_members

GENERATION='00112233445566778899aabbccddeeff'


def private_workspace(parent,required=1):
 flags=os.O_RDONLY|os.O_DIRECTORY
 if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
 descriptor=os.open(parent,flags)
 try: workspace=PrivateWorkspace.create(descriptor,GENERATION,required)
 finally: os.close(descriptor)
 workspace.path=Path(parent)/workspace.name
 return workspace

def member(name,size=0,kind=tarfile.REGTYPE,link=''):
 m=tarfile.TarInfo(name); m.size=size; m.type=kind; m.linkname=link; m.mode=0o755 if kind==tarfile.REGTYPE else 0o700; return m


def archive(entries):
 output=io.BytesIO()
 with tarfile.open(fileobj=output,mode='w',format=tarfile.USTAR_FORMAT) as bundle:
  for info,data in entries:
   bundle.addfile(info,io.BytesIO(data) if info.isreg() else None)
 output.seek(0); return output


def compressed_archive(entries):
 output=io.BytesIO()
 with tarfile.open(fileobj=output,mode='w:gz',format=tarfile.USTAR_FORMAT) as bundle:
  for info,data in entries:
   bundle.addfile(info,io.BytesIO(data) if info.isreg() else None)
 output.seek(0); return output


def macho_command(command,name,header_bytes):
 encoded=name.encode()+b'\0'; size=(header_bytes+len(encoded)+7)&~7
 if header_bytes==24: prefix=struct.pack('<IIIIII',command,size,24,0,0,0)
 else: prefix=struct.pack('<III',command,size,12)
 return prefix+encoded+b'\0'*(size-len(prefix)-len(encoded))


def thin_macho(filetype,commands=(),cputype=0x0100000c,signed=True,header_only_code_directory=False):
 commands=list(commands); signature=b''
 if signed:
  data_offset=32+sum(len(command) for command in commands)+16
  if header_only_code_directory:
   code_directory=struct.pack('>II',0xfade0c02,8)
  else:
   identifier=b'fixture\0'; hash_offset=44+len(identifier); code_limit=data_offset; n_code_slots=1
   code_directory_length=hash_offset+32
   code_directory_header=struct.pack('>9I4BI',0xfade0c02,code_directory_length,0x20001,0,hash_offset,44,0,n_code_slots,code_limit,32,2,0,12,0)
   signature_size=20+code_directory_length
   signature_command=struct.pack('<IIII',0x1d,16,data_offset,signature_size)
   payload=b''.join([*commands,signature_command])
   prefix=struct.pack('<IiiIIIII',0xfeedfacf,cputype,0,filetype,len(commands)+1,len(payload),0,0)+payload
   code_directory=code_directory_header+identifier+hashlib.sha256(prefix).digest()
  signature=struct.pack('>IIIII',0xfade0cc0,20+len(code_directory),1,0,20)+code_directory
  commands.append(struct.pack('<IIII',0x1d,16,data_offset,len(signature)))
 payload=b''.join(commands)
 return struct.pack('<IiiIIIII',0xfeedfacf,cputype,0,filetype,len(commands),len(payload),0,0)+payload+signature


def fat_macho(payload,cputype=0x0100000c):
 offset=32
 return struct.pack('>IIiiIII',0xcafebabe,1,cputype,0,offset,len(payload),2)+b'\0'*(offset-28)+payload

class ExtractionContractTests(unittest.TestCase):
 def test_accepts_regular_tree_and_relative_internal_symlink(self):
  symlink=member('pkg/bin/current',kind=tarfile.SYMTYPE,link='tool'); symlink.mode=0o777
  out=validate_tar_members([member('pkg/bin/tool',3),symlink],10,100)
  self.assertEqual([x['path'] for x in out],['pkg/bin/current','pkg/bin/tool'])

 def test_rejects_escape_alias_special_and_limits(self):
  bad=[[member('../x')],[member('pkg/x',kind=tarfile.SYMTYPE,link='../../x')],[member('pkg/x',kind=tarfile.FIFOTYPE)],[member('A'),member('a')],[member('x',101)]]
  for items in bad:
   with self.assertRaises(ValueError): validate_tar_members(items,10,100)

 def test_descriptor_relative_extraction_writes_links_and_cleans_root(self):
  payload=b'runtime-bytes'
  bundle=archive([
   (member('pkg/bin/tool',len(payload)),payload),
   (member('pkg/bin/current',kind=tarfile.SYMTYPE,link='tool'),b''),
   (member('pkg/bin/tool-copy',kind=tarfile.LNKTYPE,link='pkg/bin/tool'),b''),
  ])
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1)
   root=workspace.path
   roster=extract_tar(bundle,workspace.fd('runtime'),20,1024)
   self.assertEqual((root/'runtime/pkg/bin/tool').read_bytes(),payload)
   self.assertEqual(os.readlink(root/'runtime/pkg/bin/current'),'tool')
   self.assertEqual((root/'runtime/pkg/bin/tool').stat().st_ino,(root/'runtime/pkg/bin/tool-copy').stat().st_ino)
   self.assertEqual([item['path'] for item in roster],['pkg/bin/current','pkg/bin/tool','pkg/bin/tool-copy'])
   cleanup=workspace.cleanup()
   self.assertTrue(cleanup['root_removed'])
   self.assertEqual(cleanup['residue_entries'],0)
   self.assertFalse(root.exists())

 def test_multiple_layers_share_one_transaction_and_aggregate_limits(self):
  layers=[archive([(member('one/1.0/a',2),b'aa')]),archive([(member('two/2.0/b',3),b'bbb')])]
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1)
   roster=extract_tar_layers(layers,workspace.fd('runtime'),10,5)
   self.assertEqual([item['path'] for item in roster],['one/1.0/a','two/2.0/b'])
   self.assertEqual((workspace.path/'runtime/two/2.0/b').read_bytes(),b'bbb')
   workspace.cleanup()
  layers=[archive([(member('one/a',3),b'aaa')]),archive([(member('two/b',3),b'bbb')])]
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1)
   with self.assertRaises(ValueError): extract_tar_layers(layers,workspace.fd('runtime'),10,5)
   self.assertEqual(os.listdir(workspace.fd('runtime')),[])
   workspace.cleanup()

 def test_per_layer_extraction_caps_and_installed_sizes_are_enforced(self):
  cases=(
   ({'formula':'one','installed_bytes':2},'installed-size'),
   ({'formula':'one','maximum_extracted_bytes':2},'extraction cap'),
  )
  for limit,message in cases:
   with self.subTest(limit=limit),tempfile.TemporaryDirectory() as td:
    workspace=private_workspace(Path(td),1); payload=archive([(member('one/1.0/LICENSE',3),b'abc')])
    with self.assertRaisesRegex(ValueError,message):
     extract_tar_layers([payload],workspace.fd('runtime'),10,100,layer_limits=[limit])
    self.assertEqual(os.listdir(workspace.fd('runtime')),[])
    workspace.cleanup()

 def test_hard_link_chain_resolves_to_regular_file(self):
  payload=b'x'; bundle=archive([
   (member('a',kind=tarfile.LNKTYPE,link='b'),b''),
   (member('b',kind=tarfile.LNKTYPE,link='file'),b''),
   (member('file',1),payload),
  ])
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1)
   extract_tar(bundle,workspace.fd('runtime'),10,10)
   root=workspace.path/'runtime'
   self.assertEqual({(root/name).stat().st_ino for name in ('a','b','file')}.__len__(),1)
   workspace.cleanup()

 def test_extraction_completes_short_descriptor_writes(self):
  payload=b'0123456789'; bundle=archive([(member('file',len(payload)),payload)])
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1); original=os.write
   def short_write(descriptor,data): return original(descriptor,data[:max(1,len(data)//2)])
   with mock.patch('scripts.prebuilt_runtime_stage1.os.write',side_effect=short_write):
    extract_tar(bundle,workspace.fd('runtime'),10,100)
   self.assertEqual((workspace.path/'runtime/file').read_bytes(),payload)
   workspace.cleanup()

 def test_dirty_destination_and_cleanup_symlink_do_not_touch_external_data(self):
  with tempfile.TemporaryDirectory() as td:
   base=Path(td); outside=base/'outside'; outside.mkdir(); protected=outside/'keep'; protected.write_text('keep')
   workspace=private_workspace(base,1); runtime=workspace.path/'runtime'; sentinel=runtime/'sentinel'; sentinel.write_text('owned')
   with self.assertRaises(ValueError): extract_tar(archive([(member('new',1),b'x')]),workspace.fd('runtime'),10,10)
   self.assertEqual(sentinel.read_text(),'owned')
   (runtime/'escape').symlink_to(protected)
   cleanup=workspace.cleanup()
   self.assertTrue(cleanup['root_removed']); self.assertEqual(protected.read_text(),'keep')

 def test_cleanup_deletes_bytes_through_retained_role_descriptor_after_directory_swap(self):
  with tempfile.TemporaryDirectory() as td:
   base=Path(td); escaped=base/'escaped-runtime'
   workspace=private_workspace(base,1); runtime=workspace.path/'runtime'
   downloaded=runtime/'layer.tar'; downloaded.write_bytes(b'owned download')
   runtime.rename(escaped); runtime.mkdir()
   with self.assertRaisesRegex(OSError,'role identity changed'):
    workspace.cleanup()
   self.assertFalse(downloaded.exists())
   self.assertEqual(list(escaped.iterdir()),[])
   self.assertFalse(workspace.path.exists())

 def test_interrupted_extraction_rolls_back_every_created_entry(self):
  bundle=archive([(member('one',3),b'one'),(member('two',3),b'two')])
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1); calls=0; original=os.write
   def interrupt(descriptor,data):
    nonlocal calls
    calls+=1
    if calls>1: raise KeyboardInterrupt()
    return original(descriptor,data)
   with mock.patch('scripts.prebuilt_runtime_stage1.os.write',side_effect=interrupt):
    with self.assertRaises(KeyboardInterrupt): extract_tar(bundle,workspace.fd('runtime'),10,100)
   self.assertEqual(os.listdir(workspace.fd('runtime')),[])
   workspace.cleanup()

 def test_interrupted_workspace_preflight_removes_private_root(self):
  with tempfile.TemporaryDirectory() as td:
   with mock.patch('scripts.prebuilt_runtime_stage1.live_disk_preflight',side_effect=KeyboardInterrupt()):
    with self.assertRaises(KeyboardInterrupt): private_workspace(Path(td),1)
   self.assertEqual(list(Path(td).iterdir()),[])

 def test_rejects_sparse_pax_modes_unicode_and_link_graphs(self):
  sparse=member('sparse',1); sparse.sparse=b''
  pax=member('pax'); pax.pax_headers={'SCHILY.xattr.user.test':'x'}
  privileged=member('privileged'); privileged.mode=0o4755
  cases=[
   [sparse],
   [pax],
   [privileged],
   [member('caf\N{LATIN SMALL LETTER E WITH ACUTE}'),member('cafe\N{COMBINING ACUTE ACCENT}')],
   [member('A/one'),member('a/two')],
   [member('a',kind=tarfile.LNKTYPE,link='b'),member('b',kind=tarfile.LNKTYPE,link='a')],
   [member('link',kind=tarfile.SYMTYPE,link='target'),member('link/child'),member('target')],
  ]
  for items in cases:
   with self.subTest(items=[item.name for item in items]):
    with self.assertRaises(ValueError): validate_tar_members(items,20,100)

 def test_archive_expansion_limit_is_rejected_before_compressed_payload_is_enumerated(self):
  payload=os.urandom(2*1024*1024); raw=compressed_archive([(member('oversize',len(payload)),payload)]).getvalue()
  class CountingReader(io.BytesIO):
   def __init__(self,value): super().__init__(value); self.bytes_read=0
   def read(self,amount=None):
    value=super().read(amount); self.bytes_read+=len(value); return value
  reader=CountingReader(raw)
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1)
   with self.assertRaisesRegex(ValueError,'archive expansion limit'):
    extract_tar(reader,workspace.fd('runtime'),10,1)
   self.assertLess(reader.bytes_read,len(raw)//4)
   workspace.cleanup()

 def test_macho_dependency_and_license_inventory_is_data_only(self):
  commands=(
   macho_command(0xc,'/opt/homebrew/opt/ggml/lib/libggml.dylib',24),
   macho_command(0xc,'/usr/lib/libSystem.B.dylib',24),
   macho_command(0x8000001c,'@loader_path/../lib',12),
  )
  packages={
   'llama.cpp':{'version':'0.4.1','prefix':'llama.cpp/0.4.1','license':'MIT'},
   'ggml':{'version':'0.24.0','prefix':'ggml/0.24.0','license':'MIT'},
  }
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1)
   root=workspace.path/'runtime'
   for relative,data in (
    ('llama.cpp/0.4.1/bin/llama-cli',thin_macho(2,commands)),
    ('llama.cpp/0.4.1/LICENSE',b'llama license'),
    ('ggml/0.24.0/lib/libggml.dylib',fat_macho(thin_macho(6,(macho_command(0xd,'/opt/homebrew/opt/ggml/lib/libggml.dylib',24),)))),
    ('ggml/0.24.0/LICENSE.txt',b'ggml license'),
   ):
    path=root/relative; path.parent.mkdir(parents=True,exist_ok=True); path.write_bytes(data); path.chmod(0o755 if '/bin/' in relative else 0o644)
   inventory=inspect_runtime(workspace.fd('runtime'),packages)
   self.assertEqual([item['formula'] for item in inventory['licenses']],['ggml','llama.cpp'])
   self.assertTrue(all(item['declared_spdx']=='MIT' and 'spdx' not in item for item in inventory['licenses']))
   executable=next(item for item in inventory['mach_o'] if item['path'].endswith('llama-cli'))
   self.assertEqual(executable['slices'][0]['cputype'],0x0100000c)
   self.assertEqual(executable['loads'][0]['resolved'],'ggml/0.24.0/lib/libggml.dylib')
   self.assertEqual(executable['loads'][1]['resolved'],'system')
   self.assertTrue(executable['slices'][0]['code_signature']['present'])
   library=next(item for item in inventory['mach_o'] if item['path'].endswith('libggml.dylib'))
   self.assertEqual(library['ids'],['/opt/homebrew/opt/ggml/lib/libggml.dylib'])
   workspace.cleanup()

 def test_inventory_rejects_unsigned_macho_mismatched_dylib_id_and_non_macho_load_target(self):
  packages={
   'llama.cpp':{'version':'0.4.1','prefix':'llama.cpp/0.4.1','license':'MIT'},
   'ggml':{'version':'0.24.0','prefix':'ggml/0.24.0','license':'MIT'},
  }
  cases=(
   ('unsigned',thin_macho(2,signed=False),b'plain',None),
   ('bad-id',thin_macho(2,(macho_command(0xc,'/opt/homebrew/opt/ggml/lib/libggml.dylib',24),)),thin_macho(6,(macho_command(0xd,'/opt/homebrew/opt/ggml/lib/wrong.dylib',24),)),None),
   ('plain-load',thin_macho(2,(macho_command(0xc,'/opt/homebrew/opt/ggml/lib/plain.data',24),)),b'plain','plain.data'),
  )
  for label,executable,dependency,plain_name in cases:
   with self.subTest(label=label),tempfile.TemporaryDirectory() as td:
    workspace=private_workspace(Path(td),1); root=workspace.path/'runtime'
    values=[
     ('llama.cpp/0.4.1/bin/llama-cli',executable),('llama.cpp/0.4.1/LICENSE',b'license'),
     ('ggml/0.24.0/LICENSE',b'license'),
    ]
    if label=='bad-id': values.append(('ggml/0.24.0/lib/libggml.dylib',dependency))
    elif plain_name: values.append(('ggml/0.24.0/lib/'+plain_name,dependency))
    for relative,data in values:
     path=root/relative; path.parent.mkdir(parents=True,exist_ok=True); path.write_bytes(data); path.chmod(0o755 if '/bin/' in relative else 0o644)
    with self.assertRaises(ValueError): inspect_runtime(workspace.fd('runtime'),packages)
    workspace.cleanup()

 def test_inventory_rejects_files_outside_declared_package_prefixes(self):
  packages={'llama.cpp':{'version':'0.4.1','prefix':'llama.cpp/0.4.1','license':'MIT'}}
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1); root=workspace.path/'runtime'
   (root/'llama.cpp/0.4.1').mkdir(parents=True)
   (root/'llama.cpp/0.4.1/LICENSE').write_text('license')
   (root/'rogue').write_text('unexpected')
   with self.assertRaises(ValueError): inspect_runtime(workspace.fd('runtime'),packages)
   workspace.cleanup()

 def test_inventory_accepts_internal_directory_symlink_leaf(self):
  packages={'llama.cpp':{'version':'0.4.1','prefix':'llama.cpp/0.4.1','license':'MIT'}}
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1); root=workspace.path/'runtime/llama.cpp/0.4.1'
   (root/'share').mkdir(parents=True); (root/'LICENSE').write_text('license'); (root/'share-link').symlink_to('share')
   inventory=inspect_runtime(workspace.fd('runtime'),packages)
   self.assertIn('llama.cpp/0.4.1/share-link',[item['path'] for item in inventory['roster']])
   workspace.cleanup()

 def test_inventory_records_hard_link_target_without_double_counting(self):
  packages={'llama.cpp':{'version':'0.4.1','prefix':'llama.cpp/0.4.1','license':'MIT'}}
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1); root=workspace.path/'runtime/llama.cpp/0.4.1'; root.mkdir(parents=True)
   license_path=root/'LICENSE'; license_path.write_text('license'); os.link(license_path,root/'NOTICE')
   inventory=inspect_runtime(workspace.fd('runtime'),packages)
   notice=next(item for item in inventory['roster'] if item['path'].endswith('/NOTICE'))
   self.assertEqual((notice['kind'],notice['link_target'],notice['bytes']),('hardlink','llama.cpp/0.4.1/LICENSE',0))
   workspace.cleanup()

 def test_inventory_reads_each_hardlinked_inode_once(self):
  packages={'llama.cpp':{'version':'0.4.1','prefix':'llama.cpp/0.4.1','license':'MIT'}}
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1); root=workspace.path/'runtime/llama.cpp/0.4.1'; (root/'share').mkdir(parents=True)
   license_path=root/'LICENSE'; license_path.write_bytes(b'x'*(512*1024))
   for index in range(64): os.link(license_path,root/'share'/f'hardlink-{index:02d}')
   with mock.patch.object(stage1,'_read_regular_at',wraps=stage1._read_regular_at) as reader:
    inventory=inspect_runtime(workspace.fd('runtime'),packages)
   self.assertEqual(reader.call_count,1)
   self.assertEqual(sum(item['kind']=='hardlink' for item in inventory['roster']),64)
   workspace.cleanup()

 def test_inventory_rejects_unrelocatable_rpath(self):
  packages={'llama.cpp':{'version':'0.4.1','prefix':'llama.cpp/0.4.1','license':'MIT'}}
  binary=thin_macho(2,(macho_command(0x8000001c,'/tmp/escape',12),))
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1); root=workspace.path/'runtime/llama.cpp/0.4.1'
   (root/'bin').mkdir(parents=True); (root/'LICENSE').write_text('license'); (root/'bin/llama-cli').write_bytes(binary)
   with self.assertRaises(ValueError): inspect_runtime(workspace.fd('runtime'),packages)
   workspace.cleanup()

 def test_inventory_rejects_noncanonical_system_load_path(self):
  packages={'llama.cpp':{'version':'0.4.1','prefix':'llama.cpp/0.4.1','license':'MIT'}}
  binary=thin_macho(2,(macho_command(0xc,'/usr/lib/../../tmp/evil.dylib',24),))
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1); root=workspace.path/'runtime/llama.cpp/0.4.1'
   (root/'bin').mkdir(parents=True); (root/'LICENSE').write_text('license'); (root/'bin/llama-cli').write_bytes(binary)
   with self.assertRaisesRegex(ValueError,'system path'):
    inspect_runtime(workspace.fd('runtime'),packages)
   workspace.cleanup()

 def test_inventory_rejects_header_only_code_directory(self):
  packages={'llama.cpp':{'version':'0.4.1','prefix':'llama.cpp/0.4.1','license':'MIT'}}
  binary=thin_macho(2,header_only_code_directory=True)
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1); root=workspace.path/'runtime/llama.cpp/0.4.1'
   (root/'bin').mkdir(parents=True); (root/'LICENSE').write_text('license'); (root/'bin/llama-cli').write_bytes(binary)
   with self.assertRaisesRegex(ValueError,'CodeDirectory'):
    inspect_runtime(workspace.fd('runtime'),packages)
   workspace.cleanup()

 def test_inventory_rejects_malformed_or_foreign_macho(self):
  packages={'llama.cpp':{'version':'0.4.1','prefix':'llama.cpp/0.4.1','license':'MIT'}}
  for binary in (b'\xcf\xfa\xed\xfe',fat_macho(thin_macho(2,cputype=7),cputype=7)):
   with self.subTest(size=len(binary)),tempfile.TemporaryDirectory() as td:
    workspace=private_workspace(Path(td),1); root=workspace.path/'runtime/llama.cpp/0.4.1'
    (root/'bin').mkdir(parents=True); (root/'LICENSE').write_text('license'); (root/'bin/llama-cli').write_bytes(binary)
    with self.assertRaises(ValueError): inspect_runtime(workspace.fd('runtime'),packages)
    workspace.cleanup()
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1); root=workspace.path/'runtime/llama.cpp/0.4.1'
   (root/'lib').mkdir(parents=True); (root/'LICENSE').write_text('license'); (root/'lib/bad.dylib').write_bytes(b'not Mach-O')
   with self.assertRaises(ValueError): inspect_runtime(workspace.fd('runtime'),packages)
   workspace.cleanup()

 def test_inventory_rejects_missing_dependency_and_license_closure(self):
  packages={'llama.cpp':{'version':'0.4.1','prefix':'llama.cpp/0.4.1','license':'MIT'}}
  binary=thin_macho(2,(macho_command(0x20,'/opt/homebrew/opt/ggml/lib/libggml.dylib',24),))
  with tempfile.TemporaryDirectory() as td:
   workspace=private_workspace(Path(td),1); root=workspace.path/'runtime/llama.cpp/0.4.1'
   (root/'bin').mkdir(parents=True); (root/'bin/llama-cli').write_bytes(binary)
   with self.assertRaises(ValueError): inspect_runtime(workspace.fd('runtime'),packages)
   (root/'LICENSE').write_text('license')
   with self.assertRaises(ValueError): inspect_runtime(workspace.fd('runtime'),packages)
   workspace.cleanup()

if __name__=='__main__': unittest.main()
