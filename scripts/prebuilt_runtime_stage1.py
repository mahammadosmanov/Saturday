#!/usr/bin/env python3
"""Capability-gated stdlib contracts for prebuilt runtime stage 1."""
from __future__ import annotations
import contextlib,dataclasses,fcntl,hashlib,hmac,http.client,io,json,os,posixpath,re,stat,struct,sys,tarfile,time,unicodedata,uuid
from pathlib import Path
from typing import Any,Dict,List,Mapping,Optional,Tuple
from urllib.parse import urlencode,urlparse

try:
 from scripts import stage1_owner_authorization as owner_auth
except ImportError:
 import stage1_owner_authorization as owner_auth

CANDIDATE_SHA256='b2672f1a00121c09077bc447da7fff720238012daf2ecd645add58c48124a5bb'
REVIEW_SCHEMA='saturday.local-provider.review-manifest.v1'
AUTHORIZATION_FD=3
NONCE_FD=4
LEDGER_FD=5
WORKSPACE_PARENT_FD=6
RECEIPT_PARENT_FD=7
STAGE1_REVIEW_ROSTER=frozenset({
 'docs/spikes/005-prebuilt-runtime-candidate/README.md',
 'docs/spikes/005-prebuilt-runtime-candidate/candidate-metadata.json',
 'docs/spikes/005-prebuilt-runtime-candidate/candidate-metadata.json.sha256',
 'docs/spikes/005-prebuilt-runtime-candidate/review-manifest.json',
 'docs/spikes/005-prebuilt-runtime-candidate/review-manifest.json.sha256',
 'scripts/prebuilt_runtime_stage1.py',
 'scripts/tests/test_prebuilt_runtime_stage1_candidate.py',
 'scripts/tests/test_prebuilt_runtime_stage1_extract.py',
 'scripts/tests/test_prebuilt_runtime_stage1_runner.py',
 'scripts/tests/test_prebuilt_runtime_stage1_transport.py',
})
REVOKED_REVIEW_AGGREGATES=frozenset({
 'f5375688a80c0447741b1fde33242db1f3e6cb4a89f5fe2cca7cfe9c9361bae0',
 '380ab653ee52bd0d9684b1ed5cf3209e05a3c984c4f80e3bed04cebbd4be13d0',
 '1c49497d67bc21f3a1a3edf49bca82be180ca517fce3380ead4b99389c9e4bc0',
})
LEDGER_SCHEMA='saturday.local-provider.stage1-ledger.v2'
LEDGER_KEYS={'schema','state','generation','authorization_sha256','binding_sha256','key_id','candidate_id','candidate_sha256','review_aggregate_sha256','review_manifest_sha256','run_id','receipt_sha256','cleanup_sha256','receipt_registration','workspace'}
_CLAIM_SEAL=object()
_COMPLETION_SEAL=object()
_LIFECYCLE_SEAL=object()


class CompletedTransferAuthorization:
 def __init__(self,public:Mapping[str,str],artifacts:List[Tuple[str,str,int]],seal:object):
  if seal is not _COMPLETION_SEAL: raise PermissionError('completed authorization cannot be constructed directly')
  self.public=dict(public); self.artifacts=tuple(artifacts); self._sealed=True


class TransferAuthorization(dict):
 def __init__(self,public:Mapping[str,str],artifacts:List[Tuple[str,str,int]],seal:object):
  if seal is not _CLAIM_SEAL: raise PermissionError('authorization claim cannot be constructed directly')
  super().__init__(public); self._artifacts=list(artifacts); self._allowed={(repository,digest):size for repository,digest,size in artifacts}; self._done=set(); self._inflight=None; self._failed=False; self._completed=False

 def begin(self,repository:str,digest:str,size:int)->None:
  identity=(repository,digest)
  if self._failed or self._inflight is not None or identity in self._done or self._allowed.get(identity)!=size: raise PermissionError('artifact is not authorized')
  self._inflight=identity

 def finish(self,success:bool)->None:
  if self._inflight is None: raise PermissionError('no authorized transfer is active')
  if success: self._done.add(self._inflight)
  else: self._failed=True
  self._inflight=None

 def complete(self)->CompletedTransferAuthorization:
  if self._failed or self._inflight is not None or self._completed or self._done!=set(self._allowed): raise PermissionError('authorized transfer set is incomplete')
  self._completed=True
  return CompletedTransferAuthorization(self,self._artifacts,_COMPLETION_SEAL)


def _validated_transfer_artifacts(authorization:owner_auth.VerifiedOwnerAuthorization,candidate:Mapping[str,Any],review_aggregate_sha256:str)->List[Tuple[str,str,int]]:
 if not isinstance(authorization,owner_auth.VerifiedOwnerAuthorization): raise PermissionError('verified owner authorization is required')
 payload=authorization.payload
 if payload['candidate']['candidate_id']!=candidate.get('candidate_id') or payload['candidate']['candidate_sha256']!=CANDIDATE_SHA256: raise ValueError('authorization candidate mismatch')
 if payload['review']['review_aggregate_sha256']!=review_aggregate_sha256: raise ValueError('authorization review mismatch')
 artifacts=[]
 for index,item in enumerate([candidate.get('runtime',{}),*candidate.get('dependencies',[]) ]):
  repository=item.get('oci_repository',item.get('repository')); digest=item.get('layer_digest',item.get('layer')); size=item.get('layer_bytes')
  if not _valid_repository(repository) or not isinstance(digest,str) or not re.fullmatch(r'[0-9a-f]{64}',digest) or type(size) is not int or size<=0: raise ValueError('authorization artifact mismatch')
  signed=payload['artifacts'][index]
  if signed['repository']!=repository or signed['layer_sha256']!=digest or signed['transfer_bytes']!=size: raise ValueError('authorization artifact mismatch')
  artifacts.append((repository,digest,size))
 if len(artifacts)!=payload['limits']['artifact_count'] or sum(item[2] for item in artifacts)!=payload['limits']['verified_layer_total_bytes'] or sum(item[2] for item in artifacts)>payload['limits']['maximum_transfer_bytes']: raise ValueError('authorization artifact bound mismatch')
 return artifacts


def redacted_stage1_failure(_error:BaseException)->str:
 return 'PREBUILT_STAGE1_FAILED'


def canonical_json_bytes(value:Any)->bytes:
 return json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=True,allow_nan=False).encode('ascii')


def canonical_json_sha256(value:Any)->str:
 return hashlib.sha256(canonical_json_bytes(value)).hexdigest()


def _transfer_authorization_from_verified(authorization:owner_auth.VerifiedOwnerAuthorization,candidate:Mapping[str,Any],review_aggregate_sha256:str)->TransferAuthorization:
 artifacts=_validated_transfer_artifacts(authorization,candidate,review_aggregate_sha256)
 public={
  'authorization_sha256':authorization.envelope_sha256,
  'binding_sha256':authorization.binding_sha256,
  'generation':authorization.generation,
  'key_id':authorization.key_id,
  'nonce_sha256':authorization.nonce_sha256,
  'purpose':authorization.purpose,
 }
 return TransferAuthorization(public,artifacts,_CLAIM_SEAL)


def live_disk_preflight(descriptors:Mapping[str,int],required_bytes:int,*,fstat_fn=os.fstat,fstatvfs_fn=os.fstatvfs)->Dict[str,int]:
 if set(descriptors)!={'artifacts','runtime','evidence'}: raise ValueError('preflight descriptor shape mismatch')
 if type(required_bytes) is not int or required_bytes<=0: raise ValueError('invalid disk requirement')
 stats=[fstat_fn(descriptors[name]) for name in sorted(descriptors)]
 if any(not stat.S_ISDIR(value.st_mode) for value in stats): raise ValueError('preflight descriptor is not a directory')
 if len({value.st_dev for value in stats})!=1: raise ValueError('workspace volume mismatch')
 if len({(value.st_dev,value.st_ino) for value in stats})!=len(stats): raise ValueError('workspace roles are not disjoint')
 spaces=[fstatvfs_fn(fd) for fd in descriptors.values()]
 available=min(value.f_bavail*value.f_frsize for value in spaces)
 if available<required_bytes: raise OSError('insufficient same-volume disk space')
 return {'device':stats[0].st_dev,'available_bytes':available,'required_bytes':required_bytes}

def _pairs(pairs):
 out={}
 for k,v in pairs:
  if k in out: raise ValueError('duplicate JSON key')
  out[k]=v
 return out

def _decode_json(payload:bytes)->Dict[str,Any]:
 try: data=json.loads(payload.decode('utf-8'),object_pairs_hook=_pairs,parse_constant=lambda _:(_ for _ in ()).throw(ValueError('non-finite JSON')))
 except (UnicodeError,json.JSONDecodeError) as error: raise ValueError('invalid JSON') from error
 if not isinstance(data,dict): raise ValueError('JSON root is not an object')
 return data


def _ledger_marker_name(generation:str)->str:
 if re.fullmatch(r'[0-9a-f]{32}',generation or '') is None: raise ValueError('invalid authorization generation')
 return '.saturday-stage1-'+generation+'.json'


def _validate_ledger_directory(descriptor:int)->None:
 info=os.fstat(descriptor)
 if not stat.S_ISDIR(info.st_mode) or stat.S_IMODE(info.st_mode)&0o077 or info.st_uid!=os.getuid(): raise PermissionError('authorization ledger is not private')


@contextlib.contextmanager
def authorization_ledger_lock(ledger_fd:int):
 _validate_ledger_directory(ledger_fd); flags=os.O_RDWR|os.O_CREAT
 if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
 descriptor=os.open('.saturday-stage1.lock',flags,0o600,dir_fd=ledger_fd)
 try:
  info=os.fstat(descriptor)
  if not stat.S_ISREG(info.st_mode) or info.st_nlink!=1 or stat.S_IMODE(info.st_mode)!=0o600 or info.st_uid!=os.getuid(): raise PermissionError('authorization ledger lock is unsafe')
  try: fcntl.flock(descriptor,fcntl.LOCK_EX|fcntl.LOCK_NB)
  except BlockingIOError as error: raise PermissionError('authorization ledger is active') from error
  yield
 finally:
  try: fcntl.flock(descriptor,fcntl.LOCK_UN)
  finally: os.close(descriptor)


def _read_ledger_marker(ledger_fd:int,generation:str)->Dict[str,Any]:
 _validate_ledger_directory(ledger_fd); flags=os.O_RDONLY
 if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
 try: descriptor=os.open(_ledger_marker_name(generation),flags,dir_fd=ledger_fd)
 except OSError as error: raise PermissionError('durable receipt provenance is missing') from error
 try:
  info=os.fstat(descriptor)
  if not stat.S_ISREG(info.st_mode) or info.st_nlink!=1 or stat.S_IMODE(info.st_mode)!=0o600 or info.st_size>65536: raise PermissionError('durable receipt provenance is invalid')
  chunks=[]; remaining=info.st_size
  while remaining:
   chunk=os.read(descriptor,min(remaining,65536))
   if not chunk: raise PermissionError('durable receipt provenance is truncated')
   chunks.append(chunk); remaining-=len(chunk)
  if os.read(descriptor,1): raise PermissionError('durable receipt provenance changed')
 finally: os.close(descriptor)
 marker=_decode_json(b''.join(chunks))
 if set(marker)!=LEDGER_KEYS or marker['schema']!=LEDGER_SCHEMA or marker['generation']!=generation: raise PermissionError('durable receipt provenance is invalid')
 return marker


def _read_ledger_marker_if_present(ledger_fd:int,generation:str)->Optional[Dict[str,Any]]:
 _validate_ledger_directory(ledger_fd)
 try: os.stat(_ledger_marker_name(generation),dir_fd=ledger_fd,follow_symlinks=False)
 except FileNotFoundError: return None
 return _read_ledger_marker(ledger_fd,generation)


def _replace_ledger_marker(ledger_fd:int,generation:str,marker:Mapping[str,Any])->None:
 if set(marker)!=LEDGER_KEYS or marker.get('schema')!=LEDGER_SCHEMA or marker.get('generation')!=generation: raise PermissionError('durable receipt provenance is invalid')
 name=_ledger_marker_name(generation); temporary=name+'.tmp-'+uuid.uuid4().hex; flags=os.O_WRONLY|os.O_CREAT|os.O_EXCL
 if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
 descriptor=os.open(temporary,flags,0o600,dir_fd=ledger_fd)
 try: _write_all(descriptor,canonical_json_bytes(marker)); os.fsync(descriptor)
 except BaseException:
  try: os.unlink(temporary,dir_fd=ledger_fd)
  except OSError: pass
  raise
 finally: os.close(descriptor)
 try: os.replace(temporary,name,src_dir_fd=ledger_fd,dst_dir_fd=ledger_fd); os.fsync(ledger_fd)
 except BaseException:
  try: os.unlink(temporary,dir_fd=ledger_fd)
  except OSError: pass
  raise


def _record_cleaned_receipt(ledger_fd:int,generation:str,receipt:Mapping[str,Any],receipt_sha256:str,receipt_registration:Mapping[str,Any])->None:
 marker=_read_ledger_marker(ledger_fd,generation)
 if marker['state']!='authorized' or any(marker[key] is not None for key in ('run_id','receipt_sha256','cleanup_sha256','receipt_registration')): raise PermissionError('durable receipt provenance state mismatch')
 if marker['candidate_id']!=receipt['candidate_id'] or marker['candidate_sha256']!=receipt['candidate_sha256'] or marker['review_aggregate_sha256']!=receipt['review_aggregate_sha256']: raise PermissionError('durable receipt provenance identity mismatch')
 if not isinstance(receipt_registration,dict) or set(receipt_registration)!={'parent_device','parent_inode','parent_uid','name'}: raise PermissionError('receipt destination provenance is invalid')
 marker.update(state='cleaned',run_id=receipt['run_id'],receipt_sha256=receipt_sha256,cleanup_sha256=canonical_json_sha256(receipt['cleanup']),receipt_registration=dict(receipt_registration))
 _replace_ledger_marker(ledger_fd,generation,marker)


def _verify_receipt_provenance(ledger_fd:int,generation:str,receipt:Mapping[str,Any],receipt_sha256:str,required_state:str)->Dict[str,Any]:
 marker=_read_ledger_marker(ledger_fd,generation)
 expected={
  'state':required_state,'generation':generation,'authorization_sha256':receipt['authorization']['authorization_sha256'],'binding_sha256':receipt['authorization']['binding_sha256'],'key_id':receipt['authorization']['key_id'],'candidate_id':receipt['candidate_id'],
  'candidate_sha256':receipt['candidate_sha256'],'review_aggregate_sha256':receipt['review_aggregate_sha256'],
  'run_id':receipt['run_id'],'receipt_sha256':receipt_sha256,'cleanup_sha256':canonical_json_sha256(receipt['cleanup']),
 }
 if any(marker.get(key)!=value for key,value in expected.items()): raise PermissionError('durable receipt provenance mismatch')
 return marker


def _create_ledger_marker(ledger_fd:int,generation:str,marker:Mapping[str,Any])->None:
 if set(marker)!=LEDGER_KEYS or marker.get('schema')!=LEDGER_SCHEMA or marker.get('generation')!=generation: raise PermissionError('durable authorization marker is invalid')
 flags=os.O_WRONLY|os.O_CREAT|os.O_EXCL
 if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
 try: descriptor=os.open(_ledger_marker_name(generation),flags,0o600,dir_fd=ledger_fd)
 except FileExistsError as error: raise PermissionError('authorization already consumed') from error
 try: _write_all(descriptor,canonical_json_bytes(marker)); os.fsync(descriptor); os.fsync(ledger_fd)
 finally: os.close(descriptor)


