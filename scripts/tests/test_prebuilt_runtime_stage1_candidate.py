import copy
import hashlib
import json
import os
import shutil
import stat
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from scripts import prebuilt_runtime_stage1 as stage1
from scripts.prebuilt_runtime_stage1 import (
 CANDIDATE_SHA256,
 build_stage1_receipt,
 canonical_json_bytes,
 canonical_json_sha256,
 create_stage1_workspace,
 load_stage1_receipt,
 live_disk_preflight,
 load_candidate,
 receipt_layer_records,
 redacted_stage1_failure,
 validate_stage1_receipt,
 verify_review_manifest,
)
from scripts.tests.stage1_owner_test_support import GENERATION,verified_authorization

ROOT=Path(__file__).resolve().parents[2]
BASE=ROOT/'docs/spikes/005-prebuilt-runtime-candidate'
AGG='c034041a5ea9b5203e5e8385b010079764ff3daf13be00d5163e934cc11d447c'


def write_live_harness_manifest(directory,root=ROOT):
 paths=[item['path'] for item in json.loads((BASE/'harness-review-manifest.json').read_text())['files']]
 records=[]
 for name in paths:
  path=Path(root)/name; info=os.stat(path,follow_symlinks=False)
  records.append({'path':name,'bytes':info.st_size,'mode':stat.S_IMODE(info.st_mode),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()})
 aggregate=canonical_json_sha256(sorted(records,key=lambda item:item['path']))
 document={'schema':'jarvis.local-provider.review-manifest.v1','review_aggregate_sha256':aggregate,'files':records}
 manifest=Path(directory)/'harness-review-manifest.json'; manifest.write_text(json.dumps(document,sort_keys=True,separators=(',',':'))+'\n')
 digest=hashlib.sha256(manifest.read_bytes()).hexdigest()
 sidecar=Path(directory)/'harness-review-manifest.json.sha256'; sidecar.write_text(f'{digest}  {manifest.name}\n')
 return manifest,sidecar,aggregate,digest


