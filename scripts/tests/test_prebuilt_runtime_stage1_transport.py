import hashlib,io,json,os,tempfile,unittest
from pathlib import Path
from unittest import mock
from scripts import prebuilt_runtime_stage1 as stage1
from scripts.prebuilt_runtime_stage1 import CANDIDATE_SHA256,OCITransport,copy_verified,parse_bearer_challenge,select_oci_manifest,validate_blob_redirect,verify_oci_manifest
from scripts.tests.stage1_owner_test_support import verified_authorization


class FakeResponse:
 def __init__(self,status,body=b'',headers=()): self.status=status; self._body=io.BytesIO(body); self._headers=list(headers); self.closed=False
 def read(self,amount=-1): return self._body.read(amount)
 def getheaders(self): return list(self._headers)
 def close(self): self.closed=True


class FakeConnection:
 def __init__(self,origin,routes,calls): self.origin=origin; self.routes=routes; self.calls=calls; self.response=None
 def request(self,method,target,body=None,headers=None):
  self.calls.append((self.origin,method,target,dict(headers or {})))
  self.response=self.routes[self.origin].pop(0)
 def getresponse(self): return self.response
 def close(self): pass


def transfer_claim(repository,digest,size):
 candidate={
  'candidate_id':'fixture','authorizations':{'bottle_transfer':{'artifact_count':1,'maximum_bytes':size}},
  'runtime':{'formula':'fixture','version':'1.0','oci_repository':repository,'layer_digest':digest,'layer_bytes':size,'installed_bytes':size},'dependencies':[],
  'provenance':{'maximum_entries':10,'maximum_total_extracted_bytes':size,'bottle_transfer_cap_bytes':size,'minimum_free_bytes_before_each_transfer':1,'verified_layer_total_bytes':size,'origin_policy':{
   'registry':'https://ghcr.io:443','token_realm':'https://ghcr.io:443/token','service':'ghcr.io','allowed_blob_redirect_origin':'https://pkg-containers.githubusercontent.com:443','authorization_forwarding':'same-origin-only','strip_authorization_on_cross_origin':True,'redirect_limit':1,'reject_redirect_from_blob_origin':True,'content_length_required':True,
  }},
 }
 aggregate='d'*64
 with tempfile.TemporaryDirectory() as td:
  paths={name:Path(td)/name for name in ('ledger','workspace_parent','receipt_parent')}
  for path in paths.values(): path.mkdir(mode=0o700)
  fds={name:os.open(path,os.O_RDONLY|os.O_DIRECTORY) for name,path in paths.items()}
  try:
   verified=verified_authorization(candidate,fds,candidate_sha256=CANDIDATE_SHA256,harness_aggregate=aggregate)
   return stage1._transfer_authorization_from_verified(verified,candidate,aggregate)
  finally:
   for fd in fds.values(): os.close(fd)