def _register_authorization(ledger_fd:int,authorization:owner_auth.VerifiedOwnerAuthorization,workspace:Any,candidate:Mapping[str,Any],review_aggregate_sha256:str)->TransferAuthorization:
 _validate_ledger_directory(ledger_fd)
 payload=authorization.payload; generation=authorization.generation
 parent=os.fstat(workspace.parent_fd); root=os.fstat(workspace.root_fd)
 signed_parent=payload['workspace_parent']
 if (parent.st_dev,parent.st_ino,parent.st_uid)!=(signed_parent['device'],signed_parent['inode'],signed_parent['uid']): raise PermissionError('workspace parent identity mismatch')
 if not stat.S_ISDIR(root.st_mode) or stat.S_IMODE(root.st_mode)!=0o700 or root.st_uid!=os.getuid() or workspace.name!='saturday-prebuilt-'+generation: raise PermissionError('workspace identity mismatch')
 registration={'parent_device':parent.st_dev,'parent_inode':parent.st_ino,'parent_uid':parent.st_uid,'name':workspace.name,'device':root.st_dev,'inode':root.st_ino,'uid':root.st_uid}
 marker={
  'schema':LEDGER_SCHEMA,'state':'authorized','generation':generation,
  'authorization_sha256':authorization.envelope_sha256,'binding_sha256':authorization.binding_sha256,'key_id':authorization.key_id,
  'candidate_id':payload['candidate']['candidate_id'],'candidate_sha256':payload['candidate']['candidate_sha256'],
  'review_aggregate_sha256':payload['review']['review_aggregate_sha256'],'review_manifest_sha256':payload['review']['review_manifest_sha256'],
  'run_id':None,'receipt_sha256':None,'cleanup_sha256':None,'receipt_registration':None,'workspace':registration,
 }
 _create_ledger_marker(ledger_fd,generation,marker)
 owner={'schema':'saturday.local-provider.stage1-workspace.v2','generation':generation,'authorization_sha256':authorization.envelope_sha256,**registration}
 flags=os.O_WRONLY|os.O_CREAT|os.O_EXCL
 if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
 descriptor=os.open('.saturday-stage1-owner.json',flags,0o600,dir_fd=workspace.root_fd)
 try: _write_all(descriptor,canonical_json_bytes(owner)); os.fsync(descriptor); os.fsync(workspace.root_fd)
 finally: os.close(descriptor)
 return _transfer_authorization_from_verified(authorization,candidate,review_aggregate_sha256)


def _remove_recoverable_workspace(parent_fd:int,name:str,expected:Optional[Mapping[str,Any]]=None,owner_expected:Optional[Mapping[str,Any]]=None)->bool:
 try: info=os.stat(name,dir_fd=parent_fd,follow_symlinks=False)
 except FileNotFoundError: return False
 if not re.fullmatch(r'saturday-prebuilt-[0-9a-f]{32}',name) or not stat.S_ISDIR(info.st_mode) or stat.S_IMODE(info.st_mode)!=0o700 or info.st_uid!=os.getuid(): raise PermissionError('unsafe recoverable workspace')
 if expected is not None and (info.st_dev,info.st_ino,info.st_uid)!=(expected['device'],expected['inode'],expected['uid']): raise PermissionError('recoverable workspace identity mismatch')
 flags=os.O_RDONLY|os.O_DIRECTORY
 if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
 root_fd=os.open(name,flags,dir_fd=parent_fd)
 try:
  opened=os.fstat(root_fd)
  if (opened.st_dev,opened.st_ino)!=(info.st_dev,info.st_ino): raise PermissionError('recoverable workspace identity changed')
  if owner_expected is not None:
   owner_flags=os.O_RDONLY
   if hasattr(os,'O_NOFOLLOW'): owner_flags|=os.O_NOFOLLOW
   descriptor=os.open('.saturday-stage1-owner.json',owner_flags,dir_fd=root_fd)
   try:
    owner_info=os.fstat(descriptor)
    if not stat.S_ISREG(owner_info.st_mode) or owner_info.st_nlink!=1 or owner_info.st_uid!=os.getuid() or stat.S_IMODE(owner_info.st_mode)!=0o600 or owner_info.st_size>4096: raise PermissionError('recoverable workspace owner is unsafe')
    payload=os.read(descriptor,4097)
    if len(payload)!=owner_info.st_size or os.read(descriptor,1): raise PermissionError('recoverable workspace owner changed')
   finally: os.close(descriptor)
   owner=_decode_json(payload)
   if payload!=canonical_json_bytes(owner) or owner!=dict(owner_expected): raise PermissionError('recoverable workspace owner mismatch')
  _clear_directory(root_fd)
  if os.listdir(root_fd): raise OSError('recovered workspace cleanup incomplete')
 finally: os.close(root_fd)
 current=os.stat(name,dir_fd=parent_fd,follow_symlinks=False)
 if (current.st_dev,current.st_ino)!=(info.st_dev,info.st_ino): raise PermissionError('recoverable workspace identity changed')
 os.rmdir(name,dir_fd=parent_fd); os.fsync(parent_fd)
 return True


def _clear_workspace_registration(ledger_fd:int,generation:str,state:str)->None:
 marker=_read_ledger_marker(ledger_fd,generation)
 if marker['state']!='authorized' or marker['workspace'] is None: raise PermissionError('workspace provenance state mismatch')
 marker['workspace']=None; marker['state']=state; _replace_ledger_marker(ledger_fd,generation,marker)


def recover_stage1_workspaces(ledger_fd:int,workspace_parent_fd:int)->int:
 _validate_ledger_directory(ledger_fd); parent=os.fstat(workspace_parent_fd); recovered=0
 if not stat.S_ISDIR(parent.st_mode) or stat.S_IMODE(parent.st_mode)!=0o700 or parent.st_uid!=os.getuid(): raise PermissionError('workspace parent is not private')
 for name in sorted(os.listdir(ledger_fd)):
  match=re.fullmatch(r'\.saturday-stage1-([0-9a-f]{32})\.json',name)
  if match is None: continue
  generation=match.group(1); marker=_read_ledger_marker(ledger_fd,generation); workspace=marker['workspace']
  if workspace is None: continue
  required={'parent_device','parent_inode','parent_uid','name','device','inode','uid'}
  if not isinstance(workspace,dict) or set(workspace)!=required or any(type(workspace[key]) is not int or workspace[key]<0 for key in required-{'name'}) or not isinstance(workspace['name'],str): raise PermissionError('workspace provenance is invalid')
  if (parent.st_dev,parent.st_ino,parent.st_uid)!=(workspace['parent_device'],workspace['parent_inode'],workspace['parent_uid']): raise PermissionError('workspace parent provenance mismatch')
  owner={'schema':'saturday.local-provider.stage1-workspace.v2','generation':generation,'authorization_sha256':marker['authorization_sha256'],**workspace}
  recovered+=int(_remove_recoverable_workspace(workspace_parent_fd,workspace['name'],workspace,owner))
  marker['workspace']=None
  if marker['state']=='authorized': marker['state']='recovered'
  _replace_ledger_marker(ledger_fd,generation,marker)
 return recovered


def _read_path(path:Path,maximum_bytes:int=16777216)->bytes:
 path=Path(path); before=path.lstat(); flags=os.O_RDONLY
 if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
 try: descriptor=os.open(path,flags)
 except OSError as error: raise ValueError('unsafe JSON file') from error
 try:
  current=os.fstat(descriptor)
  if not stat.S_ISREG(current.st_mode) or current.st_nlink!=1 or (current.st_dev,current.st_ino)!=(before.st_dev,before.st_ino) or current.st_size>maximum_bytes: raise ValueError('unsafe JSON file')
  chunks=[]; remaining=current.st_size
  while remaining:
   chunk=os.read(descriptor,min(131072,remaining))
   if not chunk: raise ValueError('truncated JSON file')
   chunks.append(chunk); remaining-=len(chunk)
  if os.read(descriptor,1): raise ValueError('growing JSON file')
  return b''.join(chunks)
 finally: os.close(descriptor)


def _load(path:Path)->Dict[str,Any]:
 return _decode_json(_read_path(path))


def load_candidate(path:Path)->Dict[str,Any]:
 payload=_read_path(path)
 if hashlib.sha256(payload).hexdigest()!=CANDIDATE_SHA256: raise ValueError('candidate identity mismatch')
 d=_decode_json(payload)
 if set(d)!={'schema','candidate_id','decision','frozen_on','authorizations','model','runtime','dependencies','provenance','two_stage_gate','security'}: raise ValueError('candidate shape mismatch')
 if d['schema']!='saturday.local-provider.prebuilt-candidate.v2': raise ValueError('candidate schema mismatch')
 bottle=d['authorizations']['bottle_transfer']; model=d['authorizations']['model_transfer']
 if bottle['state']!='not_granted' or model['state']!='blocked_until_relocated_runtime_review': raise ValueError('candidate is not pre-approval')
 source=[d['runtime'],*d['dependencies']]
 if len(source)!=5 or bottle.get('artifact_count')!=len(source) or type(bottle.get('maximum_bytes')) is not int or bottle['maximum_bytes']<=0: raise ValueError('runtime artifact bound mismatch')
 layer_total=0
 for item in source:
  repository=item.get('oci_repository',item.get('repository')); digest=item.get('layer_digest',item.get('layer')); size=item.get('layer_bytes')
  if not _valid_repository(repository) or not isinstance(digest,str) or re.fullmatch(r'[0-9a-f]{64}',digest) is None or type(size) is not int or size<=0: raise ValueError('runtime artifact identity mismatch')
  installed=item.get('installed_bytes'); maximum=item.get('maximum_extracted_bytes')
  if (installed is None)==(maximum is None) or installed is not None and (type(installed) is not int or installed<=0) or maximum is not None and (type(maximum) is not int or maximum<=0): raise ValueError('layer extraction identity mismatch')
  layer_total+=size
 if layer_total!=d['provenance'].get('verified_layer_total_bytes') or bottle['maximum_bytes']!=d['provenance'].get('bottle_transfer_cap_bytes') or layer_total>bottle['maximum_bytes']: raise ValueError('runtime bound mismatch')
 if model.get('artifact_sha256')!=d['model'].get('sha256') or model.get('artifact_bytes')!=d['model'].get('bytes') or not re.fullmatch(r'[0-9a-f]{64}',str(d['model'].get('sha256'))) or type(d['model'].get('bytes')) is not int or d['model']['bytes']<=0: raise ValueError('model bound mismatch')
 if type(d['provenance'].get('maximum_total_extracted_bytes')) is not int or d['provenance']['maximum_total_extracted_bytes']<=0 or type(d['provenance'].get('maximum_entries')) is not int or d['provenance']['maximum_entries']<=0 or type(d['provenance'].get('minimum_free_bytes_before_each_transfer')) is not int or d['provenance']['minimum_free_bytes_before_each_transfer']<=0: raise ValueError('runtime resource bound mismatch')
 security=d['security']
 if any(security.get(key) is not True for key in ('no_local_source_build','no_child_downloader','no_formula_execution','network_denied_and_fork_denied_at_inference')) or security.get('product_activation') is not False: raise ValueError('candidate security policy mismatch')
 return d

def verify_review_manifest(path:Path,root:Path,expected:str)->str:
 d=_load(path)
 if set(d)!={'schema','review_aggregate_sha256','files'} or d['schema']!=REVIEW_SCHEMA: raise ValueError('review shape mismatch')
 root_info=root.lstat()
 if not stat.S_ISDIR(root_info.st_mode) or root.is_symlink(): raise ValueError('unsafe review root')
 flags=os.O_RDONLY|os.O_DIRECTORY
 if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
 root_fd=os.open(root,flags); records=[]; seen=set()
 opened_root=os.fstat(root_fd)
 if (opened_root.st_dev,opened_root.st_ino)!=(root_info.st_dev,root_info.st_ino):
  os.close(root_fd); raise ValueError('review root identity changed')
 try:
  for item in d['files']:
   if not isinstance(item,dict) or set(item)!={'path','bytes','mode','sha256'} or _archive_path(item['path'])!=item['path'] or item['path'] in seen: raise ValueError('review record shape mismatch')
   seen.add(item['path']); parent,name=_parent_directory(root_fd,item['path'],False); descriptor=None
   try:
    file_flags=os.O_RDONLY
    if hasattr(os,'O_NOFOLLOW'): file_flags|=os.O_NOFOLLOW
    descriptor=os.open(name,file_flags,dir_fd=parent); info=os.fstat(descriptor)
    if not stat.S_ISREG(info.st_mode) or info.st_nlink!=1: raise ValueError('unsafe review file')
    digest=hashlib.sha256(); total=0
    while True:
     chunk=os.read(descriptor,131072)
     if not chunk: break
     total+=len(chunk); digest.update(chunk)
    rec={'path':item['path'],'bytes':total,'mode':stat.S_IMODE(info.st_mode),'sha256':digest.hexdigest()}
    if rec!=item: raise ValueError('review file mismatch')
    records.append(rec)
   except OSError as error: raise ValueError('unsafe review file') from error
   finally:
    if descriptor is not None: os.close(descriptor)
    os.close(parent)
 finally: os.close(root_fd)
 if records!=sorted(records,key=lambda x:x['path']): raise ValueError('review order mismatch')
 aggregate=hashlib.sha256(json.dumps(records,sort_keys=True,separators=(',',':')).encode()).hexdigest()
 if aggregate!=expected or d['review_aggregate_sha256']!=aggregate: raise ValueError('review aggregate mismatch')
 return aggregate

def verify_stage1_review_manifest(path:Path,sidecar_path:Path,root:Path,expected_aggregate:str,expected_manifest_sha256:str)->Dict[str,str]:
 if expected_aggregate in REVOKED_REVIEW_AGGREGATES: raise ValueError('review aggregate is revoked')
 trusted_harness=Path(__file__).resolve(); trusted_root=trusted_harness.parents[1]
 try:
  supplied_root=Path(root).lstat(); expected_root=trusted_root.lstat(); supplied_harness=(Path(root)/'scripts/prebuilt_runtime_stage1.py').lstat(); executing_harness=trusted_harness.lstat()
 except OSError as error: raise ValueError('stage1 repository root mismatch') from error
 if (supplied_root.st_dev,supplied_root.st_ino)!=(expected_root.st_dev,expected_root.st_ino) or (supplied_harness.st_dev,supplied_harness.st_ino)!=(executing_harness.st_dev,executing_harness.st_ino): raise ValueError('stage1 repository root mismatch')
 if re.fullmatch(r'[0-9a-f]{64}',expected_manifest_sha256 or '') is None: raise ValueError('review manifest digest mismatch')
 manifest_bytes=_read_path(path); digest=hashlib.sha256(manifest_bytes).hexdigest()
 if not hmac.compare_digest(digest,expected_manifest_sha256): raise ValueError('review manifest digest mismatch')
 document=_decode_json(manifest_bytes); files=document.get('files')
 if not isinstance(files,list) or len(files)!=len(STAGE1_REVIEW_ROSTER) or {item.get('path') for item in files if isinstance(item,dict)}!=STAGE1_REVIEW_ROSTER: raise ValueError('stage1 review roster mismatch')
 try: sidecar=_read_path(sidecar_path).decode('ascii')
 except UnicodeDecodeError as error: raise ValueError('review manifest sidecar mismatch') from error
 expected_sidecar=f'{digest}  {path.name}\n'
 if sidecar!=expected_sidecar: raise ValueError('review manifest sidecar mismatch')
 aggregate=verify_review_manifest(path,root,expected_aggregate)
 return {'review_aggregate_sha256':aggregate,'review_manifest_sha256':digest}

def _origin(value:str)->Tuple[str,str,int]:
 try: parsed=urlparse(value); port=parsed.port or 443
 except ValueError as error: raise ValueError('invalid HTTPS origin') from error
 if parsed.scheme!='https' or not parsed.hostname or parsed.username is not None or parsed.password is not None or parsed.fragment or port!=443: raise ValueError('invalid HTTPS origin')
 return ('https',parsed.hostname.lower(),port)

def _origin_text(value:str)->str:
 scheme,host,port=_origin(value)
 return '%s://%s:%d'%(scheme,host,port)

