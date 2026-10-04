import hashlib
import io
import json
import os
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from scripts import prebuilt_runtime_stage1 as stage1
from scripts.tests.stage1_owner_test_support import GENERATION,verified_authorization

REPO=Path(__file__).resolve().parents[2]


class FakeResponse:
 def __init__(self,status,body=b'',headers=()):
  self.status=status; self._body=io.BytesIO(body); self._headers=list(headers); self.headers_checked=False; self.preflight_complete=False; self.requires_preflight=False
 def read(self,amount=-1):
  if self.requires_preflight and not self.preflight_complete: raise AssertionError('blob body read before adjacent disk preflight')
  return self._body.read(amount)
 def getheaders(self): self.headers_checked=True; return list(self._headers)
 def close(self): pass


class FakeConnection:
 def __init__(self,origin,routes,calls): self.origin=origin; self.routes=routes; self.calls=calls; self.response=None
 def request(self,method,target,body=None,headers=None):
  self.calls.append((self.origin,method,target,dict(headers or {})))
  self.response=self.routes[self.origin].pop(0)
 def getresponse(self): return self.response
 def close(self): pass


def macho_command(command,name,header_bytes):
 encoded=name.encode()+b'\0'; size=(header_bytes+len(encoded)+7)&~7
 prefix=struct.pack('<IIIIII',command,size,24,0,0,0) if header_bytes==24 else struct.pack('<III',command,size,12)
 return prefix+encoded+b'\0'*(size-len(prefix)-len(encoded))


def signed_macho():
 commands=[]; data_offset=32+16; identifier=b'fixture\0'; hash_offset=44+len(identifier); code_directory_length=hash_offset+32
 signature_size=20+code_directory_length; signature_command=struct.pack('<IIII',0x1d,16,data_offset,signature_size)
 payload=b''.join([*commands,signature_command]); prefix=struct.pack('<IiiIIIII',0xfeedfacf,0x0100000c,0,2,1,len(payload),0,0)+payload
 code_directory_header=struct.pack('>9I4BI',0xfade0c02,code_directory_length,0x20001,0,hash_offset,44,0,1,data_offset,32,2,0,12,0)
 code_directory=code_directory_header+identifier+hashlib.sha256(prefix).digest()
 signature=struct.pack('>IIIII',0xfade0cc0,20+len(code_directory),1,0,20)+code_directory
 return prefix+signature


def layer(entries):
 output=io.BytesIO()
 with tarfile.open(fileobj=output,mode='w:gz',format=tarfile.USTAR_FORMAT) as bundle:
  for path,data,mode in entries:
   info=tarfile.TarInfo(path); info.size=len(data); info.mode=mode; bundle.addfile(info,io.BytesIO(data))
 return output.getvalue()


def response_json(value,media_type):
 body=stage1.canonical_json_bytes(value); digest=hashlib.sha256(body).hexdigest()
 response=FakeResponse(200,body,[('Content-Length',str(len(body))),('Content-Type',media_type),('Docker-Content-Digest','sha256:'+digest)])
 return body,digest,response


