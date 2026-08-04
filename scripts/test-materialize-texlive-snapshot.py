#!/usr/bin/env python3
import hashlib, http.server, json, subprocess, tempfile, threading
from pathlib import Path
root = Path(__file__).resolve().parents[1]
script = root / "scripts/provision.py"
library = root / "scripts/texlive.py"
sha = lambda data: hashlib.sha256(data).hexdigest()
with tempfile.TemporaryDirectory() as temporary:
    work = Path(temporary); hosted = work / "hosted"; objects = hosted / "objects"; objects.mkdir(parents=True)
    payload = b"fixture payload\n"; payload_digest = sha(payload); payload_name = f"sha256-{payload_digest}"; (objects / payload_name).write_bytes(payload)
    key = "tex:fixture.tex"
    shard = json.dumps({"schema":1,"distribution":"fixture","index":0,"files":{key:{"virtualPath":"tex/fixture.tex","object":payload_name,"sha256":payload_digest,"bytes":len(payload),"dependencies":[]}}},separators=(",", ":"),sort_keys=True).encode()+b"\n"
    shard_digest=sha(shard); (objects/f"sha256-{shard_digest}").write_bytes(shard)
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self,*_): pass
    server=http.server.ThreadingHTTPServer(("127.0.0.1",0),lambda *args:Quiet(*args,directory=hosted)); threading.Thread(target=server.serve_forever,daemon=True).start()
    base=f"http://127.0.0.1:{server.server_port}/"
    manifest=json.dumps({"schema":3,"distribution":"fixture","objectsBaseUrl":base+"objects/","shardBits":0,"shardCount":1,"shards":[shard_digest],"formats":{"latex":{"object":payload_name,"sha256":payload_digest,"bytes":len(payload),"inputClosure":{"schema":1,"keys":[key]}}}},separators=(",", ":"),sort_keys=True).encode()+b"\n"; (hosted/"manifest-v3.json").write_bytes(manifest)
    fixture=work/"provision.py"; fixture.write_text(script.read_text())
    (work/"texlive.py").write_text(library.read_text())
    destination=work/"mirror"; command=["python3",str(fixture),"materialize","--root-url",base+"manifest-v3.json","--root-sha256",sha(manifest),"--output-dir",str(destination),"--format","latex"]
    subprocess.run(command,check=True,capture_output=True,text=True); subprocess.run(command+["--offline"],check=True,capture_output=True,text=True)
    assert (destination/"manifest-v3.json").read_bytes()==manifest and (destination/"objects"/payload_name).read_bytes()==payload
    assert (destination/"texmf-dist"/"tex"/"fixture.tex").read_bytes()==payload
    key_file=work/"keys.txt"; key_file.write_text(key+"\n")
    keyed_destination=work/"keyed-mirror"
    keyed_command=["python3",str(fixture),"materialize","--root-url",base+"manifest-v3.json","--root-sha256",sha(manifest),"--output-dir",str(keyed_destination),"--keys-from",str(key_file)]
    subprocess.run(keyed_command,check=True,capture_output=True,text=True)
    assert (keyed_destination/"texmf-dist"/"tex"/"fixture.tex").read_bytes()==payload
    (destination/"objects"/payload_name).write_bytes(b"corrupt"); failed=subprocess.run(command+["--offline"],capture_output=True,text=True)
    assert failed.returncode != 0 and "cached snapshot object" in failed.stderr; server.shutdown()
print("provision.py materialize contract: PASS")