def parse_bearer_challenge(value:str,expected_repository:Optional[str]=None):
 m=re.fullmatch(r'Bearer realm="([^"]+)",service="([^"]+)",scope="([^"]+)"',value)
 if not m: raise ValueError('invalid OCI challenge')
 realm,service,scope=m.groups(); parsed=urlparse(realm)
 if _origin_text(realm)!='https://ghcr.io:443' or parsed.path!='/token' or parsed.params or parsed.query: raise ValueError('invalid OCI challenge')
 if service!='ghcr.io' or not re.fullmatch(r'repository:homebrew/core/[a-z0-9][a-z0-9._@/-]*:pull',scope): raise ValueError('invalid OCI challenge')
 repository=scope[len('repository:'):-len(':pull')]
 if not _valid_repository(repository): raise ValueError('invalid OCI challenge')
 if expected_repository is not None and scope!='repository:'+expected_repository+':pull': raise ValueError('OCI scope mismatch')
 return m.groups()

def validate_blob_redirect(current:str,location:str,count:int):
 source=urlparse(current); target=urlparse(location)
 match=re.fullmatch(r'/v2/(homebrew/core/[a-z0-9][a-z0-9._@/-]*)/blobs/sha256:[0-9a-f]{64}',source.path)
 if count!=0 or _origin_text(current)!='https://ghcr.io:443' or match is None or not _valid_repository(match.group(1)) or source.params or source.query or _origin_text(location)!='https://pkg-containers.githubusercontent.com:443' or not target.path.startswith('/') or target.params: raise ValueError('invalid blob redirect')
 return location,False

def _single_header(response,name:str,required:bool=True)->Optional[str]:
 values=[str(value) for key,value in response.getheaders() if key.lower()==name.lower()]
 if len(values)>1 or (required and not values): raise ValueError('invalid '+name+' header')
 if not values: return None
 value=values[0]
 if value!=value.strip() or '\r' in value or '\n' in value: raise ValueError('invalid '+name+' header')
 return value

def _content_length(response,maximum:int,expected:Optional[int]=None)->int:
 value=_single_header(response,'Content-Length')
 if value is None or not re.fullmatch(r'0|[1-9][0-9]*',value): raise ValueError('invalid Content-Length')
 length=int(value)
 if length>maximum or (expected is not None and length!=expected): raise ValueError('invalid Content-Length')
 return length

def _read_response(response,length:int)->bytes:
 chunks=[]; total=0
 while total<length:
  chunk=response.read(min(131072,length-total))
  if not isinstance(chunk,(bytes,bytearray)) or not chunk: raise ValueError('truncated HTTP response')
  total+=len(chunk); chunks.append(bytes(chunk))
 if response.read(1): raise ValueError('HTTP response overflow')
 return b''.join(chunks)

def _valid_repository(repository:Any)->bool:
 if not isinstance(repository,str): return False
 parts=repository.split('/')
 return len(parts)>=3 and parts[:2]==['homebrew','core'] and all(part not in ('','.','..') and re.fullmatch(r'[a-z0-9][a-z0-9._@+-]*',part) for part in parts[2:])

def _bearer(token:Any)->str:
 if not isinstance(token,str) or not re.fullmatch(r'[\x21-\x7e]{1,2048}',token): raise ValueError('invalid bearer token')
 return 'Bearer '+token

def _raise_nonfinite():
 raise ValueError('non-finite JSON')

class OCITransport:
 """Bounded GHCR transport. Tests inject connections; no redirect is automatic."""
 def __init__(self,connection_factory=None,authorization:Optional[TransferAuthorization]=None):
  self.connection_factory=connection_factory or self._connection; self.authorization=authorization

 @staticmethod
 def _connection(origin:str):
  parsed=urlparse(origin)
  return http.client.HTTPSConnection(parsed.hostname,parsed.port,timeout=30)

 def _request(self,url:str,headers:Mapping[str,str]):
  origin=_origin_text(url)
  if origin not in ('https://ghcr.io:443','https://pkg-containers.githubusercontent.com:443'): raise ValueError('unapproved transport origin')
  parsed=urlparse(url); target=parsed.path+('?' + parsed.query if parsed.query else '')
  connection=self.connection_factory(origin)
  try:
   connection.request('GET',target,headers=dict(headers))
   return connection,connection.getresponse()
  except BaseException:
   connection.close(); raise

 def fetch_bearer_challenge(self,repository:str,kind:str,digest:str)->str:
  url=self._registry_url(repository,kind,digest)
  accept='application/octet-stream' if kind=='blobs' else 'application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json'
  connection,response=self._request(url,{'Accept':accept})
  try:
   if response.status!=401 or _single_header(response,'Location',required=False) is not None: raise ValueError('OCI challenge request failed')
   challenge=_single_header(response,'WWW-Authenticate')
   if challenge is None: raise ValueError('OCI challenge missing')
   _read_response(response,_content_length(response,4096))
   parse_bearer_challenge(challenge,repository)
   return challenge
  finally:
   response.close(); connection.close()

 def fetch_token(self,challenge:str,repository:str)->str:
  if not _valid_repository(repository): raise ValueError('invalid OCI repository')
  realm,service,scope=parse_bearer_challenge(challenge,repository)
  connection,response=self._request(realm+'?'+urlencode((('service',service),('scope',scope))),{'Accept':'application/json'})
  try:
   if response.status!=200: raise ValueError('token request failed')
   body=_read_response(response,_content_length(response,4096))
   value=json.loads(body.decode('utf-8'),object_pairs_hook=_pairs,parse_constant=lambda _:_raise_nonfinite())
   if not isinstance(value,dict) or set(value)!={'token'} or not isinstance(value['token'],str) or not re.fullmatch(r'[\x21-\x7e]{1,2048}',value['token']): raise ValueError('invalid token response')
   return value['token']
  except (UnicodeError,json.JSONDecodeError) as error: raise ValueError('invalid token response') from error
  finally:
   response.close(); connection.close()

 def fetch_json(self,repository:str,kind:str,digest:str,token:str,maximum_bytes:int)->Dict[str,Any]:
  url=self._registry_url(repository,kind,digest)
  connection,response=self._request(url,{'Accept':'application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json, application/vnd.oci.image.config.v1+json','Authorization':_bearer(token)})
  try:
   if response.status!=200: raise ValueError('OCI metadata request failed')
   media_type=_single_header(response,'Content-Type')
   if media_type not in {'application/vnd.oci.image.index.v1+json','application/vnd.oci.image.manifest.v1+json','application/vnd.oci.image.config.v1+json'}: raise ValueError('OCI metadata media type mismatch')
   if _single_header(response,'Docker-Content-Digest')!='sha256:'+digest: raise ValueError('OCI metadata header mismatch')
   body=_read_response(response,_content_length(response,maximum_bytes))
   if hashlib.sha256(body).hexdigest()!=digest: raise ValueError('OCI metadata identity mismatch')
   value=json.loads(body.decode('utf-8'),object_pairs_hook=_pairs,parse_constant=lambda _:_raise_nonfinite())
   if not isinstance(value,dict): raise ValueError('OCI metadata root mismatch')
   return value
  except (UnicodeError,json.JSONDecodeError) as error: raise ValueError('invalid OCI metadata') from error
  finally:
   response.close(); connection.close()

 def _download_blob(self,repository:str,digest:str,token:str,expected_bytes:int,maximum_bytes:int,writer,preflight=None)->None:
  if not isinstance(self.authorization,TransferAuthorization): raise PermissionError('bottle transfer is not authorized')
  self.authorization.begin(repository,digest,expected_bytes); success=False
  try:
   url=self._registry_url(repository,'blobs',digest)
   connection,response=self._request(url,{'Accept':'application/octet-stream','Authorization':_bearer(token)})
   try:
    if response.status==307:
     location=_single_header(response,'Location')
     if location is None: raise ValueError('redirect location missing')
     validate_blob_redirect(url,location,0)
    elif response.status==200: location=None
    else: raise ValueError('OCI blob request failed')
    if location is None:
     length=_content_length(response,maximum_bytes,expected_bytes)
     if preflight is not None: preflight()
     writer(response,length); success=True; return
   finally:
    response.close(); connection.close()
   redirected,redirect_response=self._request(location,{'Accept':'application/octet-stream'})
   try:
    if redirect_response.status!=200: raise ValueError('redirected blob request failed')
    length=_content_length(redirect_response,maximum_bytes,expected_bytes)
    if preflight is not None: preflight()
    writer(redirect_response,length); success=True
   finally:
    redirect_response.close(); redirected.close()
  finally: self.authorization.finish(success)

 def download_blob(self,repository:str,digest:str,token:str,destination:Path,expected_bytes:int,maximum_bytes:int)->None:
  self._download_blob(repository,digest,token,expected_bytes,maximum_bytes,lambda reader,length:copy_verified(reader,destination,length,digest,maximum_bytes))

 def download_blob_at(self,repository:str,digest:str,token:str,directory_fd:int,name:str,expected_bytes:int,maximum_bytes:int,*,preflight_descriptors:Mapping[str,int],required_bytes:int)->None:
  directory=os.fstat(directory_fd)
  if not stat.S_ISDIR(directory.st_mode): raise ValueError('blob destination is not a directory')
  self._download_blob(repository,digest,token,expected_bytes,maximum_bytes,lambda reader,length:copy_verified_at(reader,directory_fd,name,length,digest,maximum_bytes),lambda:live_disk_preflight(preflight_descriptors,required_bytes))

 @staticmethod
 def _registry_url(repository:str,kind:str,digest:str)->str:
  if not _valid_repository(repository) or kind not in ('manifests','blobs') or not re.fullmatch(r'[0-9a-f]{64}',digest): raise ValueError('invalid OCI identity')
  return 'https://ghcr.io:443/v2/'+repository+'/'+kind+'/sha256:'+digest


def select_oci_manifest(index:Mapping[str,Any],annotation_ref:str,expected_digest:str)->Dict[str,Any]:
 if not isinstance(index,dict) or index.get('schemaVersion')!=2 or index.get('mediaType')!='application/vnd.oci.image.index.v1+json' or not isinstance(index.get('manifests'),list): raise ValueError('invalid OCI index')
 expected='sha256:'+expected_digest; matches=[]
 for descriptor in index['manifests']:
  if not isinstance(descriptor,dict): raise ValueError('invalid OCI descriptor')
  annotations=descriptor.get('annotations'); platform=descriptor.get('platform')
  if not isinstance(annotations,dict) or not isinstance(platform,dict): raise ValueError('invalid OCI descriptor')
  if annotations.get('org.opencontainers.image.ref.name')==annotation_ref:
   if descriptor.get('mediaType')!='application/vnd.oci.image.manifest.v1+json' or descriptor.get('digest')!=expected or type(descriptor.get('size')) is not int or descriptor['size']<=0 or platform.get('architecture')!='arm64' or platform.get('os')!='darwin': raise ValueError('OCI target descriptor mismatch')
   matches.append(descriptor)
 if len(matches)!=1: raise ValueError('OCI target descriptor is not unique')
 return matches[0]


def verify_oci_manifest(manifest:Mapping[str,Any],expected_config_sha256:str,expected_layer_sha256:str,expected_layer_bytes:int)->Dict[str,Any]:
 if not isinstance(manifest,dict) or manifest.get('schemaVersion')!=2 or manifest.get('mediaType')!='application/vnd.oci.image.manifest.v1+json': raise ValueError('invalid OCI manifest')
 config=manifest.get('config'); layers=manifest.get('layers')
 if not isinstance(config,dict) or config.get('mediaType')!='application/vnd.oci.image.config.v1+json' or config.get('digest')!='sha256:'+expected_config_sha256 or type(config.get('size')) is not int or config['size']<=0: raise ValueError('OCI config mismatch')
 if not isinstance(layers,list) or len(layers)!=1 or not isinstance(layers[0],dict): raise ValueError('OCI layer count mismatch')
 layer=layers[0]
 if layer.get('mediaType') not in ('application/vnd.oci.image.layer.v1.tar+gzip','application/vnd.oci.image.layer.v1.tar') or layer.get('digest')!='sha256:'+expected_layer_sha256 or layer.get('size')!=expected_layer_bytes or type(expected_layer_bytes) is not int or expected_layer_bytes<=0: raise ValueError('OCI layer mismatch')
 return layer

def copy_verified_at(reader,directory_fd:int,name:str,content_length,expected_sha256:str,maximum_bytes:int):
 if type(content_length) is not int or content_length<0 or content_length>maximum_bytes or not re.fullmatch(r'[0-9a-f]{64}',expected_sha256): raise ValueError('invalid transfer bounds')
 if not isinstance(name,str) or name in ('','.','..') or '/' in name or '\\' in name: raise ValueError('invalid destination name')
 try: os.stat(name,dir_fd=directory_fd,follow_symlinks=False)
 except FileNotFoundError: pass
 else: raise ValueError('dirty destination')
 temporary='.part-'+uuid.uuid4().hex; flags=os.O_WRONLY|os.O_CREAT|os.O_EXCL
 if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
 descriptor=os.open(temporary,flags,0o600,dir_fd=directory_fd); digest=hashlib.sha256(); total=0
 try:
  with os.fdopen(descriptor,'wb',closefd=True) as output:
   while True:
    chunk=reader.read(131072)
    if not chunk: break
    if not isinstance(chunk,(bytes,bytearray)): raise ValueError('non-byte transfer data')
    total+=len(chunk)
    if total>content_length or total>maximum_bytes: raise ValueError('transfer overflow')
    digest.update(chunk); output.write(chunk)
   output.flush(); os.fsync(output.fileno())
  if total!=content_length or digest.hexdigest()!=expected_sha256: raise ValueError('transfer identity mismatch')
  try: os.link(temporary,name,src_dir_fd=directory_fd,dst_dir_fd=directory_fd,follow_symlinks=False)
  except FileExistsError as error: raise ValueError('dirty destination') from error
  os.unlink(temporary,dir_fd=directory_fd); os.fsync(directory_fd)
 except BaseException:
  try: os.unlink(temporary,dir_fd=directory_fd)
  except FileNotFoundError: pass
  raise


def copy_verified(reader,destination:Path,content_length,expected_sha256:str,maximum_bytes:int):
 destination=Path(destination); flags=os.O_RDONLY|os.O_DIRECTORY
 if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
 try: directory_fd=os.open(destination.parent,flags)
 except OSError as error: raise ValueError('unsafe destination parent') from error
 try: copy_verified_at(reader,directory_fd,destination.name,content_length,expected_sha256,maximum_bytes)
 finally: os.close(directory_fd)

def _archive_path(raw:Any)->str:
 if not isinstance(raw,str) or not raw or raw.startswith('/') or '\\' in raw or '\0' in raw: raise ValueError('archive path escape')
 normalized=unicodedata.normalize('NFC',raw.rstrip('/'))
 parts=normalized.split('/')
 if not normalized or any(part in ('','.','..') or len(part.encode('utf-8'))>255 for part in parts): raise ValueError('archive path escape')
 return '/'.join(parts)

def _link_target(path:str,raw:Any,root_relative:bool)->str:
 if not isinstance(raw,str) or not raw or raw.startswith('/') or '\\' in raw or '\0' in raw: raise ValueError('link escape')
 joined=raw if root_relative else posixpath.join(posixpath.dirname(path),raw)
 target=posixpath.normpath(unicodedata.normalize('NFC',joined))
 if target in ('','.','..') or target.startswith('../'): raise ValueError('link escape')
 _archive_path(target)
 return target