class CandidateReplayTests(unittest.TestCase):
 def test_exact_candidate_replays_and_rejected_harness_does_not(self):
  candidate=load_candidate(BASE/'candidate-metadata.json')
  self.assertEqual(candidate['authorizations']['bottle_transfer']['state'],'not_granted')
  self.assertEqual(candidate['authorizations']['model_transfer']['state'],'blocked_until_relocated_runtime_review')
  verify_review_manifest(BASE/'review-manifest.json',ROOT,AGG)
  harness=json.loads((BASE/'harness-review-manifest.json').read_text())
  self.assertEqual(harness['review_aggregate_sha256'],'1c49497d67bc21f3a1a3edf49bca82be180ca517fce3380ead4b99389c9e4bc0')
  with self.assertRaisesRegex(ValueError,'revoked'):
   stage1.verify_stage1_review_manifest(BASE/'harness-review-manifest.json',BASE/'harness-review-manifest.json.sha256',ROOT,harness['review_aggregate_sha256'],'3db6545c5bcd944f6a2dd5f2d5506fa8cc58bd9a51a3b214f93f0d6b8915a8a3')
  self.assertIn('scripts/tests/test_prebuilt_runtime_stage1_runner.py',{item['path'] for item in harness['files']})

 def test_stage1_review_identity_accepts_live_closed_roster_and_sidecar(self):
  with tempfile.TemporaryDirectory() as td:
   manifest,sidecar,aggregate,digest=write_live_harness_manifest(td)
   identity=stage1.verify_stage1_review_manifest(manifest,sidecar,ROOT,aggregate,digest)
   self.assertEqual(identity['review_aggregate_sha256'],aggregate)
   self.assertEqual(identity['review_manifest_sha256'],digest)

 def test_stage1_review_identity_rejects_empty_roster(self):
  with tempfile.TemporaryDirectory() as td:
   aggregate=canonical_json_sha256([])
   document={'schema':'jarvis.local-provider.review-manifest.v1','review_aggregate_sha256':aggregate,'files':[]}
   manifest=Path(td)/'harness-review-manifest.json'; manifest.write_text(json.dumps(document,sort_keys=True,separators=(',',':'))+'\n')
   digest=hashlib.sha256(manifest.read_bytes()).hexdigest()
   sidecar=Path(td)/'harness-review-manifest.json.sha256'; sidecar.write_text(f'{digest}  {manifest.name}\n')
   with self.assertRaisesRegex(ValueError,'roster'):
    stage1.verify_stage1_review_manifest(manifest,sidecar,ROOT,aggregate,digest)

 def test_stage1_review_identity_rejects_shadow_repository(self):
  with tempfile.TemporaryDirectory() as td:
   shadow=Path(td)/'shadow'; manifest_dir=Path(td)/'manifest'; manifest_dir.mkdir()
   for name in stage1.STAGE1_REVIEW_ROSTER:
    destination=shadow/name; destination.parent.mkdir(parents=True,exist_ok=True); shutil.copy2(ROOT/name,destination)
   manifest,sidecar,aggregate,digest=write_live_harness_manifest(manifest_dir,shadow)
   with self.assertRaisesRegex(ValueError,'repository root'):
    stage1.verify_stage1_review_manifest(manifest,sidecar,shadow,aggregate,digest)

 def test_stage1_review_identity_rejects_predecessor_aggregate(self):
  predecessor='380ab653ee52bd0d9684b1ed5cf3209e05a3c984c4f80e3bed04cebbd4be13d0'
  with tempfile.TemporaryDirectory() as td:
   manifest,sidecar,_aggregate,_digest=write_live_harness_manifest(td)
   document=json.loads(manifest.read_text()); document['review_aggregate_sha256']=predecessor
   manifest.write_text(json.dumps(document,sort_keys=True,separators=(',',':'))+'\n')
   digest=hashlib.sha256(manifest.read_bytes()).hexdigest(); sidecar.write_text(f'{digest}  {manifest.name}\n')
   with self.assertRaisesRegex(ValueError,'revoked'):
    stage1.verify_stage1_review_manifest(manifest,sidecar,ROOT,predecessor,digest)

 def test_receipt_layers_replay_runtime_digest_field_names(self):
  candidate=load_candidate(BASE/'candidate-metadata.json')
  runtime=receipt_layer_records(candidate)[0]
  self.assertEqual(runtime['index_sha256'],candidate['runtime']['index_digest'])
  self.assertEqual(runtime['manifest_sha256'],candidate['runtime']['manifest_digest'])
  self.assertEqual(runtime['config_sha256'],candidate['runtime']['config_digest'])
  self.assertEqual(runtime['layer_sha256'],candidate['runtime']['layer_digest'])

 def test_unknown_or_mutated_identity_fails(self):
  source=json.loads((BASE/'candidate-metadata.json').read_text())
  for mutate in (lambda d:d.update(extra=True),lambda d:d['runtime'].update(layer_digest='0'*64)):
   payload=copy.deepcopy(source); mutate(payload)
   with tempfile.TemporaryDirectory() as td:
    p=Path(td)/'candidate.json'; p.write_text(json.dumps(payload))
    with self.assertRaises(ValueError): load_candidate(p)

 def test_review_replay_rejects_symlinked_member(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td); directory=root/'files'; directory.mkdir()
   target=directory/'target'; target.write_bytes(b'frozen'); (directory/'link').symlink_to('target')
   record={'path':'files/link','bytes':6,'mode':0o644,'sha256':hashlib.sha256(b'frozen').hexdigest()}
   aggregate=hashlib.sha256(json.dumps([record],sort_keys=True,separators=(',',':')).encode()).hexdigest()
   manifest={'schema':'jarvis.local-provider.review-manifest.v1','review_aggregate_sha256':aggregate,'files':[record]}
   path=root/'review.json'; path.write_text(json.dumps(manifest))
   with self.assertRaises(ValueError): verify_review_manifest(path,root,aggregate)

 def test_public_failure_is_one_fixed_redacted_code(self):
  self.assertEqual(redacted_stage1_failure(RuntimeError('Bearer token /private/secret')),'PREBUILT_STAGE1_FAILED')

 def test_receipt_publication_has_no_importable_python_seal(self):
  self.assertFalse(hasattr(stage1,'_RECEIPT_SEAL'))
  self.assertFalse(hasattr(stage1,'Stage1Receipt'))
  self.assertFalse(hasattr(stage1,'publish_stage1_receipt'))

 def test_obsolete_caller_mintable_authorization_and_cli_paths_are_absent(self):
  for name in ('AuthorizationGate','AUTH_SCHEMA','AUTH_KEYS','build_parser'):
   self.assertFalse(hasattr(stage1,name),name)
  for option in ('--authorization','--authorization-sha256','--authorization-ledger','--nonce-fd','--receipt','--workspace-parent'):
   with self.subTest(option=option),mock.patch.object(os,'read',side_effect=AssertionError('fd read')),mock.patch.object(stage1,'run_stage1',side_effect=AssertionError('stage1 run')):
    self.assertEqual(stage1.main([option,'attacker-controlled']),1)

 def test_transfer_authority_derives_only_from_verified_owner_authorization(self):
  candidate=load_candidate(BASE/'candidate-metadata.json')
  with tempfile.TemporaryDirectory() as td:
   paths={name:Path(td)/name for name in ('ledger','workspace_parent','receipt_parent')}
   for path in paths.values(): path.mkdir(mode=0o700)
   fds={name:os.open(path,os.O_RDONLY|os.O_DIRECTORY) for name,path in paths.items()}
   try:
    verified=verified_authorization(candidate,fds,candidate_sha256=CANDIDATE_SHA256,harness_aggregate='44'*32)
    claim=stage1._transfer_authorization_from_verified(verified,candidate,'44'*32)
    self.assertEqual(set(claim),{'authorization_sha256','binding_sha256','generation','key_id','nonce_sha256','purpose'})
    self.assertEqual(claim['generation'],GENERATION)
    for repository,digest,size in claim._artifacts:
     claim.begin(repository,digest,size); claim.finish(True)
    self.assertEqual(claim.complete().public,dict(claim))
    with self.assertRaises(PermissionError): stage1._transfer_authorization_from_verified({},candidate,'44'*32)
   finally:
    for fd in fds.values(): os.close(fd)

 def test_live_disk_preflight_uses_distinct_same_volume_descriptors(self):
  with tempfile.TemporaryDirectory() as td:
   descriptors={}
   try:
    for name in ('artifacts','runtime','evidence'):
     path=Path(td)/name; path.mkdir(); descriptors[name]=os.open(path,os.O_RDONLY|os.O_DIRECTORY)
    result=live_disk_preflight(descriptors,1)
    self.assertGreaterEqual(result['available_bytes'],1)
    regular=Path(td)/'not-a-directory'; regular.write_bytes(b'x'); regular_fd=os.open(regular,os.O_RDONLY)
    try:
     wrong_kind=dict(descriptors); wrong_kind['evidence']=regular_fd
     with self.assertRaises(ValueError): live_disk_preflight(wrong_kind,1)
    finally: os.close(regular_fd)
    devices={fd:SimpleNamespace(st_dev=index,st_ino=index,st_mode=stat.S_IFDIR|0o700) for index,fd in enumerate(descriptors.values(),1)}
    with self.assertRaises(ValueError):
     live_disk_preflight(descriptors,1,fstat_fn=lambda fd:devices[fd],fstatvfs_fn=os.fstatvfs)
    one_device={fd:SimpleNamespace(st_dev=7,st_ino=index,st_mode=stat.S_IFDIR|0o700) for index,fd in enumerate(descriptors.values(),1)}
    no_space=lambda _fd:SimpleNamespace(f_bavail=1,f_frsize=1)
    with self.assertRaises(OSError):
     live_disk_preflight(descriptors,2,fstat_fn=lambda fd:one_device[fd],fstatvfs_fn=no_space)
   finally:
    for fd in descriptors.values(): os.close(fd)

 def test_candidate_workspace_cannot_downgrade_live_disk_requirement(self):
  candidate=load_candidate(BASE/'candidate-metadata.json')
  with mock.patch('scripts.prebuilt_runtime_stage1.PrivateWorkspace.create',return_value='workspace') as create:
   self.assertEqual(create_stage1_workspace(42,GENERATION,candidate),'workspace')
  create.assert_called_once_with(42,GENERATION,candidate['provenance']['minimum_free_bytes_before_each_transfer'])

 def test_candidate_requires_per_layer_installed_size_or_extraction_cap(self):
  candidate=json.loads((BASE/'candidate-metadata.json').read_text()); candidate['runtime'].pop('installed_bytes')
  payload=canonical_json_bytes(candidate)
  with tempfile.TemporaryDirectory() as td:
   path=Path(td)/'candidate.json'; path.write_bytes(payload)
   with mock.patch.object(stage1,'CANDIDATE_SHA256',hashlib.sha256(payload).hexdigest()),self.assertRaisesRegex(ValueError,'layer extraction identity'):
    load_candidate(path)

 def test_strict_redacted_receipt_publication_and_replay(self):
  candidate=load_candidate(BASE/'candidate-metadata.json')
  formulas=[
   (candidate['runtime']['formula'],candidate['runtime']['version'],candidate['runtime']['license']),
   *((item['formula'],item['version']+(f"_{item['rebuild']}" if item.get('rebuild') else ''),item['license']) for item in candidate['dependencies']),
  ]
  roster_by_path={}; licenses=[]
  def add_file(path,size,digest,mode=0o644,content_type='data'):
   parts=path.split('/')
   for index in range(1,len(parts)):
    parent='/'.join(parts[:index]); roster_by_path.setdefault(parent,{'path':parent,'kind':'directory','bytes':0,'mode':0o755})
   roster_by_path[path]={'path':path,'kind':'file','bytes':size,'mode':mode,'sha256':digest,'content_type':content_type}
  for formula,version,spdx in formulas:
   path=f'{formula}/{version}/LICENSE'; digest=hashlib.sha256(formula.encode()).hexdigest()
   add_file(path,len(formula),digest)
   licenses.append({'formula':formula,'declared_spdx':spdx,'path':path,'bytes':len(formula),'sha256':digest})
  cli_hash=hashlib.sha256(b'x'*128).hexdigest(); cli_path='llama.cpp/0.4.1/bin/llama-cli'; add_file(cli_path,128,cli_hash,0o755,'mach-o')
  for item in [candidate['runtime'],*candidate['dependencies']]:
   if 'installed_bytes' not in item: continue
   version=item['version']+(f"_{item['rebuild']}" if item.get('rebuild') else ''); prefix=f"{item['formula']}/{version}/"
   observed=sum(record['bytes'] for record in roster_by_path.values() if record.get('kind')=='file' and record['path'].startswith(prefix))
   remaining=item['installed_bytes']-observed
   self.assertGreater(remaining,0); add_file(prefix+'share/runtime.data',remaining,hashlib.sha256((item['formula']+' installed-size').encode()).hexdigest())
  machos=[{'path':cli_path,'sha256':cli_hash,'slices':[
   {'offset':0,'bytes':128,'cputype':0x0100000c,'cpusubtype':0,'filetype':2,'flags':0,'code_signature':{'present':True,'data_offset':16,'bytes':104,'slots':[{'type':0,'magic':'fade0c02','bytes':84,'code_directory':{'version':0x20001,'flags':0,'identifier':'fixture','special_slots':0,'code_slots':1,'code_limit':16,'hash_size':32,'hash_type':2,'platform':0,'page_size':4096,'hashes_verified':True}}]}},
  ],'loads':[],'rpaths':[],'ids':[]}]
  roster=sorted(roster_by_path.values(),key=lambda item:item['path']); authorization_sha256='2'*64
  cleanup={'files':1+len(receipt_layer_records(candidate))+sum(item['kind'] in ('file','hardlink') for item in roster),'directories':3+sum(item['kind']=='directory' for item in roster),'links':0,'root_removed':True,'descriptors_closed':True,'residue_entries':0}
  receipt={
   'schema':'jarvis.local-provider.prebuilt-stage1-receipt.v1','candidate_id':candidate['candidate_id'],'candidate_sha256':CANDIDATE_SHA256,
   'review_aggregate_sha256':AGG,'run_id':'123e4567-e89b-42d3-a456-426614174000',
   'authorization':{'authorization_sha256':authorization_sha256,'binding_sha256':'3'*64,'generation':GENERATION,'key_id':'4'*64,'nonce_sha256':'1'*64,'purpose':'stage1_bottle_transfer'},
   'upstream':{'formula':candidate['runtime']['formula'],'version':candidate['runtime']['version'],'commit':candidate['runtime']['upstream_commit']},
   'layers':receipt_layer_records(candidate),'extracted':roster,'mach_o':machos,'licenses':sorted(licenses,key=lambda item:item['formula']),'cleanup':cleanup,'verdict':'MODEL_TRANSFER_BLOCKED',
  }
  digest=validate_stage1_receipt(receipt,candidate,AGG,authorization_sha256)
  encoded=json.dumps(receipt,sort_keys=True)
  self.assertNotIn('Bearer',encoded); self.assertNotIn('/private/',encoded); self.assertEqual(receipt['verdict'],'MODEL_TRANSFER_BLOCKED')
  def add_link_cycle(value):
   value['extracted'].extend([
    {'path':'llama.cpp/0.4.1/a','kind':'symlink','bytes':0,'mode':0o777,'link_target':'b'},
    {'path':'llama.cpp/0.4.1/b','kind':'symlink','bytes':0,'mode':0o777,'link_target':'a'},
   ]); value['extracted'].sort(key=lambda item:item['path'])
  def add_rogue_path(value):
   value['extracted'].extend([
    {'path':'rogue','kind':'directory','bytes':0,'mode':0o700},
    {'path':'rogue/payload','kind':'file','bytes':1,'mode':0o600,'sha256':hashlib.sha256(b'x').hexdigest(),'content_type':'data'},
   ]); value['extracted'].sort(key=lambda item:item['path']); value['cleanup']['directories']+=1; value['cleanup']['files']+=1
  def make_world_writable(value):
   next(item for item in value['extracted'] if item['kind']=='file')['mode']=0o777
  def exceed_expansion_limit_and_replace_license(value):
   license_item=value['licenses'][0]; roster_item=next(item for item in value['extracted'] if item['path']==license_item['path']); changed_hash=hashlib.sha256(b'changed-license').hexdigest()
   roster_item.update(bytes=999999999,sha256=changed_hash); license_item.update(bytes=999999999,sha256=changed_hash)
  def change_declared_installed_size(value):
   record=next(item for item in value['extracted'] if item['path'].endswith('/share/runtime.data'))
   record['bytes']+=1; record['sha256']=hashlib.sha256(b'changed-installed-size').hexdigest()
  def inject_secret_like_dylib_id(value):
   value['mach_o'][0]['slices'][0]['filetype']=6; value['mach_o'][0]['ids']=['Bearer /private/secret']
  for mutate in (
   lambda value:value.update(raw_output='secret'),
   lambda value:value.update(verdict='MODEL_TRANSFER_ALLOWED'),
   lambda value:value['mach_o'][0]['loads'].append({'path':'/private/secret','resolved':'system'}),
   lambda value:value['mach_o'][0]['rpaths'].append('/tmp/unreviewed'),
   lambda value:value['mach_o'][0]['slices'][0].update(bytes=33),
   lambda value:value['mach_o'].clear(),
   lambda value:value['cleanup'].update(files=0,directories=0,links=0),
   lambda value:value['licenses'][0].update(path=cli_path,bytes=128,sha256=cli_hash),
   lambda value:value['mach_o'][0]['slices'][0].pop('code_signature'),
   lambda value:value['mach_o'][0]['slices'][0]['code_signature']['slots'][0]['code_directory'].update(hashes_verified=False),
   add_link_cycle,
   add_rogue_path,
   make_world_writable,
   exceed_expansion_limit_and_replace_license,
   change_declared_installed_size,
   inject_secret_like_dylib_id,
  ):
   changed=copy.deepcopy(receipt); mutate(changed)
   with self.assertRaises(ValueError): validate_stage1_receipt(changed,candidate,AGG,authorization_sha256)
  with self.assertRaises(ValueError): validate_stage1_receipt(receipt,candidate,AGG,'3'*64)
  with self.assertRaises((TypeError,ValueError)):
   build_stage1_receipt(candidate,AGG,receipt['run_id'],receipt['authorization'],receipt['layers'],{'roster':roster,'mach_o':machos,'licenses':licenses},cleanup)
  with self.assertRaisesRegex(PermissionError,'provenance'):
   load_stage1_receipt(-1,f'stage1-{GENERATION}.json',candidate,AGG,authorization_sha256,GENERATION)

if __name__=='__main__': unittest.main()