class TransportTests(unittest.TestCase):
 def test_exact_bearer_challenge(self):
  got=parse_bearer_challenge('Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:homebrew/core/llama.cpp:pull"')
  self.assertEqual(got,('https://ghcr.io/token','ghcr.io','repository:homebrew/core/llama.cpp:pull'))
  with self.assertRaises(ValueError): parse_bearer_challenge('Basic realm="x"')
  with self.assertRaises(ValueError): parse_bearer_challenge('Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:homebrew/core/x/../evil:pull"')
  with self.assertRaises(ValueError): parse_bearer_challenge('Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:homebrew/core/x/./evil:pull"')

 def test_transport_acquires_exact_bearer_challenge_without_authorization(self):
  repository='homebrew/core/llama.cpp'; digest='a'*64
  challenge='Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:homebrew/core/llama.cpp:pull"'
  routes={'https://ghcr.io:443':[FakeResponse(401,b'{}',[('Content-Length','2'),('WWW-Authenticate',challenge)])]}; calls=[]
  client=OCITransport(lambda origin:FakeConnection(origin,routes,calls))
  self.assertEqual(client.fetch_bearer_challenge(repository,'manifests',digest),challenge)
  self.assertEqual(calls,[('https://ghcr.io:443','GET',f'/v2/{repository}/manifests/sha256:{digest}',{'Accept':'application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json'})])

 def test_one_cross_origin_redirect_strips_authorization(self):
  source='https://ghcr.io/v2/homebrew/core/llama.cpp/blobs/sha256:'+'a'*64
  self.assertEqual(validate_blob_redirect(source,'https://pkg-containers.githubusercontent.com/blob',0),('https://pkg-containers.githubusercontent.com/blob',False))
  with self.assertRaises(ValueError): validate_blob_redirect('https://pkg-containers.githubusercontent.com/blob','https://evil.test/x',1)
  with self.assertRaises(ValueError): validate_blob_redirect('https://ghcr.io/token','https://pkg-containers.githubusercontent.com/blob',0)

 def test_bounded_copy_requires_length_and_digest(self):
  data=b'abc'; digest=hashlib.sha256(data).hexdigest()
  with tempfile.TemporaryDirectory() as td:
   out=Path(td)/'blob'; copy_verified(io.BytesIO(data),out,3,digest,4)
   self.assertEqual(out.read_bytes(),data)
   with self.assertRaises(ValueError): copy_verified(io.BytesIO(data),Path(td)/'bad',None,digest,4)

 def test_bounded_copy_retains_parent_descriptor_across_symlink_swap(self):
  data=b'owned'; digest=hashlib.sha256(data).hexdigest()
  with tempfile.TemporaryDirectory() as td:
   base=Path(td); parent=base/'parent'; moved=base/'moved'; outside=base/'outside'
   parent.mkdir(); outside.mkdir()
   class SwapReader(io.BytesIO):
    def read(self,amount=None):
     if not moved.exists(): parent.rename(moved); parent.symlink_to(outside,target_is_directory=True)
     return super().read(amount)
   try:
    copy_verified(SwapReader(data),parent/'blob',len(data),digest,len(data))
    self.assertEqual((moved/'blob').read_bytes(),data)
    self.assertEqual(list(outside.iterdir()),[])
   finally:
    if parent.is_symlink(): parent.unlink()

 def test_interrupted_copy_removes_private_partial(self):
  data=b'owned'; digest=hashlib.sha256(data).hexdigest()
  class InterruptReader(io.BytesIO):
   def read(self,amount=None):
    if self.tell(): raise KeyboardInterrupt()
    return super().read(2)
  with tempfile.TemporaryDirectory() as td:
   with self.assertRaises(KeyboardInterrupt): copy_verified(InterruptReader(data),Path(td)/'blob',len(data),digest,len(data))
   self.assertEqual(list(Path(td).iterdir()),[])

 def test_blob_transport_has_no_ungated_path(self):
  digest=hashlib.sha256(b'x').hexdigest(); calls=[]
  client=OCITransport(lambda origin:calls.append(origin))
  with tempfile.TemporaryDirectory() as td:
   with self.assertRaises(PermissionError): client.download_blob('homebrew/core/x',digest,'token',Path(td)/'blob',1,1)
  self.assertEqual(calls,[])

 def test_injected_oci_token_metadata_and_blob_transport(self):
  metadata=json.dumps({'schemaVersion':2},separators=(',',':')).encode()
  metadata_digest=hashlib.sha256(metadata).hexdigest()
  blob=b'exact bottle layer'; blob_digest=hashlib.sha256(blob).hexdigest()
  routes={
   'https://ghcr.io:443':[
    FakeResponse(200,b'{"token":"memory-only"}',[('Content-Length','23')]),
    FakeResponse(200,metadata,[('Content-Length',str(len(metadata))),('Content-Type','application/vnd.oci.image.manifest.v1+json'),('Docker-Content-Digest','sha256:'+metadata_digest)]),
    FakeResponse(307,headers=[('Location','https://pkg-containers.githubusercontent.com:443/layer')]),
   ],
   'https://pkg-containers.githubusercontent.com:443':[
    FakeResponse(200,blob,[('Content-Length',str(len(blob)))]),
   ],
  }
  calls=[]
  client=OCITransport(lambda origin:FakeConnection(origin,routes,calls),transfer_claim('homebrew/core/llama.cpp',blob_digest,len(blob)))
  challenge='Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:homebrew/core/llama.cpp:pull"'
  token=client.fetch_token(challenge,'homebrew/core/llama.cpp')
  self.assertEqual(client.fetch_json('homebrew/core/llama.cpp','manifests',metadata_digest,token,1024),{'schemaVersion':2})
  with tempfile.TemporaryDirectory() as td:
   destination=Path(td)/'layer'
   client.download_blob('homebrew/core/llama.cpp',blob_digest,token,destination,len(blob),1024)
   self.assertEqual(destination.read_bytes(),blob)
   with self.assertRaises(PermissionError): client.download_blob('homebrew/core/llama.cpp',blob_digest,token,Path(td)/'replay',len(blob),1024)
  self.assertEqual(calls[0][2],'/token?service=ghcr.io&scope=repository%3Ahomebrew%2Fcore%2Fllama.cpp%3Apull')
  self.assertNotIn('Authorization',calls[0][3])
  self.assertEqual(calls[1][3]['Authorization'],'Bearer memory-only')
  self.assertEqual(calls[2][3]['Authorization'],'Bearer memory-only')
  self.assertNotIn('Authorization',calls[3][3])

 def test_blob_can_only_be_published_descriptor_relative_for_stage_runner(self):
  blob=b'exact bottle layer'; digest=hashlib.sha256(blob).hexdigest(); redirect='https://pkg-containers.githubusercontent.com:443/layer'
  routes={'https://ghcr.io:443':[FakeResponse(307,headers=[('Location',redirect)])],'https://pkg-containers.githubusercontent.com:443':[FakeResponse(200,blob,[('Content-Length',str(len(blob)))])]}
  client=OCITransport(lambda origin:FakeConnection(origin,routes,[]),transfer_claim('homebrew/core/llama.cpp',digest,len(blob)))
  with tempfile.TemporaryDirectory() as td:
   root=Path(td); descriptors={}
   try:
    for role in ('artifacts','runtime','evidence'):
     path=root/role; path.mkdir(); descriptors[role]=os.open(path,os.O_RDONLY|os.O_DIRECTORY)
    client.download_blob_at('homebrew/core/llama.cpp',digest,'token',descriptors['artifacts'],'layer.tar',len(blob),1024,preflight_descriptors=descriptors,required_bytes=1)
   finally:
    for descriptor in descriptors.values(): os.close(descriptor)
   self.assertEqual((root/'artifacts/layer.tar').read_bytes(),blob)

 def test_live_preflight_failure_after_final_headers_reads_no_body_byte(self):
  blob=b'exact bottle layer'; digest=hashlib.sha256(blob).hexdigest(); redirect='https://pkg-containers.githubusercontent.com:443/layer'; events=[]
  class FinalResponse(FakeResponse):
   def getheaders(self): events.append('headers'); return super().getheaders()
   def read(self,amount=-1): events.append('read'); return super().read(amount)
  routes={'https://ghcr.io:443':[FakeResponse(307,headers=[('Location',redirect)])],'https://pkg-containers.githubusercontent.com:443':[FinalResponse(200,blob,[('Content-Length',str(len(blob)))])]}
  client=OCITransport(lambda origin:FakeConnection(origin,routes,[]),transfer_claim('homebrew/core/llama.cpp',digest,len(blob)))
  with tempfile.TemporaryDirectory() as td:
   root=Path(td); descriptors={}
   try:
    for role in ('artifacts','runtime','evidence'):
     path=root/role; path.mkdir(); descriptors[role]=os.open(path,os.O_RDONLY|os.O_DIRECTORY)
    def capacity_lost(observed,required):
     self.assertEqual(observed,descriptors); self.assertEqual(required,2); events.append('preflight'); raise OSError('capacity invalidated')
    with mock.patch.object(stage1,'live_disk_preflight',side_effect=capacity_lost),self.assertRaisesRegex(OSError,'capacity invalidated'):
     client.download_blob_at('homebrew/core/llama.cpp',digest,'token',descriptors['artifacts'],'layer.tar',len(blob),1024,preflight_descriptors=descriptors,required_bytes=2)
    self.assertEqual(events,['headers','preflight'])
    self.assertEqual(os.listdir(descriptors['artifacts']),[])
   finally:
    for descriptor in descriptors.values(): os.close(descriptor)

 def test_oci_metadata_rejects_unreviewed_media_type(self):
  body=b'{"schemaVersion":2}'; digest=hashlib.sha256(body).hexdigest()
  routes={'https://ghcr.io:443':[FakeResponse(200,body,[
   ('Content-Length',str(len(body))),
   ('Content-Type','text/html'),
   ('Docker-Content-Digest','sha256:'+digest),
  ])]}
  client=OCITransport(lambda origin:FakeConnection(origin,routes,[]))
  with self.assertRaises(ValueError):
   client.fetch_json('homebrew/core/llama.cpp','manifests',digest,'token',1024)

 def test_token_exchange_rejects_header_control_characters(self):
  body=b'{"token":"bad\\nvalue"}'
  routes={'https://ghcr.io:443':[FakeResponse(200,body,[('Content-Length',str(len(body)))])]}
  client=OCITransport(lambda origin:FakeConnection(origin,routes,[]))
  challenge='Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:homebrew/core/llama.cpp:pull"'
  with self.assertRaises(ValueError): client.fetch_token(challenge,'homebrew/core/llama.cpp')

 def test_blob_rejects_duplicate_length_second_redirect_and_bad_digest(self):
  data=b'layer'; digest=hashlib.sha256(data).hexdigest(); redirect='https://pkg-containers.githubusercontent.com:443/blob'
  cases=[
   FakeResponse(200,data,[('Content-Length','5'),('Content-Length','5')]),
   FakeResponse(307,headers=[('Location',redirect)]),
   FakeResponse(200,b'other',[('Content-Length','5')]),
  ]
  for response in cases:
   with self.subTest(status=response.status),tempfile.TemporaryDirectory() as td:
    routes={'https://ghcr.io:443':[FakeResponse(307,headers=[('Location',redirect)])],'https://pkg-containers.githubusercontent.com:443':[response]}
    destination=Path(td)/'blob'; client=OCITransport(lambda origin:FakeConnection(origin,routes,[]),transfer_claim('homebrew/core/llama.cpp',digest,5))
    with self.assertRaises(ValueError): client.download_blob('homebrew/core/llama.cpp',digest,'token',destination,5,5)
    self.assertFalse(destination.exists()); self.assertEqual(list(Path(td).iterdir()),[])

 def test_oci_descriptor_replay_selects_one_exact_arm64_layer(self):
  manifest_digest='a'*64; config_digest='b'*64; layer_digest='c'*64
  descriptor={'mediaType':'application/vnd.oci.image.manifest.v1+json','digest':'sha256:'+manifest_digest,'size':321,'annotations':{'org.opencontainers.image.ref.name':'0.4.1.arm64_golden_gate'},'platform':{'architecture':'arm64','os':'darwin'}}
  index={'schemaVersion':2,'mediaType':'application/vnd.oci.image.index.v1+json','manifests':[descriptor]}
  self.assertEqual(select_oci_manifest(index,'0.4.1.arm64_golden_gate',manifest_digest),descriptor)
  manifest={'schemaVersion':2,'mediaType':'application/vnd.oci.image.manifest.v1+json','config':{'mediaType':'application/vnd.oci.image.config.v1+json','digest':'sha256:'+config_digest,'size':12},'layers':[{'mediaType':'application/vnd.oci.image.layer.v1.tar+gzip','digest':'sha256:'+layer_digest,'size':99}]}
  self.assertEqual(verify_oci_manifest(manifest,config_digest,layer_digest,99)['size'],99)
  duplicate=dict(index); duplicate['manifests']=[descriptor,dict(descriptor)]
  with self.assertRaises(ValueError): select_oci_manifest(duplicate,'0.4.1.arm64_golden_gate',manifest_digest)
  wrong=dict(manifest); wrong['layers']=[dict(manifest['layers'][0],size=100)]
  with self.assertRaises(ValueError): verify_oci_manifest(wrong,config_digest,layer_digest,99)

if __name__=='__main__': unittest.main()