def validate_tar_members(members,maximum_entries:int,maximum_bytes:int)->List[Dict[str,Any]]:
 if type(maximum_entries) is not int or type(maximum_bytes) is not int or maximum_entries<0 or maximum_bytes<0 or len(members)>maximum_entries: raise ValueError('archive entry limit')
 seen=set(); aliases={}; roster=[]; total=0
 for item in members:
  raw=item.name
  if item.pax_headers or getattr(item,'sparse',None) is not None: raise ValueError('unsafe archive metadata')
  normalized=_archive_path(raw); parts=normalized.split('/'); alias=unicodedata.normalize('NFD',normalized).casefold()
  for index in range(1,len(parts)+1):
   prefix='/'.join(parts[:index]); prefix_alias=unicodedata.normalize('NFD',prefix).casefold(); prior=aliases.get(prefix_alias)
   if prior is not None and prior!=prefix: raise ValueError('archive path collision')
   aliases[prefix_alias]=prefix
  if alias in seen: raise ValueError('archive path collision')
  seen.add(alias)
  if item.isdir(): kind='directory'; size=0
  elif item.isreg(): kind='file'; size=item.size
  elif item.issym(): kind='symlink'; size=0
  elif item.islnk(): kind='hardlink'; size=0
  else: raise ValueError('special archive member')
  if type(size) is not int or size<0 or item.mode & 0o7000 or (kind in ('file','directory') and item.mode & 0o022): raise ValueError('unsafe archive mode')
  if kind=='directory' and item.mode & 0o700!=0o700: raise ValueError('unsafe directory mode')
  if kind=='file' and not item.mode & 0o400: raise ValueError('unreadable archive file')
  total+=size
  if total>maximum_bytes: raise ValueError('archive expansion limit')
  record={'path':normalized,'kind':kind,'bytes':size,'mode':item.mode & 0o777}
  if kind in ('symlink','hardlink'):
   record['link_target']=item.linkname
   record['_target']=_link_target(normalized,item.linkname,kind=='hardlink')
  roster.append(record)
 by_path={record['path']:record for record in roster}
 for record in roster:
  parts=record['path'].split('/')
  for index in range(1,len(parts)):
   ancestor=by_path.get('/'.join(parts[:index]))
   if ancestor and ancestor['kind']!='directory': raise ValueError('non-directory archive ancestor')
 def resolve(path:str,active:set)->Dict[str,Any]:
  if path in active: raise ValueError('archive link cycle')
  value=by_path.get(path)
  if value is None: raise ValueError('archive link target missing')
  if value['kind'] in ('symlink','hardlink'):
   return resolve(value['_target'],active|{path})
  return value
 for record in roster:
  if record['kind']=='symlink': resolve(record['_target'],{record['path']})
  if record['kind']=='hardlink' and resolve(record['_target'],{record['path']})['kind']!='file': raise ValueError('hard-link target is not a file')
 for record in roster: record.pop('_target',None)
 return sorted(roster,key=lambda value:value['path'])


def _bounded_tar_members(archive:tarfile.TarFile,maximum_entries:int,maximum_bytes:int)->List[tarfile.TarInfo]:
 if type(maximum_entries) is not int or type(maximum_bytes) is not int or maximum_entries<0 or maximum_bytes<0: raise ValueError('archive limits are invalid')
 members=[]; expanded=0
 while True:
  item=archive.next()
  if item is None: break
  if len(members)>=maximum_entries: raise ValueError('archive entry limit')
  if item.pax_headers or getattr(item,'sparse',None) is not None: raise ValueError('unsafe archive metadata')
  if not (item.isdir() or item.isreg() or item.issym() or item.islnk()): raise ValueError('special archive member')
  if item.isreg():
   if type(item.size) is not int or item.size<0 or item.size>maximum_bytes-expanded: raise ValueError('archive expansion limit')
   expanded+=item.size
  members.append(item)
 return members


def _same_tar_member(left:tarfile.TarInfo,right:tarfile.TarInfo)->bool:
 return (left.name,left.size,left.type,left.linkname,left.mode,left.pax_headers,getattr(left,'sparse',None))==(right.name,right.size,right.type,right.linkname,right.mode,right.pax_headers,getattr(right,'sparse',None))


def _open_directory(root_fd:int,parts:List[str],create:bool)->int:
 descriptor=os.dup(root_fd); root_device=os.fstat(root_fd).st_dev
 try:
  for part in parts:
   if create:
    try: os.mkdir(part,0o700,dir_fd=descriptor)
    except FileExistsError: pass
   flags=os.O_RDONLY|os.O_DIRECTORY
   if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
   child=os.open(part,flags,dir_fd=descriptor)
   if os.fstat(child).st_dev!=root_device:
    os.close(child); raise ValueError('archive crossed filesystem')
   os.close(descriptor); descriptor=child
  return descriptor
 except BaseException:
  os.close(descriptor); raise

def _parent_directory(root_fd:int,path:str,create:bool=True)->Tuple[int,str]:
 parts=path.split('/')
 return _open_directory(root_fd,parts[:-1],create),parts[-1]

def _write_all(descriptor:int,data:bytes)->None:
 view=memoryview(data)
 while view:
  written=os.write(descriptor,view)
  if written<=0: raise OSError('descriptor write made no progress')
  view=view[written:]


def _clear_directory(directory_fd:int,counters:Optional[Dict[str,int]]=None)->Dict[str,int]:
 counts=counters if counters is not None else {'files':0,'directories':0,'links':0}
 for name in os.listdir(directory_fd):
  info=os.stat(name,dir_fd=directory_fd,follow_symlinks=False)
  if stat.S_ISDIR(info.st_mode):
   child=_open_directory(directory_fd,[name],False)
   try: _clear_directory(child,counts)
   finally: os.close(child)
   current=os.stat(name,dir_fd=directory_fd,follow_symlinks=False)
   if (current.st_dev,current.st_ino)!=(info.st_dev,info.st_ino): raise OSError('cleanup directory identity changed')
   os.rmdir(name,dir_fd=directory_fd); counts['directories']+=1
  else:
   os.unlink(name,dir_fd=directory_fd)
   counts['links' if stat.S_ISLNK(info.st_mode) else 'files']+=1
 return counts

class PrivateWorkspace:
 """Descriptor-owned deterministic generation root with disjoint stage-1 roles."""
 def __init__(self,parent_fd:int,name:str,root_fd:int,role_fds:Dict[str,int],role_identities:Dict[str,Tuple[int,int]],identity:Tuple[int,int]):
  self.parent_fd=parent_fd; self.name=name; self.root_fd=root_fd; self.role_fds=role_fds; self.role_identities=role_identities; self.identity=identity; self.path=None; self._cleanup=None; self._cleanup_error=None

 @classmethod
 def create(cls,parent_descriptor:int,generation:str,required_bytes:int):
  if re.fullmatch(r'[0-9a-f]{32}',generation or '') is None: raise ValueError('invalid workspace generation')
  flags=os.O_RDONLY|os.O_DIRECTORY
  if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
  parent_fd=os.dup(parent_descriptor)
  opened_parent=os.fstat(parent_fd)
  if not stat.S_ISDIR(opened_parent.st_mode) or stat.S_IMODE(opened_parent.st_mode)!=0o700 or opened_parent.st_uid!=os.getuid(): os.close(parent_fd); raise PermissionError('workspace parent is not private')
  name='saturday-prebuilt-'+generation; root_fd=None; role_fds={}; role_identities={}; created=False
  try:
   os.mkdir(name,0o700,dir_fd=parent_fd); created=True; root_fd=os.open(name,flags,dir_fd=parent_fd)
   identity_info=os.fstat(root_fd); identity=(identity_info.st_dev,identity_info.st_ino)
   for role in ('artifacts','runtime','evidence'):
    os.mkdir(role,0o700,dir_fd=root_fd); role_fds[role]=os.open(role,flags,dir_fd=root_fd)
    role_info=os.fstat(role_fds[role]); role_identities[role]=(role_info.st_dev,role_info.st_ino)
   live_disk_preflight(role_fds,required_bytes)
   return cls(parent_fd,name,root_fd,role_fds,role_identities,identity)
  except BaseException:
   for descriptor in role_fds.values(): os.close(descriptor)
   if root_fd is not None:
    try: _clear_directory(root_fd)
    finally: os.close(root_fd)
   if created:
    try: os.rmdir(name,dir_fd=parent_fd)
    except FileNotFoundError: pass
   os.close(parent_fd); raise

 def fd(self,role:str)->int:
  if self._cleanup is not None or role not in self.role_fds: raise ValueError('workspace is closed')
  return self.role_fds[role]

 def cleanup(self)->Dict[str,Any]:
  if self._cleanup is not None: return dict(self._cleanup)
  if self._cleanup_error is not None: raise OSError(self._cleanup_error)
  counts={'files':0,'directories':0,'links':0}
  role_identity_changed=False
  for role,descriptor in self.role_fds.items():
   info=os.fstat(descriptor)
   if (info.st_dev,info.st_ino)!=self.role_identities[role]: raise OSError('workspace role descriptor identity changed')
   _clear_directory(descriptor,counts)
   if os.listdir(descriptor): raise OSError('workspace role cleanup incomplete')
   try: current_role=os.stat(role,dir_fd=self.root_fd,follow_symlinks=False)
   except FileNotFoundError: role_identity_changed=True
   else:
    if (current_role.st_dev,current_role.st_ino)!=self.role_identities[role]: role_identity_changed=True
  current=os.stat(self.name,dir_fd=self.parent_fd,follow_symlinks=False)
  if (current.st_dev,current.st_ino)!=self.identity: raise OSError('workspace identity changed')
  _clear_directory(self.root_fd,counts)
  if os.listdir(self.root_fd): raise OSError('workspace cleanup incomplete')
  for descriptor in self.role_fds.values(): os.close(descriptor)
  self.role_fds={}
  os.close(self.root_fd); self.root_fd=-1
  os.rmdir(self.name,dir_fd=self.parent_fd); os.fsync(self.parent_fd)
  try: os.stat(self.name,dir_fd=self.parent_fd,follow_symlinks=False)
  except FileNotFoundError: pass
  else: raise OSError('workspace root remains')
  os.close(self.parent_fd); self.parent_fd=-1
  result={**counts,'root_removed':True,'descriptors_closed':True,'residue_entries':0}
  if role_identity_changed:
   self._cleanup_error='workspace role identity changed after owned contents were deleted'
   raise OSError(self._cleanup_error)
  self._cleanup=result
  return dict(self._cleanup)

 def __enter__(self): return self
 def __exit__(self,_type,_value,_traceback): self.cleanup()


def create_stage1_workspace(parent_fd:int,generation:str,candidate:Mapping[str,Any])->PrivateWorkspace:
 required=candidate.get('provenance',{}).get('minimum_free_bytes_before_each_transfer')
 if type(required) is not int or required<=0: raise ValueError('candidate disk requirement missing')
 return PrivateWorkspace.create(parent_fd,generation,required)


def extract_tar(fileobj,root_fd:int,maximum_entries:int,maximum_bytes:int,_allow_existing:bool=False)->List[Dict[str,Any]]:
 if os.listdir(root_fd) and not _allow_existing: raise ValueError('dirty extraction destination')
 try:
  try: fileobj.seek(0)
  except (AttributeError,OSError) as error: raise ValueError('archive is not seekable') from error
  with tarfile.open(fileobj=fileobj,mode='r:*') as archive:
   if archive.pax_headers: raise ValueError('global PAX metadata is forbidden')
   members=_bounded_tar_members(archive,maximum_entries,maximum_bytes)
  roster=validate_tar_members(members,maximum_entries,maximum_bytes); output={item['path']:dict(item) for item in roster}
  directories=set()
  for record in roster:
   parts=record['path'].split('/')
   directories.update('/'.join(parts[:index]) for index in range(1,len(parts)))
   if record['kind']=='directory': directories.add(record['path'])
  for path in sorted(directories,key=lambda value:(value.count('/'),value)):
   descriptor=_open_directory(root_fd,path.split('/'),True); os.close(descriptor)
  fileobj.seek(0)
  with tarfile.open(fileobj=fileobj,mode='r:*') as archive:
   if archive.pax_headers: raise ValueError('global PAX metadata is forbidden')
   seen=0
   while True:
    info=archive.next()
    if info is None: break
    if seen>=len(members) or not _same_tar_member(info,members[seen]): raise ValueError('archive changed between validation and extraction')
    seen+=1; path=_archive_path(info.name); record=output[path]
    if record['kind']!='file': continue
    parent,name=_parent_directory(root_fd,path); flags=os.O_WRONLY|os.O_CREAT|os.O_EXCL
    if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
    descriptor=os.open(name,flags,0o600,dir_fd=parent); source=archive.extractfile(info); digest=hashlib.sha256(); total=0
    try:
     if source is None: raise ValueError('missing archive payload')
     while True:
      chunk=source.read(131072)
      if not chunk: break
      total+=len(chunk)
      if total>record['bytes']: raise ValueError('archive payload overflow')
      digest.update(chunk); _write_all(descriptor,chunk)
     if total!=record['bytes']: raise ValueError('archive payload size mismatch')
     os.fsync(descriptor); os.fchmod(descriptor,record['mode']); output[path]['sha256']=digest.hexdigest()
    finally:
     if source is not None: source.close()
     os.close(descriptor); os.close(parent)
   if seen!=len(members): raise ValueError('archive changed between validation and extraction')
  by_path={record['path']:record for record in roster}
  def hard_target(path:str,active:set)->str:
   if path in active: raise ValueError('hard-link cycle')
   record=by_path[path]
   if record['kind']=='hardlink': return hard_target(_link_target(path,record['link_target'],True),active|{path})
   if record['kind']!='file': raise ValueError('hard-link target is not a file')
   return path
  for record in roster:
   if record['kind']!='hardlink': continue
   target=hard_target(record['path'],set())
   source_parent,source_name=_parent_directory(root_fd,target,False); target_parent,target_name=_parent_directory(root_fd,record['path'])
   try: os.link(source_name,target_name,src_dir_fd=source_parent,dst_dir_fd=target_parent,follow_symlinks=False)
   finally: os.close(source_parent); os.close(target_parent)
   output[record['path']]['sha256']=output[target]['sha256']
  for record in roster:
   if record['kind']!='symlink': continue
   parent,name=_parent_directory(root_fd,record['path'])
   try: os.symlink(record['link_target'],name,dir_fd=parent)
   finally: os.close(parent)
  for record in sorted((value for value in roster if value['kind']=='directory'),key=lambda value:value['path'].count('/'),reverse=True):
   descriptor=_open_directory(root_fd,record['path'].split('/'),False)
   try: os.fchmod(descriptor,record['mode'])
   finally: os.close(descriptor)
  os.fsync(root_fd)
  return [output[path] for path in sorted(output)]
 except BaseException:
  _clear_directory(root_fd)
  if os.listdir(root_fd): raise OSError('extraction rollback incomplete')
  raise


def extract_tar_layers(fileobjs,root_fd:int,maximum_entries:int,maximum_bytes:int,*,layer_limits:Optional[List[Mapping[str,Any]]]=None)->List[Dict[str,Any]]:
 layers=list(fileobjs)
 if not layers or os.listdir(root_fd): raise ValueError('dirty or empty layer transaction')
 if layer_limits is not None:
  if not isinstance(layer_limits,list) or len(layer_limits)!=len(layers): raise ValueError('layer extraction limit shape mismatch')
  for item in layer_limits:
   if not isinstance(item,dict) or set(item) not in ({'formula','installed_bytes'},{'formula','maximum_extracted_bytes'}) or not isinstance(item['formula'],str): raise ValueError('layer extraction limit shape mismatch')
   key='installed_bytes' if 'installed_bytes' in item else 'maximum_extracted_bytes'
   if type(item[key]) is not int or item[key]<0: raise ValueError('layer extraction limit shape mismatch')
 roster=[]; aliases={}; paths=set(); expanded=0
 try:
  for index,fileobj in enumerate(layers):
   current=extract_tar(fileobj,root_fd,maximum_entries-len(roster),maximum_bytes-expanded,_allow_existing=index>0)
   layer_bytes=sum(record['bytes'] for record in current)
   if layer_limits is not None:
    limit=layer_limits[index]
    if 'installed_bytes' in limit and layer_bytes!=limit['installed_bytes']: raise ValueError('layer installed-size mismatch')
    if 'maximum_extracted_bytes' in limit and layer_bytes>limit['maximum_extracted_bytes']: raise ValueError('per-layer extraction cap exceeded')
   for record in current:
    if record['path'] in paths: raise ValueError('cross-layer path collision')
    parts=record['path'].split('/')
    for depth in range(1,len(parts)+1):
     candidate='/'.join(parts[:depth]); alias=unicodedata.normalize('NFD',candidate).casefold(); prior=aliases.get(alias)
     if prior is not None and prior!=candidate: raise ValueError('cross-layer path collision')
     aliases[alias]=candidate
    paths.add(record['path'])
   roster.extend(current); expanded+=sum(record['bytes'] for record in current)
  return sorted(roster,key=lambda record:record['path'])
 except BaseException:
  _clear_directory(root_fd)
  if os.listdir(root_fd): raise OSError('layer transaction rollback incomplete')
  raise