class Stage1RunnerTests(unittest.TestCase):
 def test_cli_reads_only_fixed_inherited_fds_and_rejects_every_argument(self):
  verified=object(); resources=stage1.Stage1Resources.from_script(stage1.Path(stage1.__file__))
  with mock.patch.object(stage1.Stage1Resources,'from_script',return_value=resources),mock.patch.object(stage1,'_read_inherited_fd',side_effect=[b'envelope',b'n'*32]) as reader,mock.patch.object(stage1,'_read_path',side_effect=[b'anchor',b'binding']),mock.patch.object(stage1.os,'fstat',return_value=SimpleNamespace()),mock.patch.object(stage1.owner_auth,'verify_owner_authorization',return_value=verified) as verify,mock.patch.object(stage1,'run_stage1',return_value='d'*64) as run,mock.patch('builtins.print'):
   self.assertEqual(stage1.main([]),0)
  self.assertEqual(reader.call_args_list,[mock.call(stage1.AUTHORIZATION_FD,stage1.owner_auth.MAX_ENVELOPE_BYTES),mock.call(stage1.NONCE_FD,32,32)])
  verify.assert_called_once(); run.assert_called_once_with(resources,verified,stage1.LEDGER_FD,stage1.WORKSPACE_PARENT_FD,stage1.RECEIPT_PARENT_FD)
  for option in ('--authorization','--authorization-sha256','--authorization-ledger','--nonce-fd','--receipt','--workspace-parent','--candidate','--review-manifest'):
   with self.subTest(option=option),mock.patch.object(stage1,'_read_inherited_fd',side_effect=AssertionError('descriptor read')),mock.patch('builtins.print'):
    self.assertEqual(stage1.main([option,'attacker']),1)

 def test_cli_consumes_real_temporary_descriptors_at_fixed_numbers(self):
  child="""
from unittest import mock
from scripts import prebuilt_runtime_stage1 as stage1
sentinel=object(); resources=stage1.Stage1Resources.from_script(stage1.Path(stage1.__file__))
with mock.patch.object(stage1,'_read_path',side_effect=[b'anchor',b'binding']), mock.patch.object(stage1.owner_auth,'SecurityFrameworkSignatureVerifier',return_value=object()), mock.patch.object(stage1.owner_auth,'verify_owner_authorization',return_value=sentinel) as verify, mock.patch.object(stage1,'run_stage1',return_value='d'*64) as run:
 assert stage1.main([])==0
assert verify.call_args.args[0]==b'fixed-envelope'
assert verify.call_args.args[1]==bytes(range(32))
assert run.call_args.args[2:]==(5,6,7)
"""
  with tempfile.TemporaryDirectory() as td:
   root=Path(td); sources=[]
   envelope=root/'envelope'; envelope.write_bytes(b'fixed-envelope'); nonce=root/'nonce'; nonce.write_bytes(bytes(range(32)))
   sources.extend([os.open(envelope,os.O_RDONLY),os.open(nonce,os.O_RDONLY)])
   for name in ('ledger','workspace','receipt'):
    path=root/name; path.mkdir(mode=0o700); sources.append(os.open(path,os.O_RDONLY|os.O_DIRECTORY))
   safe=[stage1.fcntl.fcntl(fd,stage1.fcntl.F_DUPFD,20) for fd in sources]; saved={}
   try:
    for source,target in zip(safe,(3,4,5,6,7)):
     try: saved[target]=stage1.fcntl.fcntl(target,stage1.fcntl.F_DUPFD,30)
     except OSError: saved[target]=None
     os.dup2(source,target)
    result=subprocess.run([sys.executable,'-c',child],cwd=REPO,pass_fds=(3,4,5,6,7),capture_output=True,text=True)
    self.assertEqual(result.returncode,0,result.stderr)
   finally:
    for target,original in saved.items():
     if original is None:
      try: os.close(target)
      except OSError: pass
     else: os.dup2(original,target); os.close(original)
    for fd in [*safe,*sources]: os.close(fd)

 def test_authorization_ledger_lock_excludes_concurrent_process(self):
  child="""
import os,sys
from pathlib import Path
from scripts import prebuilt_runtime_stage1 as stage1
fd=os.open(Path(sys.argv[1]),os.O_RDONLY|os.O_DIRECTORY)
try:
 with stage1.authorization_ledger_lock(fd): pass
except PermissionError: os._exit(17)
os._exit(0)
"""
  with tempfile.TemporaryDirectory() as td:
   ledger=Path(td)/'ledger'; ledger.mkdir(mode=0o700); ledger_fd=os.open(ledger,os.O_RDONLY|os.O_DIRECTORY)
   try:
    with stage1.authorization_ledger_lock(ledger_fd):
     result=subprocess.run([sys.executable,'-c',child,str(ledger)],cwd=REPO)
    self.assertEqual(result.returncode,17)
   finally: os.close(ledger_fd)

 def test_one_generation_marker_is_exclusive_across_processes(self):
  child="""
import os,sys
from pathlib import Path
from scripts import prebuilt_runtime_stage1 as stage1
fd=os.open(sys.argv[1],os.O_RDONLY|os.O_DIRECTORY); marker=stage1._decode_json(Path(sys.argv[2]).read_bytes())
try: stage1._create_ledger_marker(fd,marker['generation'],marker)
except PermissionError: raise SystemExit(17)
finally: os.close(fd)
"""
  with tempfile.TemporaryDirectory() as td:
   root=Path(td); ledger=root/'ledger'; ledger.mkdir(mode=0o700)
   marker={key:None for key in stage1.LEDGER_KEYS}
   marker.update(schema=stage1.LEDGER_SCHEMA,state='authorized',generation=GENERATION,authorization_sha256='a'*64,binding_sha256='b'*64,key_id='c'*64,candidate_id='fixture',candidate_sha256='d'*64,review_aggregate_sha256='e'*64,review_manifest_sha256='f'*64)
   marker_path=root/'marker.json'; marker_path.write_bytes(stage1.canonical_json_bytes(marker))
   processes=[subprocess.Popen([sys.executable,'-c',child,str(ledger),str(marker_path)],cwd=REPO) for _ in range(2)]
   self.assertEqual(sorted(process.wait(timeout=10) for process in processes),[0,17])
   fd=os.open(ledger,os.O_RDONLY|os.O_DIRECTORY)
   try: self.assertEqual(stage1._read_ledger_marker(fd,GENERATION),marker)
   finally: os.close(fd)

 def test_registered_generation_workspace_recovers_after_abrupt_stop(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td); paths={name:root/name for name in ('ledger','workspace_parent','receipt_parent')}
   for path in paths.values(): path.mkdir(mode=0o700)
   fds={name:os.open(path,os.O_RDONLY|os.O_DIRECTORY) for name,path in paths.items()}
   candidate=stage1.load_candidate(REPO/'docs/spikes/005-prebuilt-runtime-candidate/candidate-metadata.json')
   try:
    authorization=verified_authorization(candidate,fds,candidate_sha256=stage1.CANDIDATE_SHA256,harness_aggregate='44'*32)
    workspace=stage1.create_stage1_workspace(fds['workspace_parent'],GENERATION,candidate)
    stage1._register_authorization(fds['ledger'],authorization,workspace,candidate,'44'*32)
    residue=os.open('crash.bin',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600,dir_fd=workspace.fd('artifacts')); os.write(residue,b'owned'); os.close(residue)
    for descriptor in workspace.role_fds.values(): os.close(descriptor)
    os.close(workspace.root_fd); os.close(workspace.parent_fd)
    self.assertEqual(stage1.recover_stage1_workspaces(fds['ledger'],fds['workspace_parent']),1)
    self.assertEqual(list(paths['workspace_parent'].iterdir()),[])
    self.assertEqual(stage1._read_ledger_marker(fds['ledger'],GENERATION)['state'],'recovered')
   finally:
    for fd in fds.values(): os.close(fd)

 def test_tampered_registered_owner_file_blocks_recovery_without_deletion(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td); paths={name:root/name for name in ('ledger','workspace_parent','receipt_parent')}
   for path in paths.values(): path.mkdir(mode=0o700)
   fds={name:os.open(path,os.O_RDONLY|os.O_DIRECTORY) for name,path in paths.items()}
   candidate=stage1.load_candidate(REPO/'docs/spikes/005-prebuilt-runtime-candidate/candidate-metadata.json')
   try:
    authorization=verified_authorization(candidate,fds,candidate_sha256=stage1.CANDIDATE_SHA256,harness_aggregate='44'*32)
    workspace=stage1.create_stage1_workspace(fds['workspace_parent'],GENERATION,candidate)
    stage1._register_authorization(fds['ledger'],authorization,workspace,candidate,'44'*32)
    owner=paths['workspace_parent']/workspace.name/'.saturday-stage1-owner.json'; owner.write_bytes(b'{}')
    for descriptor in workspace.role_fds.values(): os.close(descriptor)
    os.close(workspace.root_fd); os.close(workspace.parent_fd)
    with self.assertRaises(PermissionError): stage1.recover_stage1_workspaces(fds['ledger'],fds['workspace_parent'])
    self.assertTrue((paths['workspace_parent']/workspace.name).is_dir())
   finally:
    for fd in fds.values(): os.close(fd)

 def test_recovery_does_not_delete_unregistered_matching_directory(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td); ledger=root/'ledger'; ledger.mkdir(mode=0o700); parent=root/'workspace'; parent.mkdir(mode=0o700)
   unrelated=parent/('saturday-prebuilt-'+'a'*32); unrelated.mkdir(mode=0o700); payload=unrelated/'user-owned.bin'; payload.write_bytes(b'not stage-1 owned')
   ledger_fd=os.open(ledger,os.O_RDONLY|os.O_DIRECTORY); parent_fd=os.open(parent,os.O_RDONLY|os.O_DIRECTORY)
   try: self.assertEqual(stage1.recover_stage1_workspaces(ledger_fd,parent_fd),0)
   finally: os.close(ledger_fd); os.close(parent_fd)
   self.assertEqual(payload.read_bytes(),b'not stage-1 owned')

 def make_fixture(self,root):
  packages=[('llama.cpp','1.0',None,'MIT'),('ggml','1.0',None,'MIT'),('libomp','1.0',None,'MIT'),('openssl@3','1.0',None,'Apache-2.0'),('ca-certificates','1.0',1,'MPL-2.0')]
  items=[]; metadata=[]
  for index,(formula,version,rebuild,license_name) in enumerate(packages):
   package_version=version+(f'_{rebuild}' if rebuild else '')
   entries=[(f'{formula}/{package_version}/LICENSE',formula.encode(),0o644)]
   if formula=='llama.cpp': entries.append((f'{formula}/{package_version}/bin/llama-cli',signed_macho(),0o755))
   payload=layer(entries); layer_digest=hashlib.sha256(payload).hexdigest()
   item={'formula':formula,'version':version,'repository':f'homebrew/core/{formula.replace("@","/")}','layer':layer_digest,'layer_bytes':len(payload),'license':license_name}
   if index<4: item['installed_bytes']=sum(len(data) for _path,data,_mode in entries)
   if rebuild: item['rebuild']=rebuild; item['maximum_extracted_bytes']=4096; item['post_install_forbidden']=True
   if index<4:
    config_value={'fixture':formula}; config_body,config_digest,config_response=response_json(config_value,'application/vnd.oci.image.config.v1+json')
    manifest_value={'schemaVersion':2,'mediaType':'application/vnd.oci.image.manifest.v1+json','config':{'mediaType':'application/vnd.oci.image.config.v1+json','digest':'sha256:'+config_digest,'size':len(config_body)},'layers':[{'mediaType':'application/vnd.oci.image.layer.v1.tar+gzip','digest':'sha256:'+layer_digest,'size':len(payload)}]}
    manifest_body,manifest_digest,manifest_response=response_json(manifest_value,'application/vnd.oci.image.manifest.v1+json')
    annotation=f'{version}.arm64_golden_gate'
    index_value={'schemaVersion':2,'mediaType':'application/vnd.oci.image.index.v1+json','manifests':[{'mediaType':'application/vnd.oci.image.manifest.v1+json','digest':'sha256:'+manifest_digest,'size':len(manifest_body),'annotations':{'org.opencontainers.image.ref.name':annotation},'platform':{'architecture':'arm64','os':'darwin'}}]}
    _index_body,index_digest,index_response=response_json(index_value,'application/vnd.oci.image.index.v1+json')
    item.update({'index':index_digest,'manifest':manifest_digest,'config':config_digest,'child_annotation_ref':annotation})
    metadata.append((index_response,manifest_response,config_response))
   else: metadata.append(None)
   items.append((item,payload))
  runtime=dict(items[0][0]); runtime.update({'distribution':'Homebrew OCI bottle','target':'arm64_golden_gate','upstream_commit':'a'*40,'index_digest':runtime.pop('index'),'manifest_digest':runtime.pop('manifest'),'config_digest':runtime.pop('config'),'layer_digest':runtime.pop('layer'),'oci_repository':runtime.pop('repository')})
  dependencies=[dict(value) for value,_payload in items[1:]]
  total=sum(len(payload) for _item,payload in items)
  candidate={
   'schema':'saturday.local-provider.prebuilt-candidate.v2','candidate_id':'fixture-stage1','decision':'metadata_candidate_only','frozen_on':'2026-09-20',
   'authorizations':{'bottle_transfer':{'state':'not_granted','artifact_count':5,'maximum_bytes':total,'nonce_sha256':None},'model_transfer':{'state':'blocked_until_relocated_runtime_review','artifact_sha256':'b'*64,'artifact_bytes':1,'nonce_sha256':None}},
   'model':{'bytes':1,'sha256':'b'*64},'runtime':runtime,'dependencies':dependencies,
   'provenance':{'verified_layer_total_bytes':total,'bottle_transfer_cap_bytes':total,'maximum_total_extracted_bytes':1048576,'maximum_entries':100,'minimum_free_bytes_before_each_transfer':1,'origin_policy':{'registry':'https://ghcr.io:443','token_realm':'https://ghcr.io:443/token','service':'ghcr.io','allowed_blob_redirect_origin':'https://pkg-containers.githubusercontent.com:443','authorization_forwarding':'same-origin-only','strip_authorization_on_cross_origin':True,'redirect_limit':1,'reject_redirect_from_blob_origin':True,'content_length_required':True}},
   'two_stage_gate':{'stage_1':'fixture','stage_2':'blocked'},
   'security':{'no_local_source_build':True,'no_child_downloader':True,'no_formula_execution':True,'network_denied_and_fork_denied_at_inference':True,'product_activation':False},
  }
  candidate_path=root/'candidate.json'; candidate_bytes=stage1.canonical_json_bytes(candidate); candidate_path.write_bytes(candidate_bytes); candidate_sha=hashlib.sha256(candidate_bytes).hexdigest()
  records=[]
  for name in sorted(stage1.STAGE1_REVIEW_ROSTER):
   path=REPO/name; info=path.stat()
   records.append({'path':name,'bytes':info.st_size,'mode':stat.S_IMODE(info.st_mode),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()})
  aggregate=stage1.canonical_json_sha256(records)
  manifest={'schema':'saturday.local-provider.review-manifest.v1','review_aggregate_sha256':aggregate,'files':records}
  manifest_path=root/'harness-review-manifest.json'; manifest_path.write_bytes(stage1.canonical_json_bytes(manifest)); manifest_sha=hashlib.sha256(manifest_path.read_bytes()).hexdigest()
  Path(str(manifest_path)+'.sha256').write_text(f'{manifest_sha}  {manifest_path.name}\n')
  routes={'https://ghcr.io:443':[],'https://pkg-containers.githubusercontent.com:443':[]}
  challenge_template='Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:{repository}:pull"'
  for index,((item,payload),metadata_responses) in enumerate(zip(items,metadata)):
   repository=item['repository']; challenge=challenge_template.format(repository=repository)
   kind='manifests' if metadata_responses else 'blobs'
   routes['https://ghcr.io:443'].append(FakeResponse(401,b'{}',[('Content-Length','2'),('WWW-Authenticate',challenge)]))
   token_body=b'{"token":"fixture-token"}'
   routes['https://ghcr.io:443'].append(FakeResponse(200,token_body,[('Content-Length',str(len(token_body)))]))
   if metadata_responses: routes['https://ghcr.io:443'].extend(metadata_responses)
   redirect=f'https://pkg-containers.githubusercontent.com:443/layer-{index}'
   routes['https://ghcr.io:443'].append(FakeResponse(307,headers=[('Location',redirect)]))
   layer_response=FakeResponse(200,payload,[('Content-Length',str(len(payload)))]); layer_response.requires_preflight=True
   routes['https://pkg-containers.githubusercontent.com:443'].append(layer_response)
  return candidate_path,candidate_sha,aggregate,routes

 def pipeline_context(self,root):
  paths={name:root/name for name in ('ledger','workspace_parent','receipt_parent')}
  for path in paths.values(): path.mkdir(mode=0o700)
  candidate_path,candidate_sha,aggregate,routes=self.make_fixture(root)
  candidate=json.loads(candidate_path.read_bytes()); fds={name:os.open(path,os.O_RDONLY|os.O_DIRECTORY) for name,path in paths.items()}
  authorization=verified_authorization(candidate,fds,candidate_sha256=candidate_sha,harness_aggregate=aggregate)
  resources=stage1.Stage1Resources(root,candidate_path,candidate_path,candidate_path,candidate_path,candidate_path,candidate_path,candidate_path,candidate_path)
  calls=[]
  return {'paths':paths,'fds':fds,'candidate':candidate,'candidate_sha':candidate_sha,'aggregate':aggregate,'routes':routes,'calls':calls,'authorization':authorization,'resources':resources,'connection_factory':lambda origin:FakeConnection(origin,routes,calls)}

 def run_pipeline(self,context,**patches):
  with mock.patch.object(stage1,'CANDIDATE_SHA256',context['candidate_sha']),mock.patch.object(stage1,'_replay_owner_resources',return_value=(context['candidate'],context['aggregate'])):
   return stage1.run_stage1(context['resources'],context['authorization'],context['fds']['ledger'],context['fds']['workspace_parent'],context['fds']['receipt_parent'],connection_factory=context['connection_factory'],run_id_factory=lambda:'123e4567-e89b-42d3-a456-426614174000')

 def test_descriptor_identity_mismatch_stops_before_resource_replay_lock_or_transport(self):
  with tempfile.TemporaryDirectory() as td:
   context=self.pipeline_context(Path(td))
   try:
    with mock.patch.object(stage1,'_replay_owner_resources',side_effect=AssertionError('resource replay')),mock.patch.object(stage1,'authorization_ledger_lock',side_effect=AssertionError('ledger lock')),self.assertRaises(PermissionError):
     stage1.run_stage1(context['resources'],context['authorization'],context['fds']['ledger'],context['fds']['ledger'],context['fds']['receipt_parent'],connection_factory=lambda _origin:self.fail('network used'))
   finally:
    for fd in context['fds'].values(): os.close(fd)

 def test_complete_five_artifact_pipeline_publishes_exact_generation_receipt_and_replays_idempotently(self):
  with tempfile.TemporaryDirectory() as td:
   context=self.pipeline_context(Path(td)); responses=list(context['routes']['https://pkg-containers.githubusercontent.com:443']); actual=stage1.live_disk_preflight
   try:
    def adjacent(descriptors,required):
     ready=[value for value in responses if value.headers_checked and not value.preflight_complete]
     if ready: ready[0].preflight_complete=True
     return actual(descriptors,required)
    with mock.patch.object(stage1,'live_disk_preflight',side_effect=adjacent): digest=self.run_pipeline(context)
    receipt_path=context['paths']['receipt_parent']/context['authorization'].receipt_name
    receipt=json.loads(receipt_path.read_bytes())
    self.assertEqual(digest,hashlib.sha256(receipt_path.read_bytes()).hexdigest())
    self.assertEqual(receipt['verdict'],'MODEL_TRANSFER_BLOCKED'); self.assertEqual(len(receipt['layers']),5)
    self.assertEqual(set(receipt['authorization']),{'authorization_sha256','binding_sha256','generation','key_id','nonce_sha256','purpose'})
    self.assertEqual(receipt['authorization']['generation'],GENERATION)
    self.assertEqual(stage1._read_ledger_marker(context['fds']['ledger'],GENERATION)['state'],'published')
    self.assertEqual(list(context['paths']['workspace_parent'].iterdir()),[])
    with mock.patch.object(stage1,'create_stage1_workspace',side_effect=AssertionError('workspace created')),mock.patch.object(stage1,'_replay_owner_resources',return_value=(context['candidate'],context['aggregate'])),mock.patch.object(stage1,'CANDIDATE_SHA256',context['candidate_sha']):
     self.assertEqual(stage1.run_stage1(context['resources'],context['authorization'],context['fds']['ledger'],context['fds']['workspace_parent'],context['fds']['receipt_parent'],connection_factory=lambda _origin:self.fail('network used')),digest)
   finally:
    for fd in context['fds'].values(): os.close(fd)

 def test_cleaned_receipt_gap_reconciles_without_workspace_or_network_and_tampering_fails_closed(self):
  with tempfile.TemporaryDirectory() as td:
   context=self.pipeline_context(Path(td)); actual_replace=stage1._replace_ledger_marker; failed=False
   for response in context['routes']['https://pkg-containers.githubusercontent.com:443']: response.requires_preflight=False
   def fail_published(fd,generation,marker):
    nonlocal failed
    if marker['state']=='published' and not failed: failed=True; raise OSError('publication marker gap')
    return actual_replace(fd,generation,marker)
   try:
    with mock.patch.object(stage1,'_replace_ledger_marker',side_effect=fail_published),self.assertRaisesRegex(OSError,'publication marker gap'): self.run_pipeline(context)
    receipt_path=context['paths']['receipt_parent']/context['authorization'].receipt_name
    self.assertEqual(stage1._read_ledger_marker(context['fds']['ledger'],GENERATION)['state'],'cleaned')
    with mock.patch.object(stage1,'create_stage1_workspace',side_effect=AssertionError('workspace created')),mock.patch.object(stage1,'_replay_owner_resources',return_value=(context['candidate'],context['aggregate'])),mock.patch.object(stage1,'CANDIDATE_SHA256',context['candidate_sha']):
     digest=stage1.run_stage1(context['resources'],context['authorization'],context['fds']['ledger'],context['fds']['workspace_parent'],context['fds']['receipt_parent'],connection_factory=lambda _origin:self.fail('network used'))
    self.assertEqual(digest,hashlib.sha256(receipt_path.read_bytes()).hexdigest())
    marker=stage1._read_ledger_marker(context['fds']['ledger'],GENERATION); marker['state']='cleaned'; stage1._replace_ledger_marker(context['fds']['ledger'],GENERATION,marker); receipt_path.write_bytes(b'{}')
    with mock.patch.object(stage1,'create_stage1_workspace',side_effect=AssertionError('workspace created')),mock.patch.object(stage1,'_replay_owner_resources',return_value=(context['candidate'],context['aggregate'])),mock.patch.object(stage1,'CANDIDATE_SHA256',context['candidate_sha']),self.assertRaises((ValueError,PermissionError)):
     stage1.run_stage1(context['resources'],context['authorization'],context['fds']['ledger'],context['fds']['workspace_parent'],context['fds']['receipt_parent'],connection_factory=lambda _origin:self.fail('network used'))
   finally:
    for fd in context['fds'].values(): os.close(fd)


if __name__=='__main__': unittest.main()