def _read_regular_at(parent_fd:int,name:str,expected,maximum_bytes:int)->bytes:
  if expected.st_size>maximum_bytes: raise ValueError('inventory file limit')
  flags=os.O_RDONLY
  if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
  descriptor=os.open(name,flags,dir_fd=parent_fd)
  try:
   current=os.fstat(descriptor)
   if not stat.S_ISREG(current.st_mode) or (current.st_dev,current.st_ino,current.st_size)!=(expected.st_dev,expected.st_ino,expected.st_size): raise ValueError('inventory file identity changed')
   chunks=[]; remaining=current.st_size
   while remaining:
    chunk=os.read(descriptor,min(131072,remaining))
    if not chunk: raise ValueError('inventory file truncated')
    chunks.append(chunk); remaining-=len(chunk)
   if os.read(descriptor,1): raise ValueError('inventory file grew')
   return b''.join(chunks)
  finally: os.close(descriptor)

def _scan_runtime(root_fd:int,maximum_entries:int,maximum_bytes:int):
 roster=[]; contents={}; aliases={}; total=0; inode_paths={}; inode_counts={}; inode_values={}
 def walk(directory_fd:int,prefix:str):
  nonlocal total
  for name in sorted(os.listdir(directory_fd)):
   path=prefix+'/'+name if prefix else name; canonical=_archive_path(path); parts=canonical.split('/')
   for index in range(1,len(parts)+1):
    candidate='/'.join(parts[:index]); alias=unicodedata.normalize('NFD',candidate).casefold(); prior=aliases.get(alias)
    if prior is not None and prior!=candidate: raise ValueError('runtime path collision')
    aliases[alias]=candidate
   info=os.stat(name,dir_fd=directory_fd,follow_symlinks=False)
   mode=stat.S_IMODE(info.st_mode)
   if stat.S_ISDIR(info.st_mode):
    roster.append({'path':canonical,'kind':'directory','bytes':0,'mode':mode})
    child=_open_directory(directory_fd,[name],False)
    try: walk(child,canonical)
    finally: os.close(child)
   elif stat.S_ISREG(info.st_mode):
    identity=(info.st_dev,info.st_ino); prior=inode_paths.get(identity)
    inode_counts[identity]=(inode_counts.get(identity,(0,info.st_nlink))[0]+1,info.st_nlink)
    if prior is None:
     data=_read_regular_at(directory_fd,name,info,maximum_bytes-total); digest=hashlib.sha256(data).hexdigest()
     total+=len(data); inode_paths[identity]=canonical; inode_values[identity]=(len(data),digest); contents[canonical]=data
     roster.append({'path':canonical,'kind':'file','bytes':len(data),'mode':mode,'sha256':digest})
    else:
     _size,digest=inode_values[identity]
     roster.append({'path':canonical,'kind':'hardlink','bytes':0,'mode':mode,'sha256':digest,'link_target':prior})
   elif stat.S_ISLNK(info.st_mode):
    target=os.readlink(name,dir_fd=directory_fd)
    _link_target(canonical,target,False)
    roster.append({'path':canonical,'kind':'symlink','bytes':0,'mode':mode,'link_target':target})
   else: raise ValueError('special runtime entry')
   if len(roster)>maximum_entries: raise ValueError('runtime entry limit')
 walk(root_fd,'')
 if any(observed!=expected for observed,expected in inode_counts.values()): raise ValueError('runtime hard link escapes root')
 return sorted(roster,key=lambda value:value['path']),contents

def _macho_string(data:bytes,start:int,size:int,offset:int,minimum:int)->str:
  if offset<minimum or offset>=size: raise ValueError('invalid Mach-O string offset')
  end=data.find(b'\0',start+offset,start+size)
  if end<0: raise ValueError('unterminated Mach-O string')
  try: value=data[start+offset:end].decode('utf-8')
  except UnicodeError as error: raise ValueError('invalid Mach-O string') from error
  if not value or any(ord(char)<0x20 or ord(char)==0x7f for char in value): raise ValueError('invalid Mach-O string')
  return value

def _parse_code_directory(data:bytes,start:int,length:int,slice_start:int,data_offset:int)->Dict[str,Any]:
 if length<44: raise ValueError('invalid CodeDirectory structure')
 magic,total,version,flags,hash_offset,identifier_offset,special_slots,code_slots,code_limit,hash_size,hash_type,platform,page_exponent,spare2=struct.unpack_from('>9I4BI',data,start)
 if magic!=0xfade0c02 or total!=length or not 0x20000<=version<=0x20600 or spare2!=0: raise ValueError('invalid CodeDirectory structure')
 header_size=44
 if version>=0x20100: header_size=48
 if version>=0x20200: header_size=52
 if version>=0x20300:
  header_size=64
  if length<header_size: raise ValueError('invalid CodeDirectory structure')
  code_limit_64=struct.unpack_from('>Q',data,start+56)[0]
  if code_limit==0: code_limit=code_limit_64
 if version>=0x20400: header_size=88
 if version>=0x20500: header_size=96
 if version>=0x20600: header_size=108
 if length<header_size or code_limit!=data_offset or code_limit<=0 or not 0<=special_slots<=64 or not 1<=code_slots<=1048576: raise ValueError('invalid CodeDirectory structure')
 hash_algorithms={1:('sha1',20),2:('sha256',32),3:('sha256',20),4:('sha384',48)}
 algorithm=hash_algorithms.get(hash_type)
 if algorithm is None or hash_size!=algorithm[1] or page_exponent>30: raise ValueError('invalid CodeDirectory hash policy')
 page_bytes=(1<<page_exponent) if page_exponent else code_limit
 expected_slots=(code_limit+page_bytes-1)//page_bytes
 if code_slots!=expected_slots: raise ValueError('invalid CodeDirectory slot count')
 special_start=hash_offset-special_slots*hash_size; hashes_end=hash_offset+code_slots*hash_size
 if special_start<header_size or hashes_end>length or not header_size<=identifier_offset<special_start: raise ValueError('invalid CodeDirectory bounds')
 identifier_end=data.find(b'\0',start+identifier_offset,start+special_start)
 if identifier_end<0: raise ValueError('invalid CodeDirectory identifier')
 try: identifier=data[start+identifier_offset:identifier_end].decode('utf-8')
 except UnicodeError as error: raise ValueError('invalid CodeDirectory identifier') from error
 if not identifier or any(ord(char)<0x20 or ord(char)==0x7f for char in identifier): raise ValueError('invalid CodeDirectory identifier')
 for index in range(code_slots):
  page_start=slice_start+index*page_bytes; page_end=slice_start+min(code_limit,(index+1)*page_bytes)
  digest=hashlib.new(algorithm[0],data[page_start:page_end]).digest()[:hash_size]
  observed=data[start+hash_offset+index*hash_size:start+hash_offset+(index+1)*hash_size]
  if not hmac.compare_digest(digest,observed): raise ValueError('CodeDirectory page hash mismatch')
 return {'version':version,'flags':flags,'identifier':identifier,'special_slots':special_slots,'code_slots':code_slots,'code_limit':code_limit,'hash_size':hash_size,'hash_type':hash_type,'platform':platform,'page_size':page_bytes,'hashes_verified':True}


def _parse_code_signature(data:bytes,slice_start:int,slice_size:int,data_offset:int,data_size:int,minimum_offset:int)->Dict[str,Any]:
 if data_offset<minimum_offset or data_size<12 or data_offset>slice_size-data_size: raise ValueError('invalid Mach-O code signature bounds')
 start=slice_start+data_offset; end=start+data_size
 magic,length,count=struct.unpack_from('>III',data,start)
 if magic!=0xfade0cc0 or length!=data_size or not 1<=count<=64 or 12+count*8>length: raise ValueError('invalid Mach-O code signature superblob')
 slots=[]; seen=set(); ranges=[]; table_end=12+count*8
 for index in range(count):
  slot_type,offset=struct.unpack_from('>II',data,start+12+index*8)
  if slot_type in seen or offset<table_end or offset+8>length: raise ValueError('invalid Mach-O code signature slot')
  seen.add(slot_type); slot_magic,slot_length=struct.unpack_from('>II',data,start+offset)
  if slot_length<8 or offset+slot_length>length or any(offset<prior_end and prior<offset+slot_length for prior,prior_end in ranges): raise ValueError('invalid Mach-O code signature slot')
  ranges.append((offset,offset+slot_length)); slot={'type':slot_type,'magic':f'{slot_magic:08x}','bytes':slot_length}
  if slot_magic==0xfade0c02: slot['code_directory']=_parse_code_directory(data,start+offset,slot_length,slice_start,data_offset)
  slots.append(slot)
 if not any('code_directory' in slot for slot in slots): raise ValueError('Mach-O CodeDirectory missing')
 return {'present':True,'data_offset':data_offset,'bytes':data_size,'slots':slots}

def _parse_macho_slice(data:bytes,start:int,size:int)->Dict[str,Any]:
  if start<0 or size<32 or start+size>len(data): raise ValueError('invalid Mach-O slice bounds')
  magic=data[start:start+4]
  formats={b'\xcf\xfa\xed\xfe':('<',True),b'\xfe\xed\xfa\xcf':('>',True),b'\xce\xfa\xed\xfe':('<',False),b'\xfe\xed\xfa\xce':('>',False)}
  if magic not in formats: raise ValueError('invalid Mach-O slice magic')
  endian,is64=formats[magic]; header_size=32 if is64 else 28
  fields=struct.unpack_from(endian+'IiiIIII'+('I' if is64 else ''),data,start)
  _magic,cputype,cpusubtype,filetype,ncmds,sizeofcmds,flags,*_reserved=fields
  if ncmds>4096 or sizeofcmds>size-header_size: raise ValueError('invalid Mach-O command bounds')
  cursor=start+header_size; command_end=cursor+sizeofcmds; loads=[]; rpaths=[]; ids=[]; code_signature=None
  load_commands={0xc,0x18,0x80000018,0x1f,0x8000001f,0x20,0x23,0x80000023}
  for _index in range(ncmds):
   if cursor+8>command_end: raise ValueError('truncated Mach-O command')
   command,command_size=struct.unpack_from(endian+'II',data,cursor)
   if command_size<8 or command_size%4 or cursor+command_size>command_end: raise ValueError('invalid Mach-O command size')
   relative=cursor-start
   if command in load_commands:
    if command_size<24: raise ValueError('short Mach-O dylib command')
    name_offset=struct.unpack_from(endian+'I',data,cursor+8)[0]
    loads.append(_macho_string(data,start+relative,command_size,name_offset,24))
   elif command==0xd:
    if command_size<24: raise ValueError('short Mach-O id command')
    name_offset=struct.unpack_from(endian+'I',data,cursor+8)[0]
    ids.append(_macho_string(data,start+relative,command_size,name_offset,24))
   elif command==0x8000001c:
    if command_size<12: raise ValueError('short Mach-O rpath command')
    path_offset=struct.unpack_from(endian+'I',data,cursor+8)[0]
    rpaths.append(_macho_string(data,start+relative,command_size,path_offset,12))
   elif command==0x1d:
    if command_size!=16 or code_signature is not None: raise ValueError('invalid Mach-O code signature command')
    data_offset,data_size=struct.unpack_from(endian+'II',data,cursor+8)
    code_signature=_parse_code_signature(data,start,size,data_offset,data_size,header_size+sizeofcmds)
   cursor+=command_size
  if cursor!=command_end: raise ValueError('Mach-O command size mismatch')
  if code_signature is None: raise ValueError('unsigned Mach-O slice')
  return {'offset':start,'bytes':size,'cputype':cputype,'cpusubtype':cpusubtype,'filetype':filetype,'flags':flags,'loads':loads,'rpaths':rpaths,'ids':ids,'code_signature':code_signature}

def _parse_macho(data:bytes)->List[Dict[str,Any]]:
  if len(data)<4: raise ValueError('truncated Mach-O')
  magic=data[:4]
  if magic in (b'\xcf\xfa\xed\xfe',b'\xfe\xed\xfa\xcf',b'\xce\xfa\xed\xfe',b'\xfe\xed\xfa\xce'):
   slices=[_parse_macho_slice(data,0,len(data))]
  elif magic in (b'\xca\xfe\xba\xbe',b'\xbe\xba\xfe\xca',b'\xca\xfe\xba\xbf',b'\xbf\xba\xfe\xca'):
   endian='>' if magic[0]==0xca else '<'; is64=magic in (b'\xca\xfe\xba\xbf',b'\xbf\xba\xfe\xca')
   if len(data)<8: raise ValueError('truncated fat Mach-O')
   count=struct.unpack_from(endian+'I',data,4)[0]; arch_size=32 if is64 else 20; table_end=8+count*arch_size
   if not 1<=count<=16 or table_end>len(data): raise ValueError('invalid fat Mach-O table')
   slices=[]; ranges=[]
   for index in range(count):
    offset=8+index*arch_size
    if is64: cputype,cpusubtype,slice_offset,slice_size,alignment,_reserved=struct.unpack_from(endian+'iiQQII',data,offset)
    else: cputype,cpusubtype,slice_offset,slice_size,alignment=struct.unpack_from(endian+'iiIII',data,offset)
    if alignment>31 or slice_offset<table_end or slice_offset+slice_size>len(data) or slice_offset%(1<<alignment): raise ValueError('invalid fat Mach-O slice')
    if any(slice_offset<end and prior<slice_offset+slice_size for prior,end in ranges): raise ValueError('overlapping fat Mach-O slices')
    ranges.append((slice_offset,slice_offset+slice_size)); parsed=_parse_macho_slice(data,slice_offset,slice_size)
    if (parsed['cputype'],parsed['cpusubtype'])!=(cputype,cpusubtype): raise ValueError('fat Mach-O identity mismatch')
    slices.append(parsed)
  else: raise ValueError('not Mach-O')
  if any(value['cputype']!=0x0100000c for value in slices): raise ValueError('non-arm64 Mach-O slice')
  return slices

def _resolve_runtime_path(path:str,record_by_path:Mapping[str,Dict[str,Any]],active:Optional[set]=None,require_file:bool=True)->str:
  active=set() if active is None else active
  if path in active: raise ValueError('runtime symlink cycle')
  record=record_by_path.get(path)
  if record is None: raise ValueError('runtime dependency missing')
  if record['kind']=='symlink': return _resolve_runtime_path(_link_target(path,record['link_target'],False),record_by_path,active|{path},require_file)
  if record['kind']=='hardlink': return _resolve_runtime_path(_link_target(path,record['link_target'],True),record_by_path,active|{path},require_file)
  if require_file and record['kind']!='file': raise ValueError('runtime dependency is not a file')
  return path

def _expand_runtime_reference(value:str,binary_path:str,packages:Mapping[str,Mapping[str,str]])->str:
  if value.startswith('@loader_path/'):
   target=posixpath.normpath(posixpath.join(posixpath.dirname(binary_path),value[len('@loader_path/'):]))
   return _archive_path(target)
  if value.startswith('/opt/homebrew/opt/'):
   remainder=value[len('/opt/homebrew/opt/'):]; formula,separator,suffix=remainder.partition('/')
   if not separator or formula not in packages: raise ValueError('unknown Homebrew load path')
   return _archive_path(packages[formula]['prefix']+'/'+suffix)
  if value.startswith('/opt/homebrew/Cellar/'):
   remainder=value[len('/opt/homebrew/Cellar/'):]; parts=remainder.split('/',2)
   if len(parts)!=3 or parts[0] not in packages or parts[1]!=packages[parts[0]]['version']: raise ValueError('unknown Homebrew load path')
   return _archive_path(packages[parts[0]]['prefix']+'/'+parts[2])
  raise ValueError('unknown Mach-O load path')

def _canonical_system_macho_path(value:str)->bool:
 return isinstance(value,str) and (value.startswith('/usr/lib/') or value.startswith('/System/Library/')) and posixpath.normpath(value)==value and '//' not in value and all(part not in ('','.','..') for part in value.split('/')[1:])

def inspect_runtime(root_fd:int,packages:Mapping[str,Mapping[str,str]],maximum_entries:int=20000,maximum_bytes:int=536870912)->Dict[str,Any]:
  if not isinstance(packages,dict) or not packages: raise ValueError('package inventory is empty')
  for formula,package in packages.items():
   if not re.fullmatch(r'[a-z0-9][a-z0-9.@+-]*',formula) or set(package)!={'version','prefix','license'} or _archive_path(package['prefix'])!=package['prefix']: raise ValueError('invalid package inventory')
  roster,contents=_scan_runtime(root_fd,maximum_entries,maximum_bytes); by_path={record['path']:record for record in roster}
  prefixes=[package['prefix'] for package in packages.values()]
  if any(not any(record['path']==prefix or record['path'].startswith(prefix+'/') or prefix.startswith(record['path']+'/') for prefix in prefixes) for record in roster): raise ValueError('runtime entry outside package closure')
  for record in roster:
   if record['kind']=='symlink': _resolve_runtime_path(record['path'],by_path,require_file=False)
  licenses=[]
  for formula,package in sorted(packages.items()):
   prefix=package['prefix']+'/'
   matches=[record for record in roster if record['kind']=='file' and record['path'].startswith(prefix) and re.fullmatch(r'(?:LICENSE|COPYING|NOTICE)(?:\.[A-Za-z0-9._-]+)?',posixpath.basename(record['path']),re.IGNORECASE)]
   if not matches: raise ValueError('package license missing')
   for record in matches: licenses.append({'formula':formula,'declared_spdx':package['license'],'path':record['path'],'bytes':record['bytes'],'sha256':record['sha256']})
  machos=[]
  macho_magics={b'\xcf\xfa\xed\xfe',b'\xfe\xed\xfa\xcf',b'\xce\xfa\xed\xfe',b'\xfe\xed\xfa\xce',b'\xca\xfe\xba\xbe',b'\xbe\xba\xfe\xca',b'\xca\xfe\xba\xbf',b'\xbf\xba\xfe\xca'}
  for path,data in sorted(contents.items()):
   if data[:4] not in macho_magics:
    if path.endswith(('.dylib','.so','.bundle')): raise ValueError('dynamic library is not Mach-O')
    continue
   slices=_parse_macho(data); loads=[]; rpaths=[]; ids=[]
   for slice_info in slices:
    rpaths.extend(slice_info['rpaths'])
    for rpath in slice_info['rpaths']: _expand_runtime_reference(rpath,path,packages)
    if slice_info['filetype']==6:
     if len(slice_info['ids'])!=1: raise ValueError('Mach-O dylib identity missing or ambiguous')
     identity=slice_info['ids'][0]
     if identity.startswith('@rpath/'):
      identity_candidates=[_archive_path(_expand_runtime_reference(rpath,path,packages)+'/'+identity[len('@rpath/'):]) for rpath in slice_info['rpaths']]
      if not any(candidate in by_path and _resolve_runtime_path(candidate,by_path)==path for candidate in identity_candidates): raise ValueError('Mach-O dylib identity mismatch')
     elif _resolve_runtime_path(_expand_runtime_reference(identity,path,packages),by_path)!=path: raise ValueError('Mach-O dylib identity mismatch')
    elif slice_info['ids']: raise ValueError('Mach-O identity on non-dylib')
    ids.extend(slice_info['ids'])
    for value in slice_info['loads']:
     if value.startswith('/usr/lib/') or value.startswith('/System/Library/'):
      if not _canonical_system_macho_path(value): raise ValueError('noncanonical system path')
      resolved='system'
     elif value.startswith('@rpath/'):
      candidates=[]
      for rpath in slice_info['rpaths']:
       base=_expand_runtime_reference(rpath,path,packages)
       candidates.append(_archive_path(base+'/'+value[len('@rpath/'):]))
      resolved=next((_resolve_runtime_path(candidate,by_path) for candidate in candidates if candidate in by_path),None)
      if resolved is None: raise ValueError('runtime dependency missing')
     else:
      resolved=_resolve_runtime_path(_expand_runtime_reference(value,path,packages),by_path)
     loads.append({'path':value,'resolved':resolved})
   public_slices=[{key:value for key,value in item.items() if key not in ('loads','rpaths','ids')} for item in slices]
   machos.append({'path':path,'sha256':hashlib.sha256(data).hexdigest(),'slices':public_slices,'loads':loads,'rpaths':rpaths,'ids':ids})
  macho_by_path={entry['path']:entry for entry in machos}
  for record in roster:
   if record['kind']=='file': record['content_type']='mach-o' if record['path'] in macho_by_path else 'data'
  for record in roster:
   if record['kind']=='hardlink': record['content_type']='mach-o' if _resolve_runtime_path(record['path'],by_path) in macho_by_path else 'data'
  for entry in machos:
   for load in entry['loads']:
    if load['resolved']=='system': continue
    dependency=macho_by_path.get(load['resolved'])
    if dependency is None or any(slice_info['filetype']!=6 for slice_info in dependency['slices']): raise ValueError('runtime dependency is not a Mach-O dylib')
  return {'roster':roster,'mach_o':machos,'licenses':licenses}

RECEIPT_SCHEMA='saturday.local-provider.prebuilt-stage1-receipt.v1'
RECEIPT_KEYS={'schema','candidate_id','candidate_sha256','review_aggregate_sha256','run_id','authorization','upstream','layers','extracted','mach_o','licenses','cleanup','verdict'}


def receipt_layer_records(candidate:Mapping[str,Any])->List[Dict[str,Any]]:
 source=[candidate['runtime'],*candidate['dependencies']]; records=[]
 for item in source:
  records.append({
   'formula':item['formula'],'version':item['version'],
   'index_sha256':item.get('index_digest',item.get('index')),'manifest_sha256':item.get('manifest_digest',item.get('manifest')),'config_sha256':item.get('config_digest',item.get('config')),
   'layer_sha256':item.get('layer_digest',item.get('layer')),'bytes':item['layer_bytes'],
  })
 return records


def _receipt_formulas(candidate:Mapping[str,Any])->Dict[str,Tuple[str,str]]:
 values=[candidate['runtime'],*candidate['dependencies']]
 return {item['formula']:(item['version']+(f"_{item['rebuild']}" if item.get('rebuild') else ''),item['license']) for item in values}


def _hex(value:Any)->bool:
 return isinstance(value,str) and re.fullmatch(r'[0-9a-f]{64}',value) is not None


def _validate_receipt_roster(records:Any)->Dict[str,Dict[str,Any]]:
 if not isinstance(records,list) or not records: raise ValueError('receipt roster missing')
 by_path={}; aliases={}
 for record in records:
  if not isinstance(record,dict) or record.get('kind') not in ('directory','file','symlink','hardlink'): raise ValueError('receipt roster shape mismatch')
  kind=record['kind']; keys={'path','kind','bytes','mode'}
  if kind in ('file','hardlink'): keys.update(('sha256','content_type'))
  if kind in ('symlink','hardlink'): keys.add('link_target')
  if set(record)!=keys or _archive_path(record['path'])!=record['path'] or record['path'] in by_path: raise ValueError('receipt roster shape mismatch')
  parts=record['path'].split('/')
  for index in range(1,len(parts)+1):
   candidate='/'.join(parts[:index]); alias=unicodedata.normalize('NFD',candidate).casefold(); prior=aliases.get(alias)
   if prior is not None and prior!=candidate: raise ValueError('receipt path collision')
   aliases[alias]=candidate
  if type(record['bytes']) is not int or record['bytes']<0 or type(record['mode']) is not int or not 0<=record['mode']<=0o777: raise ValueError('receipt roster value mismatch')
  if kind=='directory' and (record['mode']&0o700)!=0o700 or kind in ('directory','file','hardlink') and record['mode']&0o022 or kind in ('file','hardlink') and not record['mode']&0o400 or kind=='symlink' and record['mode']!=0o777: raise ValueError('receipt unsafe mode')
  if kind in ('file','hardlink') and not _hex(record['sha256']): raise ValueError('receipt file identity mismatch')
  if kind in ('file','hardlink') and record['content_type'] not in ('data','mach-o'): raise ValueError('receipt file type mismatch')
  if kind=='directory' and record['bytes']!=0: raise ValueError('receipt directory size mismatch')
  if kind in ('symlink','hardlink'):
   _link_target(record['path'],record['link_target'],kind=='hardlink')
   if record['bytes']!=0: raise ValueError('receipt link size mismatch')
  by_path[record['path']]=record
 if records!=sorted(records,key=lambda value:value['path']): raise ValueError('receipt roster order mismatch')
 for path in by_path:
  parts=path.split('/')
  if any('/'.join(parts[:index]) not in by_path or by_path['/'.join(parts[:index])]['kind']!='directory' for index in range(1,len(parts))): raise ValueError('receipt ancestor missing')
 def resolve(path:str,active:set)->str:
  if path in active: raise ValueError('receipt link cycle')
  record=by_path.get(path)
  if record is None: raise ValueError('receipt link target missing')
  if record['kind'] in ('symlink','hardlink'):
   final=resolve(_link_target(path,record['link_target'],record['kind']=='hardlink'),active|{path})
   if record['kind']=='hardlink' and final!='file': raise ValueError('receipt hard-link target mismatch')
   return final
  return record['kind']
 for path,record in by_path.items():
  if record['kind'] in ('symlink','hardlink'): resolve(path,set())
 for path,record in by_path.items():
  if record['kind']=='hardlink':
   target=_link_target(path,record['link_target'],True)
   while by_path[target]['kind']=='hardlink': target=_link_target(target,by_path[target]['link_target'],True)
   if record['content_type']!=by_path[target]['content_type'] or record['sha256']!=by_path[target]['sha256']: raise ValueError('receipt hard-link identity mismatch')
 return by_path


def _validate_receipt_text_policy(value:Any)->None:
 if isinstance(value,dict):
  for item in value.values(): _validate_receipt_text_policy(item)
 elif isinstance(value,list):
  for item in value: _validate_receipt_text_policy(item)
 elif isinstance(value,str):
  lowered=value.casefold()
  if '\0' in value or re.search(r'(?i)\bbearer\s',value) or re.search(r'(?i)\bauthorization\s*:',value) or any(marker in lowered for marker in ('/private/','/tmp/','/users/','file://','http://','https://')): raise ValueError('receipt redaction policy mismatch')


def validate_stage1_receipt(receipt:Any,candidate:Mapping[str,Any],review_aggregate_sha256:str,expected_authorization_sha256:str)->str:
 if not isinstance(receipt,dict) or set(receipt)!=RECEIPT_KEYS or receipt['schema']!=RECEIPT_SCHEMA: raise ValueError('receipt shape mismatch')
 _validate_receipt_text_policy(receipt)
 if receipt['candidate_id']!=candidate['candidate_id'] or receipt['candidate_sha256']!=CANDIDATE_SHA256 or receipt['review_aggregate_sha256']!=review_aggregate_sha256: raise ValueError('receipt candidate mismatch')
 try: run_id=uuid.UUID(receipt['run_id'])
 except (ValueError,TypeError,AttributeError) as error: raise ValueError('invalid receipt run ID') from error
 if str(run_id)!=receipt['run_id'] or run_id.version!=4: raise ValueError('invalid receipt run ID')
 authorization=receipt['authorization']
 if not isinstance(authorization,dict) or set(authorization)!={'authorization_sha256','binding_sha256','generation','key_id','nonce_sha256','purpose'} or authorization['purpose']!='stage1_bottle_transfer' or re.fullmatch(r'[0-9a-f]{32}',str(authorization['generation'])) is None or any(not _hex(authorization[field]) for field in ('authorization_sha256','binding_sha256','key_id','nonce_sha256')) or not _hex(expected_authorization_sha256) or not hmac.compare_digest(authorization['authorization_sha256'],expected_authorization_sha256): raise ValueError('receipt authorization mismatch')
 upstream={'formula':candidate['runtime']['formula'],'version':candidate['runtime']['version'],'commit':candidate['runtime']['upstream_commit']}
 if receipt['upstream']!=upstream or receipt['layers']!=receipt_layer_records(candidate): raise ValueError('receipt upstream mismatch')
 if len(receipt['layers'])!=candidate['authorizations']['bottle_transfer']['artifact_count'] or sum(record['bytes'] for record in receipt['layers'])!=candidate['provenance']['verified_layer_total_bytes']: raise ValueError('receipt layer total mismatch')
 roster=_validate_receipt_roster(receipt['extracted'])
 formulas=_receipt_formulas(candidate)
 if len(roster)>candidate['provenance']['maximum_entries'] or sum(record['bytes'] for record in roster.values() if record['kind']=='file')>candidate['provenance']['maximum_total_extracted_bytes']: raise ValueError('receipt extraction bound mismatch')
 allowed_roots={formula:(formula,formula+'/'+version) for formula,(version,_license) in formulas.items()}
 for path in roster:
  formula=path.split('/',1)[0]; roots=allowed_roots.get(formula)
  if roots is None or not (path==roots[0] or path==roots[1] or path.startswith(roots[1]+'/')): raise ValueError('receipt package path mismatch')
 source_by_formula={item['formula']:item for item in [candidate['runtime'],*candidate['dependencies']]}
 for formula,(_root,prefix) in allowed_roots.items():
  extracted_bytes=sum(record['bytes'] for path,record in roster.items() if record['kind']=='file' and path.startswith(prefix+'/'))
  source=source_by_formula[formula]
  if 'installed_bytes' in source and extracted_bytes!=source['installed_bytes']: raise ValueError('receipt installed-size mismatch')
  if 'maximum_extracted_bytes' in source and extracted_bytes>source['maximum_extracted_bytes']: raise ValueError('receipt per-layer extraction cap exceeded')
 receipt_packages={formula:{'version':version,'prefix':formula+'/'+version,'license':license_name} for formula,(version,license_name) in formulas.items()}
 machos=receipt['mach_o']
 if not isinstance(machos,list) or not machos or machos!=sorted(machos,key=lambda value:value['path']): raise ValueError('receipt Mach-O inventory mismatch')
 macho_by_path={}
 for item in machos:
  if not isinstance(item,dict) or set(item)!={'path','sha256','slices','loads','rpaths','ids'} or item['path'] in macho_by_path or item['path'] not in roster or roster[item['path']]['kind'] not in ('file','hardlink') or roster[item['path']]['content_type']!='mach-o' or item['sha256']!=roster[item['path']]['sha256']: raise ValueError('receipt Mach-O identity mismatch')
  if not isinstance(item['slices'],list) or not item['slices'] or not isinstance(item['loads'],list) or not isinstance(item['rpaths'],list) or not isinstance(item['ids'],list): raise ValueError('receipt Mach-O inventory mismatch')
  ranges=[]
  for slice_info in item['slices']:
   numeric={'offset','bytes','cputype','cpusubtype','filetype','flags'}
   if not isinstance(slice_info,dict) or set(slice_info)!=numeric|{'code_signature'} or any(type(slice_info[key]) is not int for key in numeric) or slice_info['offset']<0 or slice_info['bytes']<=0 or slice_info['offset']+slice_info['bytes']>roster[item['path']]['bytes'] or slice_info['cputype']!=0x0100000c: raise ValueError('receipt Mach-O slice mismatch')
   signature=slice_info['code_signature']
   if not isinstance(signature,dict) or set(signature)!={'present','data_offset','bytes','slots'} or signature['present'] is not True or type(signature['data_offset']) is not int or type(signature['bytes']) is not int or signature['data_offset']<0 or signature['bytes']<=0 or signature['data_offset']+signature['bytes']>slice_info['bytes'] or not isinstance(signature['slots'],list) or not signature['slots']: raise ValueError('receipt Mach-O signature mismatch')
   code_directories=[]
   for slot in signature['slots']:
    if not isinstance(slot,dict) or not {'type','magic','bytes'}<=set(slot) or type(slot['type']) is not int or type(slot['bytes']) is not int or slot['bytes']<8 or not isinstance(slot['magic'],str) or re.fullmatch(r'[0-9a-f]{8}',slot['magic']) is None: raise ValueError('receipt Mach-O signature mismatch')
    if slot['magic']=='fade0c02':
     if set(slot)!={'type','magic','bytes','code_directory'} or slot['bytes']<44: raise ValueError('receipt Mach-O CodeDirectory mismatch')
     directory=slot['code_directory']; numeric={'version','flags','special_slots','code_slots','code_limit','hash_size','hash_type','platform','page_size'}
     if not isinstance(directory,dict) or set(directory)!=numeric|{'identifier','hashes_verified'} or any(type(directory[key]) is not int for key in numeric) or not isinstance(directory['identifier'],str) or not directory['identifier'] or directory['hashes_verified'] is not True: raise ValueError('receipt Mach-O CodeDirectory mismatch')
     expected_hash_sizes={1:20,2:32,3:20,4:48}
     if not 0x20000<=directory['version']<=0x20600 or directory['hash_size']!=expected_hash_sizes.get(directory['hash_type']) or not 0<=directory['special_slots']<=64 or directory['code_slots']<1 or directory['code_limit']!=signature['data_offset'] or directory['page_size']<1 or directory['code_slots']!=(directory['code_limit']+directory['page_size']-1)//directory['page_size']: raise ValueError('receipt Mach-O CodeDirectory mismatch')
     code_directories.append(directory)
    elif set(slot)!={'type','magic','bytes'}: raise ValueError('receipt Mach-O signature mismatch')
   if not code_directories: raise ValueError('receipt Mach-O CodeDirectory missing')
   interval=(slice_info['offset'],slice_info['offset']+slice_info['bytes'])
   if any(interval[0]<end and start<interval[1] for start,end in ranges): raise ValueError('receipt Mach-O slice overlap')
   ranges.append(interval)
  if any(not isinstance(value,str) or not value or '\0' in value for value in item['ids']): raise ValueError('receipt Mach-O identity mismatch')
  if len(item['ids'])!=sum(slice_info['filetype']==6 for slice_info in item['slices']): raise ValueError('receipt Mach-O identity mismatch')
  for rpath in item['rpaths']:
   if not isinstance(rpath,str) or '\0' in rpath or '://' in rpath: raise ValueError('receipt local path disclosure')
   try: _expand_runtime_reference(rpath,item['path'],receipt_packages)
   except ValueError as error: raise ValueError('receipt unreviewed RPATH') from error
  macho_by_path[item['path']]=item
 if set(macho_by_path)!={path for path,record in roster.items() if record.get('content_type')=='mach-o'}: raise ValueError('receipt Mach-O closure mismatch')
 for item in machos:
  for load in item['loads']:
   if not isinstance(load,dict) or set(load)!={'path','resolved'} or not isinstance(load['path'],str) or not isinstance(load['resolved'],str) or not load['path']: raise ValueError('receipt Mach-O load mismatch')
   if load['resolved']=='system':
    if not _canonical_system_macho_path(load['path']): raise ValueError('receipt system load mismatch')
   else:
    dependency=macho_by_path.get(load['resolved'])
    if not (load['path'].startswith('/opt/homebrew/') or load['path'].startswith('@rpath/') or load['path'].startswith('@loader_path/')) or dependency is None or any(slice_info['filetype']!=6 for slice_info in dependency['slices']): raise ValueError('receipt Mach-O dependency mismatch')
 formulas=_receipt_formulas(candidate); licenses=receipt['licenses']
 if not isinstance(licenses,list) or licenses!=sorted(licenses,key=lambda value:(value['formula'],value['path'])): raise ValueError('receipt license order mismatch')
 seen=set()
 for item in licenses:
  if not isinstance(item,dict) or set(item)!={'formula','declared_spdx','path','bytes','sha256'} or item['formula'] not in formulas or item['declared_spdx']!=formulas[item['formula']][1]: raise ValueError('receipt license mismatch')
  prefix=item['formula']+'/'+formulas[item['formula']][0]+'/'
  if not item['path'].startswith(prefix) or re.fullmatch(r'(?:LICENSE|COPYING|NOTICE)(?:\.[A-Za-z0-9._-]+)?',posixpath.basename(item['path']),re.IGNORECASE) is None: raise ValueError('receipt license path mismatch')
  if item['path'] not in roster or roster[item['path']]['kind'] not in ('file','hardlink') or roster[item['path']]['content_type']!='data' or item['bytes']!=roster[item['path']]['bytes'] or item['sha256']!=roster[item['path']]['sha256']: raise ValueError('receipt license identity mismatch')
  seen.add(item['formula'])
 if seen!=set(formulas): raise ValueError('receipt license closure mismatch')
 cleanup=receipt['cleanup']; expected_cleanup={
  'files':1+len(receipt['layers'])+sum(record['kind'] in ('file','hardlink') for record in roster.values()),
  'directories':3+sum(record['kind']=='directory' for record in roster.values()),
  'links':sum(record['kind']=='symlink' for record in roster.values()),'root_removed':True,'descriptors_closed':True,'residue_entries':0,
 }
 if cleanup!=expected_cleanup: raise ValueError('receipt cleanup mismatch')
 if receipt['verdict']!='MODEL_TRANSFER_BLOCKED' or candidate['authorizations']['model_transfer']['state']!='blocked_until_relocated_runtime_review': raise ValueError('receipt verdict mismatch')
 return canonical_json_sha256(receipt)


class _Stage1Lifecycle:
 def __init__(self,completed:CompletedTransferAuthorization,inventory:Mapping[str,Any],cleanup:Mapping[str,Any],seal:object):
  if seal is not _LIFECYCLE_SEAL or not getattr(completed,'_sealed',False): raise PermissionError('stage-1 lifecycle cannot be constructed directly')
  self.completed=completed
  self.inventory=json.loads(canonical_json_bytes(inventory))
  self.cleanup=json.loads(canonical_json_bytes(cleanup))


def _complete_stage1_lifecycle(completed:CompletedTransferAuthorization,inventory:Mapping[str,Any],cleanup:Mapping[str,Any])->_Stage1Lifecycle:
 return _Stage1Lifecycle(completed,inventory,cleanup,_LIFECYCLE_SEAL)


def build_stage1_receipt(candidate:Mapping[str,Any],review_aggregate_sha256:str,run_id:str,lifecycle:_Stage1Lifecycle)->Dict[str,Any]:
 if not isinstance(lifecycle,_Stage1Lifecycle): raise TypeError('sealed stage-1 lifecycle required')
 expected_artifacts=[]
 for item in [candidate['runtime'],*candidate['dependencies']]:
  expected_artifacts.append((item.get('oci_repository',item.get('repository')),item.get('layer_digest',item.get('layer')),item['layer_bytes']))
 if lifecycle.completed.artifacts!=tuple(expected_artifacts): raise PermissionError('completed transfer set does not match candidate')
 receipt={
  'schema':RECEIPT_SCHEMA,'candidate_id':candidate['candidate_id'],'candidate_sha256':CANDIDATE_SHA256,'review_aggregate_sha256':review_aggregate_sha256,'run_id':run_id,
  'authorization':dict(lifecycle.completed.public),'upstream':{'formula':candidate['runtime']['formula'],'version':candidate['runtime']['version'],'commit':candidate['runtime']['upstream_commit']},
  'layers':receipt_layer_records(candidate),'extracted':lifecycle.inventory['roster'],'mach_o':lifecycle.inventory['mach_o'],'licenses':lifecycle.inventory['licenses'],'cleanup':dict(lifecycle.cleanup),'verdict':'MODEL_TRANSFER_BLOCKED',
 }
 validate_stage1_receipt(receipt,candidate,review_aggregate_sha256,lifecycle.completed.public['authorization_sha256'])
 return receipt


def _read_receipt_destination_at(parent_fd:int,name:str,maximum_bytes:int=16777216)->bytes:
 try: before=os.stat(name,dir_fd=parent_fd,follow_symlinks=False)
 except OSError as error: raise PermissionError('receipt destination is missing or unsafe') from error
 if not stat.S_ISREG(before.st_mode) or before.st_nlink!=1 or stat.S_IMODE(before.st_mode)!=0o600 or before.st_uid!=os.getuid() or before.st_size>maximum_bytes: raise PermissionError('receipt destination is not a canonical regular file')
 flags=os.O_RDONLY
 if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
 try: descriptor=os.open(name,flags,dir_fd=parent_fd)
 except OSError as error: raise PermissionError('receipt destination is missing or unsafe') from error
 try:
  current=os.fstat(descriptor)
  identity=lambda value:(value.st_dev,value.st_ino,value.st_uid,value.st_mode,value.st_nlink,value.st_size)
  if identity(current)!=identity(before): raise PermissionError('receipt destination identity changed')
  chunks=[]; remaining=current.st_size
  while remaining:
   chunk=os.read(descriptor,min(131072,remaining))
   if not chunk: raise PermissionError('receipt destination is truncated')
   chunks.append(chunk); remaining-=len(chunk)
  if os.read(descriptor,1) or identity(os.fstat(descriptor))!=identity(current): raise PermissionError('receipt destination changed')
  return b''.join(chunks)
 finally: os.close(descriptor)


def _receipt_registration(parent_fd:int,name:str)->Dict[str,Any]:
 info=os.fstat(parent_fd)
 if not stat.S_ISDIR(info.st_mode) or stat.S_IMODE(info.st_mode)!=0o700 or info.st_uid!=os.getuid() or re.fullmatch(r'stage1-[0-9a-f]{32}\.json',name or '') is None: raise PermissionError('receipt destination provenance is invalid')
 return {'parent_device':info.st_dev,'parent_inode':info.st_ino,'parent_uid':info.st_uid,'name':name}


def _recheck_authorized_directories(authorization:owner_auth.VerifiedOwnerAuthorization,ledger_fd:int,workspace_parent_fd:int,receipt_parent_fd:int)->None:
 observed={
  'ledger':os.fstat(ledger_fd),
  'workspace_parent':os.fstat(workspace_parent_fd),
  'receipt':os.fstat(receipt_parent_fd),
 }
 signed={
  'ledger':authorization.payload['ledger'],
  'workspace_parent':authorization.payload['workspace_parent'],
  'receipt':authorization.payload['receipt'],
 }
 for name,info in observed.items():
  expected=signed[name]
  if not stat.S_ISDIR(info.st_mode) or stat.S_IMODE(info.st_mode)!=0o700 or info.st_uid!=os.getuid() or (info.st_dev,info.st_ino,info.st_uid)!=(expected['device'],expected['inode'],expected['uid']): raise PermissionError('authorized directory identity mismatch')
 if len({(info.st_dev,info.st_ino) for info in observed.values()})!=3 or signed['receipt']['name']!=authorization.receipt_name: raise PermissionError('authorized directory roles overlap')


def _publish_stage1_receipt(receipt_parent_fd:int,receipt_name:str,receipt:Mapping[str,Any],candidate:Mapping[str,Any],review_aggregate_sha256:str,expected_authorization_sha256:str,generation:str,*,expected_receipt_sha256:Optional[str]=None,authorization_ledger_fd:Optional[int]=None)->str:
 if not isinstance(receipt,dict): raise TypeError('stage-1 receipt mapping required')
 if expected_receipt_sha256 is None or authorization_ledger_fd is None: raise PermissionError('durable receipt provenance is required')
 digest=validate_stage1_receipt(receipt,candidate,review_aggregate_sha256,expected_authorization_sha256)
 if not _hex(expected_receipt_sha256) or not hmac.compare_digest(digest,expected_receipt_sha256): raise ValueError('receipt digest mismatch')
 assert authorization_ledger_fd is not None
 marker=_verify_receipt_provenance(authorization_ledger_fd,generation,receipt,digest,'cleaned')
 payload=canonical_json_bytes(receipt)
 registration=_receipt_registration(receipt_parent_fd,receipt_name)
 if marker['receipt_registration']!=registration: raise PermissionError('receipt destination provenance mismatch')
 copy_verified_at(io.BytesIO(payload),receipt_parent_fd,receipt_name,len(payload),digest,len(payload))
 marker['state']='published'; _replace_ledger_marker(authorization_ledger_fd,generation,marker)
 _verify_receipt_provenance(authorization_ledger_fd,generation,receipt,digest,'published')
 return digest


def load_stage1_receipt(receipt_parent_fd:int,receipt_name:str,candidate:Mapping[str,Any],review_aggregate_sha256:str,expected_authorization_sha256:str,generation:str,*,expected_receipt_sha256:Optional[str]=None,authorization_ledger_fd:Optional[int]=None)->Dict[str,Any]:
 if expected_receipt_sha256 is None or authorization_ledger_fd is None: raise PermissionError('durable receipt provenance is required')
 _receipt_registration(receipt_parent_fd,receipt_name)
 payload=_read_receipt_destination_at(receipt_parent_fd,receipt_name); digest=hashlib.sha256(payload).hexdigest()
 if not _hex(expected_receipt_sha256) or not hmac.compare_digest(digest,expected_receipt_sha256): raise ValueError('receipt digest mismatch')
 value=_decode_json(payload)
 if payload!=canonical_json_bytes(value): raise ValueError('receipt is not canonical JSON')
 validate_stage1_receipt(value,candidate,review_aggregate_sha256,expected_authorization_sha256)
 assert authorization_ledger_fd is not None
 _verify_receipt_provenance(authorization_ledger_fd,generation,value,digest,'published')
 return value


def _reconcile_stage1_receipt(ledger_fd:int,receipt_parent_fd:int,receipt_candidate:Mapping[str,Any],review_aggregate_sha256:str,authorization:owner_auth.VerifiedOwnerAuthorization)->Optional[str]:
 marker=_read_ledger_marker_if_present(ledger_fd,authorization.generation)
 if marker is None: return None
 if marker['state'] not in ('cleaned','published'): raise PermissionError('authorization already consumed without a recoverable receipt')
 expected={
  'authorization_sha256':authorization.envelope_sha256,'binding_sha256':authorization.binding_sha256,'key_id':authorization.key_id,
  'candidate_id':authorization.payload['candidate']['candidate_id'],'candidate_sha256':authorization.payload['candidate']['candidate_sha256'],
  'review_aggregate_sha256':review_aggregate_sha256,'review_manifest_sha256':authorization.payload['review']['review_manifest_sha256'],'workspace':None,
 }
 if any(marker.get(key)!=value for key,value in expected.items()): raise PermissionError('durable receipt provenance mismatch')
 registration=_receipt_registration(receipt_parent_fd,authorization.receipt_name)
 if marker['receipt_registration']!=registration: raise PermissionError('receipt destination provenance mismatch')
 payload=_read_receipt_destination_at(receipt_parent_fd,registration['name'])
 digest=hashlib.sha256(payload).hexdigest()
 if not _hex(marker['receipt_sha256']) or not hmac.compare_digest(digest,marker['receipt_sha256']): raise ValueError('receipt digest mismatch')
 receipt=_decode_json(payload)
 if payload!=canonical_json_bytes(receipt): raise ValueError('receipt is not canonical JSON')
 if receipt.get('authorization',{}).get('nonce_sha256')!=authorization.nonce_sha256 or receipt.get('authorization',{}).get('generation')!=authorization.generation: raise PermissionError('receipt authorization mismatch')
 validated=validate_stage1_receipt(receipt,receipt_candidate,review_aggregate_sha256,authorization.envelope_sha256)
 if not hmac.compare_digest(validated,digest): raise ValueError('receipt digest mismatch')
 _verify_receipt_provenance(ledger_fd,authorization.generation,receipt,digest,marker['state'])
 if marker['state']=='cleaned':
  marker['state']='published'; _replace_ledger_marker(ledger_fd,authorization.generation,marker)
  _verify_receipt_provenance(ledger_fd,authorization.generation,receipt,digest,'published')
 return digest


def _artifact_values(item:Mapping[str,Any])->Tuple[str,str,int]:
 repository=item.get('oci_repository',item.get('repository')); digest=item.get('layer_digest',item.get('layer')); size=item.get('layer_bytes')
 if not isinstance(repository,str) or not isinstance(digest,str) or type(size) is not int: raise ValueError('invalid candidate artifact')
 return repository,digest,size


def _replay_oci_metadata(transport:OCITransport,item:Mapping[str,Any],token:str)->None:
 index_digest=item.get('index_digest',item.get('index')); manifest_digest=item.get('manifest_digest',item.get('manifest')); config_digest=item.get('config_digest',item.get('config'))
 if index_digest is None:
  if any(value is not None for value in (manifest_digest,config_digest,item.get('child_annotation_ref'))): raise ValueError('partial OCI metadata identity')
  return
 if not all(isinstance(value,str) and re.fullmatch(r'[0-9a-f]{64}',value) for value in (index_digest,manifest_digest,config_digest)) or not isinstance(item.get('child_annotation_ref'),str): raise ValueError('invalid OCI metadata identity')
 repository,_layer_digest,layer_bytes=_artifact_values(item)
 index=transport.fetch_json(repository,'manifests',index_digest,token,16777216)
 select_oci_manifest(index,item['child_annotation_ref'],manifest_digest)
 manifest=transport.fetch_json(repository,'manifests',manifest_digest,token,16777216)
 verify_oci_manifest(manifest,config_digest,_artifact_values(item)[1],layer_bytes)
 transport.fetch_json(repository,'blobs',config_digest,token,16777216)


def _layer_extraction_limits(candidate:Mapping[str,Any])->List[Dict[str,Any]]:
 limits=[]
 for item in [candidate['runtime'],*candidate['dependencies']]:
  if 'installed_bytes' in item: limits.append({'formula':item['formula'],'installed_bytes':item['installed_bytes']})
  elif 'maximum_extracted_bytes' in item: limits.append({'formula':item['formula'],'maximum_extracted_bytes':item['maximum_extracted_bytes']})
  else: raise ValueError('layer extraction identity missing')
 return limits


def _runtime_packages(candidate:Mapping[str,Any])->Dict[str,Dict[str,str]]:
 packages={}
 for item in [candidate['runtime'],*candidate['dependencies']]:
  version=item['version']+(f"_{item['rebuild']}" if item.get('rebuild') else '')
  packages[item['formula']]={'version':version,'prefix':item['formula']+'/'+version,'license':item['license']}
 return packages


@dataclasses.dataclass(frozen=True)
class Stage1Resources:
 root:Path
 script:Path
 candidate:Path
 candidate_review_manifest:Path
 candidate_review_sidecar:Path
 harness_review_manifest:Path
 harness_review_sidecar:Path
 public_key:Path
 binding:Path

 @classmethod
 def from_script(cls,script_path:Path):
  script=Path(script_path).resolve(strict=True)
  root=script.parent.parent if script.parent.name=='scripts' else script.parent
  return cls(
   root=root,script=script,
   candidate=root/'docs/spikes/005-prebuilt-runtime-candidate/candidate-metadata.json',
   candidate_review_manifest=root/'docs/spikes/005-prebuilt-runtime-candidate/review-manifest.json',
   candidate_review_sidecar=root/'docs/spikes/005-prebuilt-runtime-candidate/review-manifest.json.sha256',
   harness_review_manifest=root/'docs/spikes/005-prebuilt-runtime-candidate/harness-review-manifest.json',
   harness_review_sidecar=root/'docs/spikes/005-prebuilt-runtime-candidate/harness-review-manifest.json.sha256',
   public_key=root/'src-tauri/resources/stage1-owner-public-key.json',
   binding=root/'src-tauri/resources/stage1-owner-binding.json',
  )


def _verify_resource_roster(root:Path,records:List[Mapping[str,Any]])->None:
 if [item.get('path') for item in records]!=sorted(item.get('path') for item in records): raise ValueError('resource roster order mismatch')
 root_info=Path(root).lstat(); flags=os.O_RDONLY|os.O_DIRECTORY
 if not stat.S_ISDIR(root_info.st_mode) or Path(root).is_symlink(): raise ValueError('unsafe resource root')
 if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
 root_fd=os.open(root,flags)
 try:
  for item in records:
   if not isinstance(item,dict) or set(item)!={'bytes','mode','path','sha256'} or _archive_path(item['path'])!=item['path']: raise ValueError('resource roster record mismatch')
   parent,name=_parent_directory(root_fd,item['path'],False); descriptor=None
   try:
    file_flags=os.O_RDONLY
    if hasattr(os,'O_NOFOLLOW'): file_flags|=os.O_NOFOLLOW
    descriptor=os.open(name,file_flags,dir_fd=parent); info=os.fstat(descriptor)
    if not stat.S_ISREG(info.st_mode) or info.st_nlink!=1: raise ValueError('unsafe resource roster file')
    digest=hashlib.sha256(); total=0
    while True:
     chunk=os.read(descriptor,131072)
     if not chunk: break
     total+=len(chunk); digest.update(chunk)
    if {'bytes':total,'mode':stat.S_IMODE(info.st_mode),'path':item['path'],'sha256':digest.hexdigest()}!=item: raise ValueError('resource roster identity mismatch')
   finally:
    if descriptor is not None: os.close(descriptor)
    os.close(parent)
 finally: os.close(root_fd)


def _replay_owner_resources(resources:Stage1Resources,authorization:owner_auth.VerifiedOwnerAuthorization)->Tuple[Dict[str,Any],str]:
 binding_bytes=_read_path(resources.binding,owner_auth.MAX_BINDING_BYTES); binding=owner_auth.parse_frozen_binding(binding_bytes)
 if binding.binding_sha256!=authorization.binding_sha256: raise ValueError('authorization binding identity mismatch')
 manifest_bytes=_read_path(resources.harness_review_manifest); sidecar_bytes=_read_path(resources.harness_review_sidecar,4096)
 if hashlib.sha256(manifest_bytes).hexdigest()!=binding.review['review_manifest_sha256'] or hashlib.sha256(sidecar_bytes).hexdigest()!=binding.review['review_sidecar_sha256']: raise ValueError('harness review identity mismatch')
 if sidecar_bytes!=f"{binding.review['review_manifest_sha256']}  harness-review-manifest.json\n".encode('ascii'): raise ValueError('harness review sidecar mismatch')
 manifest=_decode_json(manifest_bytes)
 if manifest_bytes!=canonical_json_bytes(manifest) or set(manifest)!={'schema','review_aggregate_sha256','files'} or manifest['schema']!=REVIEW_SCHEMA or manifest['review_aggregate_sha256']!=binding.review['review_aggregate_sha256']: raise ValueError('harness review manifest mismatch')
 generated={'docs/spikes/005-prebuilt-runtime-candidate/harness-review-manifest.json','docs/spikes/005-prebuilt-runtime-candidate/harness-review-manifest.json.sha256'}
 base_records=[item for item in binding.resource_roster if item['path'] not in generated]
 if manifest['files']!=base_records: raise ValueError('harness review roster mismatch')
 aggregate=hashlib.sha256(canonical_json_bytes(base_records)).hexdigest()
 if aggregate!=binding.review['review_aggregate_sha256']: raise ValueError('harness review aggregate mismatch')
 _verify_resource_roster(resources.root,binding.resource_roster)
 candidate=load_candidate(resources.candidate)
 candidate_manifest_bytes=_read_path(resources.candidate_review_manifest); candidate_manifest_sha=hashlib.sha256(candidate_manifest_bytes).hexdigest()
 if _read_path(resources.candidate_review_sidecar,4096)!=f'{candidate_manifest_sha}  review-manifest.json\n'.encode('ascii'): raise ValueError('candidate review sidecar mismatch')
 candidate_aggregate=verify_review_manifest(resources.candidate_review_manifest,resources.root,binding.candidate['review_aggregate_sha256'])
 if candidate_aggregate!=binding.candidate['review_aggregate_sha256'] or binding.candidate['candidate_sha256']!=CANDIDATE_SHA256: raise ValueError('candidate binding mismatch')
 _validated_transfer_artifacts(authorization,candidate,aggregate)
 return candidate,aggregate


def run_stage1(resources:Stage1Resources,authorization:owner_auth.VerifiedOwnerAuthorization,authorization_ledger_fd:int,workspace_parent_fd:int,receipt_parent_fd:int,*,connection_factory=None,run_id_factory=lambda:str(uuid.uuid4()))->str:
 if not isinstance(resources,Stage1Resources) or not isinstance(authorization,owner_auth.VerifiedOwnerAuthorization): raise PermissionError('verified stage-1 launch inputs are required')
 _recheck_authorized_directories(authorization,authorization_ledger_fd,workspace_parent_fd,receipt_parent_fd)
 candidate,review_aggregate=_replay_owner_resources(resources,authorization)
 _recheck_authorized_directories(authorization,authorization_ledger_fd,workspace_parent_fd,receipt_parent_fd)
 receipt_registration=_receipt_registration(receipt_parent_fd,authorization.receipt_name)
 signed_receipt=authorization.payload['receipt']
 if (receipt_registration['parent_device'],receipt_registration['parent_inode'],receipt_registration['parent_uid'],receipt_registration['name'])!=(signed_receipt['device'],signed_receipt['inode'],signed_receipt['uid'],signed_receipt['name']): raise PermissionError('receipt parent identity mismatch')
 with authorization_ledger_lock(authorization_ledger_fd):
  return _run_stage1_locked(candidate,review_aggregate,authorization,authorization_ledger_fd,workspace_parent_fd,receipt_parent_fd,connection_factory=connection_factory,run_id_factory=run_id_factory)


def _run_stage1_locked(candidate:Mapping[str,Any],review_aggregate:str,verified_authorization:owner_auth.VerifiedOwnerAuthorization,authorization_ledger_fd:int,workspace_parent_fd:int,receipt_parent_fd:int,*,connection_factory=None,run_id_factory=lambda:str(uuid.uuid4()))->str:
 """Execute the exact reviewed stage-1 lifecycle; never transfers a model."""
 recover_stage1_workspaces(authorization_ledger_fd,workspace_parent_fd)
 reconciled=_reconcile_stage1_receipt(authorization_ledger_fd,receipt_parent_fd,candidate,review_aggregate,verified_authorization)
 if reconciled is not None: return reconciled
 workspace=create_stage1_workspace(workspace_parent_fd,verified_authorization.generation,candidate); registered=False
 try:
  authorization=_register_authorization(authorization_ledger_fd,verified_authorization,workspace,candidate,review_aggregate); registered=True
  transport=OCITransport(connection_factory,authorization); artifact_names=[]
  required=candidate['provenance']['minimum_free_bytes_before_each_transfer']
  for index,item in enumerate([candidate['runtime'],*candidate['dependencies']]):
   repository,digest,size=_artifact_values(item)
   metadata_digest=item.get('index_digest',item.get('index')); challenge_kind='manifests' if metadata_digest is not None else 'blobs'; challenge_digest=metadata_digest or digest
   challenge=transport.fetch_bearer_challenge(repository,challenge_kind,challenge_digest)
   token=transport.fetch_token(challenge,repository)
   _replay_oci_metadata(transport,item,token)
   name=f'layer-{index}.tar'; transport.download_blob_at(repository,digest,token,workspace.fd('artifacts'),name,size,candidate['authorizations']['bottle_transfer']['maximum_bytes'],preflight_descriptors=workspace.role_fds,required_bytes=required); artifact_names.append((name,size))
  completed=authorization.complete(); streams=[]
  try:
   flags=os.O_RDONLY
   if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
   for name,size in artifact_names:
    descriptor=os.open(name,flags,dir_fd=workspace.fd('artifacts')); info=os.fstat(descriptor)
    if not stat.S_ISREG(info.st_mode) or info.st_size!=size:
     os.close(descriptor); raise ValueError('downloaded layer identity changed')
    streams.append(os.fdopen(descriptor,'rb',closefd=True))
   extract_tar_layers(streams,workspace.fd('runtime'),candidate['provenance']['maximum_entries'],candidate['provenance']['maximum_total_extracted_bytes'],layer_limits=_layer_extraction_limits(candidate))
  finally:
   for stream in streams: stream.close()
  inventory=inspect_runtime(workspace.fd('runtime'),_runtime_packages(candidate),candidate['provenance']['maximum_entries'],candidate['provenance']['maximum_total_extracted_bytes'])
 except BaseException:
  cleanup_completed=False
  try: workspace.cleanup(); cleanup_completed=True
  finally:
   if registered and cleanup_completed: _clear_workspace_registration(authorization_ledger_fd,verified_authorization.generation,'failed_cleaned')
  raise
 cleanup=workspace.cleanup()
 _clear_workspace_registration(authorization_ledger_fd,verified_authorization.generation,'authorized')
 lifecycle=_complete_stage1_lifecycle(completed,inventory,cleanup)
 receipt=build_stage1_receipt(candidate,review_aggregate,run_id_factory(),lifecycle)
 receipt_sha256=canonical_json_sha256(receipt)
 destination_registration=_receipt_registration(receipt_parent_fd,verified_authorization.receipt_name)
 _record_cleaned_receipt(authorization_ledger_fd,verified_authorization.generation,receipt,receipt_sha256,destination_registration)
 return _publish_stage1_receipt(receipt_parent_fd,verified_authorization.receipt_name,receipt,candidate,review_aggregate,verified_authorization.envelope_sha256,verified_authorization.generation,expected_receipt_sha256=receipt_sha256,authorization_ledger_fd=authorization_ledger_fd)


def _read_inherited_fd(descriptor:int,maximum_bytes:int,exact_bytes:Optional[int]=None)->bytes:
 chunks=[]; total=0
 while True:
  chunk=os.read(descriptor,min(1024,maximum_bytes-total+1))
  if not chunk: break
  total+=len(chunk)
  if total>maximum_bytes: raise ValueError('inherited authorization input is too large')
  chunks.append(chunk)
 value=b''.join(chunks)
 if exact_bytes is not None and len(value)!=exact_bytes: raise ValueError('inherited authorization input has invalid length')
 return value


def main(argv=None)->int:
 try:
  arguments=list(sys.argv[1:] if argv is None else argv)
  if arguments: raise ValueError('stage-1 accepts no command-line arguments')
  resources=Stage1Resources.from_script(Path(__file__))
  envelope=_read_inherited_fd(AUTHORIZATION_FD,owner_auth.MAX_ENVELOPE_BYTES)
  nonce=_read_inherited_fd(NONCE_FD,32,32)
  directory_stats={'ledger':os.fstat(LEDGER_FD),'workspace_parent':os.fstat(WORKSPACE_PARENT_FD),'receipt_parent':os.fstat(RECEIPT_PARENT_FD)}
  authorization=owner_auth.verify_owner_authorization(envelope,nonce,_read_path(resources.public_key,4096),_read_path(resources.binding,owner_auth.MAX_BINDING_BYTES),directory_stats,int(time.time()),owner_auth.SecurityFrameworkSignatureVerifier())
  digest=run_stage1(resources,authorization,LEDGER_FD,WORKSPACE_PARENT_FD,RECEIPT_PARENT_FD)
  print(digest); return 0
 except BaseException as error:
  print(redacted_stage1_failure(error),file=sys.stderr); return 1


if __name__=='__main__':
 raise SystemExit(main())
